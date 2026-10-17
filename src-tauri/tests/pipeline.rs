use ditto_app::coordinator::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
struct Counted {
    downloads: AtomicUsize,
    active_downloads: AtomicUsize,
    max_downloads: AtomicUsize,
    active_converts: AtomicUsize,
    max_converts: AtomicUsize,
    overlap: AtomicBool,
}
impl Counted {
    fn new() -> Self {
        Self {
            downloads: 0.into(),
            active_downloads: 0.into(),
            max_downloads: 0.into(),
            active_converts: 0.into(),
            max_converts: 0.into(),
            overlap: false.into(),
        }
    }
}
impl Pipeline for Counted {
    fn probe(&self, _: &Path, _: &Arc<AtomicBool>) -> Result<f64, String> {
        Ok(1.)
    }
    fn acquire(
        &self,
        _: &Source,
        d: &Path,
        _: &Arc<AtomicBool>,
        p: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        let n = self.active_downloads.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_downloads.fetch_max(n, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        p(0.5);
        std::fs::create_dir_all(d).unwrap();
        let path = d.join("acquired.wav");
        std::fs::write(&path, b"controlled acquisition fixture").unwrap();
        self.active_downloads.fetch_sub(1, Ordering::SeqCst);
        Ok(path)
    }
    fn convert(
        &self,
        _: &Path,
        d: &Path,
        _: &Job,
        _: &Arc<AtomicBool>,
        _: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        let n = self.active_converts.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_converts.fetch_max(n, Ordering::SeqCst);
        if self.active_downloads.load(Ordering::SeqCst) > 0 {
            self.overlap.store(true, Ordering::SeqCst)
        }
        std::thread::sleep(Duration::from_millis(60));
        self.active_converts.fetch_sub(1, Ordering::SeqCst);
        Ok(d.join("export.wav"))
    }
    fn preview(&self, p: &Path, _: &Path, _: &Arc<AtomicBool>) -> Result<PathBuf, String> {
        Ok(p.into())
    }
}
fn setup(name: &str) -> (Coordinator, Arc<Counted>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".work")
        .join(name);
    std::fs::create_dir_all(&path).unwrap();
    let e = Arc::new(Counted::new());
    let a = Coordinator::new(e.clone(), path.clone());
    a.set_output_dir(path).unwrap();
    a.component_status(true, "controlled test adapter".into());
    (a, e)
}
fn entry(n: usize) -> OnlineEntry {
    OnlineEntry {
        title: format!("item-{n}"),
        url: format!("https://soundcloud.com/test/item-{n}"),
        provider: "soundcloud".into(),
        duration: Some(1.),
        position: n,
        available: true,
    }
}
fn wait(a: &Coordinator) {
    let t = Instant::now();
    while a.snapshot().running {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn explicit_preview_source_is_reused_for_conversion() {
    let (a, e) = setup("preview-reuse");
    a.add_online(vec![entry(0)]).unwrap();
    let id = a.snapshot().jobs[0].id.clone();
    a.preview(&id).unwrap();
    a.start().unwrap();
    wait(&a);
    assert_eq!(
        e.downloads.load(Ordering::SeqCst),
        1,
        "previewed source should not be acquired again"
    );
}
#[test]
fn controlled_pipeline_overlaps_with_two_downloads_and_one_conversion() {
    let (a, e) = setup("bounded-pipeline");
    a.add_online((0..12).map(entry).collect()).unwrap();
    a.start().unwrap();
    wait(&a);
    assert_eq!(e.downloads.load(Ordering::SeqCst), 12);
    assert_eq!(e.max_downloads.load(Ordering::SeqCst), 2);
    assert_eq!(e.max_converts.load(Ordering::SeqCst), 1);
    assert!(e.overlap.load(Ordering::SeqCst));
    assert!(a.snapshot().jobs.iter().all(|j| j.state == "complete"));
}
