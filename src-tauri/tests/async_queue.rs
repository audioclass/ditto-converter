use ditto_app::coordinator::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

// Handshakes, not sleeps, place cancellation/removal between a callback and
// completion. Timeouts only bound broken-test failures.
enum Step {
    Progress(f64, mpsc::Sender<()>),
    Finish(bool),
}
struct Call {
    stage: &'static str,
    steps: mpsc::Sender<Step>,
}
impl Call {
    fn progress(&self, value: f64) {
        let (tx, rx) = mpsc::channel();
        self.steps.send(Step::Progress(value, tx)).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    fn finish(self, success: bool) {
        self.steps.send(Step::Finish(success)).unwrap();
    }
}
struct Controlled(Mutex<mpsc::Sender<Call>>);
impl Controlled {
    fn run(
        &self,
        stage: &'static str,
        output: PathBuf,
        progress: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        let (tx, rx) = mpsc::channel();
        self.0
            .lock()
            .unwrap()
            .send(Call { stage, steps: tx })
            .unwrap();
        loop {
            match rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| e.to_string())?
            {
                Step::Progress(p, ack) => {
                    progress(p);
                    let _ = ack.send(());
                }
                Step::Finish(true) => {
                    std::fs::write(&output, b"committed controlled output").unwrap();
                    return Ok(output);
                }
                Step::Finish(false) => return Err("controlled failure".into()),
            }
        }
    }
}
impl Pipeline for Controlled {
    fn probe(&self, _: &Path, _: &Arc<AtomicBool>) -> Result<f64, String> {
        Ok(2.)
    }
    fn acquire(
        &self,
        _: &Source,
        dir: &Path,
        _: &Arc<AtomicBool>,
        progress: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        self.run("acquiring", dir.join("source.wav"), progress)
    }
    fn convert(
        &self,
        _: &Path,
        dir: &Path,
        job: &Job,
        _: &Arc<AtomicBool>,
        progress: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        self.run("converting", dir.join(format!("{}.wav", job.id)), progress)
    }
    fn preview(&self, _: &Path, _: &Path, _: &Arc<AtomicBool>) -> Result<PathBuf, String> {
        unreachable!()
    }
}
struct Harness {
    queue: Coordinator,
    calls: mpsc::Receiver<Call>,
    dir: PathBuf,
}
impl Harness {
    fn new() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.work/async-queue")
            .join(format!(
                "{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::SeqCst)
            ));
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, calls) = mpsc::channel();
        let queue = Coordinator::new(Arc::new(Controlled(Mutex::new(tx))), dir.join("session"));
        queue.set_output_dir(dir.clone()).unwrap();
        queue.component_status(true, "controlled adapter".into());
        Self { queue, calls, dir }
    }
    fn call(&self, stage: &str) -> Call {
        let call = self.calls.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(call.stage, stage);
        call
    }
    fn wait(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.queue.snapshot().running {
            assert!(Instant::now() < deadline, "queue did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn entry(n: usize) -> OnlineEntry {
    OnlineEntry {
        title: format!("review-{n}"),
        url: format!("https://soundcloud.com/test/track-{n}"),
        provider: "soundcloud".into(),
        duration: Some(2.),
        position: n,
        available: true,
    }
}

#[test]
fn online_rows_imported_during_conversion_need_a_new_explicit_admission() {
    let h = Harness::new();
    h.queue.add_local(vec!["first.wav".into()]).unwrap();
    h.queue.start().unwrap();
    let call = h.call("converting");
    h.queue.add_online(vec![entry(0), entry(1)]).unwrap();
    let removed = h.queue.snapshot().jobs[1].id.clone();
    h.queue.remove(&removed).unwrap();
    call.finish(true);
    h.wait();
    let rows = h.queue.snapshot().jobs;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].title, "review-1");
    assert_eq!(rows[1].state, "queued");
    assert!(
        h.calls.try_recv().is_err(),
        "metadata review must not acquire media"
    );
    h.queue.start().unwrap();
    h.call("acquiring").finish(true);
    h.call("converting").finish(true);
    h.wait();
    assert!(h
        .queue
        .snapshot()
        .jobs
        .iter()
        .all(|j| j.state == "complete"));
}

