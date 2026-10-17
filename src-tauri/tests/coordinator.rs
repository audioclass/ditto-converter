use ditto_app::coordinator::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
struct Executor;
impl Pipeline for Executor {
    fn probe(&self, _: &Path, _: &Arc<AtomicBool>) -> Result<f64, String> {
        Ok(2.0)
    }
    fn acquire(
        &self,
        _: &Source,
        _: &Path,
        c: &Arc<AtomicBool>,
        p: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        p(0.5);
        thread::sleep(Duration::from_millis(30));
        if c.load(Ordering::Relaxed) {
            Err("cancelled".into())
        } else {
            Ok(PathBuf::from("acquired.wav"))
        }
    }
    fn convert(
        &self,
        _: &Path,
        out: &Path,
        _: &Job,
        c: &Arc<AtomicBool>,
        p: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        p(0.5);
        thread::sleep(Duration::from_millis(50));
        if c.load(Ordering::Relaxed) {
            Err("cancelled".into())
        } else {
            Ok(out.join("track.wav"))
        }
    }
    fn preview(&self, path: &Path, _: &Path, _: &Arc<AtomicBool>) -> Result<PathBuf, String> {
        Ok(path.into())
    }
}
fn app() -> Coordinator {
    Coordinator::new(
        Arc::new(Executor),
        std::env::current_dir()
            .unwrap()
            .join("../.work/scheduler-tests"),
    )
}
fn wait(a: &Coordinator) {
    let t = Instant::now();
    while a.snapshot().running {
        assert!(t.elapsed() < Duration::from_secs(4));
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn explicit_start_is_required_and_selection_is_respected() {
    let a = app();
    a.set_output_dir(std::env::current_dir().unwrap()).unwrap();
    a.add_local(vec![PathBuf::from("a.wav"), PathBuf::from("b.wav")])
        .unwrap();
    let s = a.snapshot();
    assert_eq!(s.jobs.len(), 2);
    assert!(s.jobs.iter().all(|j| j.state == "queued"));
    a.update(&s.jobs[1].id, Some(false), None, None).unwrap();
    a.start().unwrap();
    wait(&a);
    let s = a.snapshot();
    assert_eq!(s.jobs[0].state, "complete");
    assert_eq!(s.jobs[1].state, "queued");
}
#[test]
fn cancelled_item_can_retry_and_completed_removal_preserves_output() {
    let a = app();
    a.set_output_dir(std::env::current_dir().unwrap()).unwrap();
    a.add_local(vec![PathBuf::from("a.wav")]).unwrap();
    let id = a.snapshot().jobs[0].id.clone();
    a.start().unwrap();
    a.cancel(&id).unwrap();
    wait(&a);
    assert_eq!(a.snapshot().jobs[0].state, "cancelled");
    a.retry(&id).unwrap();
    a.start().unwrap();
    wait(&a);
    assert_eq!(a.snapshot().jobs[0].state, "complete");
    a.remove(&id).unwrap();
    assert!(a.snapshot().jobs.is_empty());
}
#[test]
fn invalid_trim_and_missing_output_are_rejected() {
    let a = app();
    a.add_local(vec![PathBuf::from("a.wav")]).unwrap();
    let id = a.snapshot().jobs[0].id.clone();
    assert!(a.update(&id, None, Some(-1.0), None).is_err());
    assert!(a.update(&id, None, Some(3.0), Some(Some(1.0))).is_err());
    assert!(a.start().is_err());
}

#[test]
fn items_added_during_conversion_wait_for_the_next_explicit_start() {
    let a = app();
    a.set_output_dir(std::env::current_dir().unwrap()).unwrap();
    a.add_local(vec![PathBuf::from("first.wav")]).unwrap();
    a.start().unwrap();
    a.add_local(vec![PathBuf::from("review-me.wav")]).unwrap();
    wait(&a);
    assert_eq!(a.snapshot().jobs[1].state, "queued");
    a.start().unwrap();
    wait(&a);
    assert_eq!(a.snapshot().jobs[1].state, "complete");
}

#[test]
fn local_probe_batch_can_cancel_before_any_helper_or_row() {
    let a = app();
    let cancel = Arc::new(AtomicBool::new(true));
    assert!(a
        .add_local_with_cancel(vec![PathBuf::from("a.wav")], &cancel)
        .is_err());
    assert!(a.snapshot().jobs.is_empty());
}

#[test]
fn repeated_imports_obey_the_shared_session_queue_limit() {
    let a = app();
    let entries = || {
        (0..1000)
            .map(|n| OnlineEntry {
                title: n.to_string(),
                url: "https://soundcloud.com/test/track".into(),
                provider: "soundcloud".into(),
                duration: None,
                position: n,
                available: true,
            })
            .collect()
    };
    a.add_online(entries()).unwrap();
    a.add_online(entries()).unwrap();
    assert!(a
        .add_online(vec![OnlineEntry {
            title: "overflow".into(),
            url: "https://soundcloud.com/test/track".into(),
            provider: "soundcloud".into(),
            duration: None,
            position: 0,
            available: true
        }])
        .is_err());
    assert_eq!(a.snapshot().jobs.len(), 2000);
}

#[test]
fn completed_rows_can_be_selected_for_removal_but_not_retrimmed() {
    let a = app();
    a.set_output_dir(std::env::current_dir().unwrap()).unwrap();
    a.add_local(vec![PathBuf::from("done.wav")]).unwrap();
    a.start().unwrap();
    wait(&a);
    let id = a.snapshot().jobs[0].id.clone();
    let output = a.snapshot().jobs[0].output.clone();
    a.update(&id, Some(false), None, None).unwrap();
    assert!(!a.snapshot().jobs[0].selected);
    assert_eq!(a.snapshot().jobs[0].output, output);
    assert!(a.update(&id, None, Some(0.2), None).is_err());
}