#[test]
fn removing_active_acquisition_does_not_resurrect_row_or_convert_late_success() {
    let h = Harness::new();
    h.queue.add_online(vec![entry(0)]).unwrap();
    let id = h.queue.snapshot().jobs[0].id.clone();
    h.queue.start().unwrap();
    let call = h.call("acquiring");
    h.queue.remove(&id).unwrap();
    call.progress(0.8);
    call.finish(true);
    h.wait();
    assert!(h.queue.snapshot().jobs.is_empty());
    assert!(!h.dir.join(format!("session/job-{id}")).exists());
    assert!(h.calls.try_recv().is_err());
}

#[test]
fn acquisition_cancel_wins_late_success_and_ignores_late_progress() {
    let h = Harness::new();
    h.queue.add_online(vec![entry(0)]).unwrap();
    let id = h.queue.snapshot().jobs[0].id.clone();
    h.queue.start().unwrap();
    let call = h.call("acquiring");
    call.progress(0.25);
    assert_eq!(h.queue.snapshot().jobs[0].progress, Some(0.25));
    h.queue.cancel(&id).unwrap();
    call.progress(0.9);
    assert_eq!(h.queue.snapshot().jobs[0].state, "cancelling");
    assert_eq!(h.queue.snapshot().jobs[0].progress, Some(0.25));
    call.finish(true);
    h.wait();
    assert_eq!(h.queue.snapshot().jobs[0].state, "cancelled");
    assert!(!h.dir.join(format!("session/job-{id}")).exists());
    assert!(h.calls.try_recv().is_err());
}

#[test]
fn committed_conversion_wins_late_cancel_and_removing_row_preserves_export() {
    let h = Harness::new();
    h.queue.add_local(vec!["local.wav".into()]).unwrap();
    let id = h.queue.snapshot().jobs[0].id.clone();
    h.queue.start().unwrap();
    let call = h.call("converting");
    for (value, expected) in [
        (f64::NAN, None),
        (-1., Some(0.)),
        (2., Some(1.)),
        (0.4, Some(0.4)),
    ] {
        call.progress(value);
        assert_eq!(h.queue.snapshot().jobs[0].progress, expected);
    }
    h.queue.cancel(&id).unwrap();
    call.progress(0.9);
    assert_eq!(h.queue.snapshot().jobs[0].state, "cancelling");
    // A successful converter has already validated and published the export.
    call.finish(true);
    h.wait();
    let row = &h.queue.snapshot().jobs[0];
    assert_eq!(row.state, "complete");
    assert_eq!(row.progress, Some(1.));
    let output = PathBuf::from(row.output.as_ref().unwrap());
    h.queue.remove(&id).unwrap();
    assert_eq!(
        std::fs::read(output).unwrap(),
        b"committed controlled output"
    );
}

#[test]
fn cancelled_conversion_failure_can_retry_only_on_another_explicit_start() {
    let h = Harness::new();
    h.queue.add_local(vec!["local.wav".into()]).unwrap();
    let id = h.queue.snapshot().jobs[0].id.clone();
    h.queue.start().unwrap();
    let call = h.call("converting");
    h.queue.cancel(&id).unwrap();
    assert!(h.queue.retry(&id).is_err());
    call.finish(false);
    h.wait();
    let row = &h.queue.snapshot().jobs[0];
    assert_eq!(row.state, "cancelled");
    assert!(row.error.is_none());
    assert!(row.output.is_none());
    h.queue.retry(&id).unwrap();
    assert_eq!(h.queue.snapshot().jobs[0].state, "queued");
    assert!(h.calls.try_recv().is_err());
    h.queue.start().unwrap();
    h.call("converting").finish(true);
    h.wait();
    assert_eq!(h.queue.snapshot().jobs[0].state, "complete");
}

#[test]
fn removing_converting_row_does_not_resurrect_it_after_a_committed_export() {
    let h = Harness::new();
    h.queue.add_local(vec!["local.wav".into()]).unwrap();
    let id = h.queue.snapshot().jobs[0].id.clone();
    h.queue.start().unwrap();
    let call = h.call("converting");
    h.queue.remove(&id).unwrap();
    call.progress(0.9);
    call.finish(true);
    h.wait();
    assert!(h.queue.snapshot().jobs.is_empty());
    assert!(h.dir.join(format!("{id}.wav")).is_file());
}
