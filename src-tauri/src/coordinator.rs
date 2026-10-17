use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Local { path: String },
    Online { url: String, provider: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub title: String,
    pub source: Source,
    pub position: usize,
    pub duration: Option<f64>,
    pub trim_start: f64,
    pub trim_end: Option<f64>,
    pub selected: bool,
    pub state: String,
    pub progress: Option<f64>,
    pub error: Option<String>,
    pub output: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnlineEntry {
    pub title: String,
    pub url: String,
    pub provider: String,
    pub duration: Option<f64>,
    pub position: usize,
    pub available: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub jobs: Vec<Job>,
    pub output_dir: Option<String>,
    pub running: bool,
    pub online_available: bool,
    pub component_status: String,
    pub notice: Option<String>,
}
pub trait Pipeline: Send + Sync + 'static {
    fn probe(&self, input: &Path, cancel: &Arc<AtomicBool>) -> Result<f64, String>;
    fn acquire(
        &self,
        source: &Source,
        dir: &Path,
        cancel: &Arc<AtomicBool>,
        progress: &dyn Fn(f64),
    ) -> Result<PathBuf, String>;
    fn convert(
        &self,
        input: &Path,
        dir: &Path,
        job: &Job,
        cancel: &Arc<AtomicBool>,
        progress: &dyn Fn(f64),
    ) -> Result<PathBuf, String>;
    fn preview(
        &self,
        input: &Path,
        dir: &Path,
        cancel: &Arc<AtomicBool>,
    ) -> Result<PathBuf, String>;
}
struct Inner {
    snapshot: Snapshot,
    tokens: HashMap<String, Arc<AtomicBool>>,
    acquired: HashMap<String, PathBuf>,
    acquired_bytes: u64,
    admitted: HashSet<String>,
}
#[derive(Clone)]
pub struct Coordinator {
    inner: Arc<Mutex<Inner>>,
    pipeline: Arc<dyn Pipeline>,
    work: PathBuf,
    serial: Arc<AtomicU64>,
}
impl Coordinator {
    pub fn new(pipeline: Arc<dyn Pipeline>, work: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                snapshot: Snapshot {
                    jobs: vec![],
                    output_dir: None,
                    running: false,
                    online_available: false,
                    component_status: "Online components not installed".into(),
                    notice: None,
                },
                tokens: HashMap::new(),
                acquired: HashMap::new(),
                acquired_bytes: 0,
                admitted: HashSet::new(),
            })),
            pipeline,
            work,
            serial: Arc::new(AtomicU64::new(1)),
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        self.inner.lock().unwrap().snapshot.clone()
    }
    pub fn component_status(&self, available: bool, status: String) {
        let mut i = self.inner.lock().unwrap();
        i.snapshot.online_available = available;
        i.snapshot.component_status = status;
    }
    pub fn notice(&self, message: String) {
        self.inner.lock().unwrap().snapshot.notice = Some(message);
    }
    pub fn set_output_dir(&self, path: PathBuf) -> Result<(), String> {
        if !path.is_dir() {
            return Err("Choose an existing output folder".into());
        }
        self.inner.lock().unwrap().snapshot.output_dir = Some(path.to_string_lossy().into_owned());
        Ok(())
    }
    fn push(
        &self,
        title: String,
        source: Source,
        duration: Option<f64>,
        error: Option<String>,
    ) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        if i.snapshot.jobs.len() >= 2000 {
            return Err(
                "The queue holds at most 2000 items. Remove some rows before importing more."
                    .into(),
            );
        }
        let id = self.serial.fetch_add(1, Ordering::Relaxed).to_string();
        let position = id.parse::<usize>().unwrap_or(0);
        i.tokens
            .insert(id.clone(), Arc::new(AtomicBool::new(false)));
        i.snapshot.jobs.push(Job {
            id,
            title: title.chars().take(1024).collect(),
            source,
            position,
            duration,
            trim_start: 0.,
            trim_end: None,
            selected: error.is_none(),
            state: if error.is_some() { "failed" } else { "queued" }.into(),
            progress: None,
            error,
            output: None,
        });
        Ok(())
    }
    pub fn add_local(&self, paths: Vec<PathBuf>) -> Result<(), String> {
        self.add_local_with_cancel(paths, &Arc::new(AtomicBool::new(false)))
    }
    pub fn add_local_with_cancel(
        &self,
        paths: Vec<PathBuf>,
        c: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        if paths.len() > 1000 {
            return Err("Add at most 1000 files at once".into());
        }
        for p in paths {
            if c.load(Ordering::SeqCst) {
                return Err("File import cancelled".into());
            }
            let result = self.pipeline.probe(&p, c);
            if c.load(Ordering::SeqCst) {
                return Err("File import cancelled".into());
            }
            let title = p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let (duration, error) = match result {
                Ok(d) => (Some(d), None),
                Err(e) => (None, Some(e)),
            };
            self.push(
                title,
                Source::Local {
                    path: p.to_string_lossy().into_owned(),
                },
                duration,
                error,
            )?;
        }
        Ok(())
    }
    pub fn add_online(&self, entries: Vec<OnlineEntry>) -> Result<(), String> {
        if entries.len() > 1000 {
            return Err("Import limited to 1000 entries".into());
        }
        for e in entries {
            self.push(
                e.title,
                Source::Online {
                    url: e.url,
                    provider: e.provider,
                },
                e.duration,
                if e.available {
                    None
                } else {
                    Some("This item is unavailable".into())
                },
            )?;
        }
        Ok(())
    }
    pub fn update(
        &self,
        id: &str,
        selected: Option<bool>,
        start: Option<f64>,
        end: Option<Option<f64>>,
    ) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let j = i
            .snapshot
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("Item no longer exists")?;
        if j.state == "complete" && start.is_none() && end.is_none() {
            if let Some(v) = selected {
                j.selected = v;
            }
            return Ok(());
        }
        if !["queued", "failed", "cancelled"].contains(&j.state.as_str()) {
            return Err("Wait or cancel before editing this item".into());
        }
        let s = start.unwrap_or(j.trim_start);
        let e = end.unwrap_or(j.trim_end);
        if !s.is_finite()
            || s < 0.
            || e.is_some_and(|e| !e.is_finite() || e <= s)
            || j.duration
                .is_some_and(|d| s >= d || e.is_some_and(|e| e > d + 0.001))
        {
            return Err("Trim must be within the track, with end after start".into());
        }
        j.trim_start = s;
        j.trim_end = e;
        if let Some(v) = selected {
            j.selected = v;
        }
        Ok(())
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let token = i.tokens.get(id).ok_or("Item no longer exists")?.clone();
        token.store(true, Ordering::SeqCst);
        if let Some(j) = i.snapshot.jobs.iter_mut().find(|j| j.id == id) {
            if !["complete", "failed", "cancelled"].contains(&j.state.as_str()) {
                j.state =
                    if ["converting", "acquiring", "validating"].contains(&j.state.as_str()) {
                        "cancelling"
                    } else {
                        "cancelled"
                    }
                    .into();
            }
        }
        if let Some(path) = i.acquired.remove(id) {
            i.acquired_bytes = i
                .acquired_bytes
                .saturating_sub(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0));
            drop(i);
            let _ = std::fs::remove_dir_all(self.work.join(format!("job-{}", id)));
        }
        Ok(())
    }
    pub fn cancel_all(&self) {
        for j in self.snapshot().jobs {
            let _ = self.cancel(&j.id);
        }
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.cancel(id)?;
        let mut i = self.inner.lock().unwrap();
        i.snapshot.jobs.retain(|j| j.id != id);
        drop(i);
        let _ = std::fs::remove_dir_all(self.work.join(format!("preview-{id}")));
        Ok(())
    }
    pub fn retry(&self, id: &str) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let j = i
            .snapshot
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("Item no longer exists")?;
        if !["failed", "cancelled"].contains(&j.state.as_str()) {
            return Err("Only failed or cancelled items can retry".into());
        }
        j.state = "queued".into();
        j.error = None;
        j.progress = None;
        j.selected = true;
        i.tokens.insert(id.into(), Arc::new(AtomicBool::new(false)));
        i.admitted.remove(id);
        Ok(())
    }
    fn change(
        &self,
        id: &str,
        state: &str,
        progress: Option<f64>,
        error: Option<String>,
        output: Option<String>,
    ) {
        let mut i = self.inner.lock().unwrap();
        if let Some(j) = i.snapshot.jobs.iter_mut().find(|j| j.id == id) {
            if j.state == "cancelling" && ["acquiring", "converting"].contains(&state) {
                return;
            }
            j.state = state.into();
            j.progress = progress.filter(|p| p.is_finite()).map(|p| p.clamp(0., 1.));
            j.error = error;
            if output.is_some() {
                j.output = output;
            }
        }
    }
    pub fn start(&self) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        if i.snapshot.running {
            return Err("The queue is already running".into());
        }
        let out = PathBuf::from(
            i.snapshot
                .output_dir
                .as_ref()
                .ok_or("Choose an output folder first")?,
        );
        if !out.is_dir() {
            return Err("Output folder is unavailable".into());
        }
        if i.snapshot
            .jobs
            .iter()
            .all(|j| !j.selected || j.state != "queued")
        {
            return Err("Select at least one queued item".into());
        }
        if !i.snapshot.online_available
            && i.snapshot.jobs.iter().any(|j| {
                j.selected && j.state == "queued" && matches!(j.source, Source::Online { .. })
            })
        {
            return Err(
                "Install online components, or deselect online items to convert local files".into(),
            );
        }
        std::fs::create_dir_all(&self.work).map_err(|e| e.to_string())?;
        i.admitted = i
            .snapshot
            .jobs
            .iter()
            .filter(|j| j.selected && j.state == "queued")
            .map(|j| j.id.clone())
            .collect();
        let cached: HashSet<String> = i
            .acquired
            .iter()
            .filter(|(_, p)| p.is_file())
            .map(|(id, _)| id.clone())
            .collect();
        for j in &mut i.snapshot.jobs {
            if j.selected
                && j.state == "queued"
                && (matches!(j.source, Source::Local { .. }) || cached.contains(&j.id))
            {
                j.state = "ready".into();
            }
        }
        i.snapshot.running = true;
        drop(i);
        let app = self.clone();
        thread::spawn(move || {
            let mut handles = vec![];
            for _ in 0..2 {
                let a = app.clone();
                handles.push(thread::spawn(move || a.acquire_worker()));
            }
            let a = app.clone();
            handles.push(thread::spawn(move || a.convert_worker(out)));
            for h in handles {
                let _ = h.join();
            }
            let mut i = app.inner.lock().unwrap();
            i.snapshot.running = false;
            i.admitted.clear();
        });
        Ok(())
    }
    fn acquire_worker(&self) {
        loop {
            let next = {
                let mut i = self.inner.lock().unwrap();
                let pending = i.snapshot.jobs.iter().any(|j| {
                    i.admitted.contains(&j.id)
                        && j.selected
                        && ["queued", "acquiring"].contains(&j.state.as_str())
                        && matches!(j.source, Source::Online { .. })
                });
                if !pending {
                    return;
                }
                if i.acquired.len()
                    + i.snapshot
                        .jobs
                        .iter()
                        .filter(|j| j.state == "acquiring")
                        .count()
                    >= 3
                    || i.acquired_bytes >= 1024 * 1024 * 1024
                {
                    None
                } else {
                    let candidate = i.snapshot.jobs.iter().position(|j| {
                        i.admitted.contains(&j.id)
                            && j.selected
                            && j.state == "queued"
                            && matches!(j.source, Source::Online { .. })
                    });
                    candidate.map(|n| {
                        i.snapshot.jobs[n].state = "acquiring".into();
                        let j = i.snapshot.jobs[n].clone();
                        let c = i.tokens[&j.id].clone();
                        (j, c)
                    })
                }
            };
            let Some((j, c)) = next else {
                thread::sleep(Duration::from_millis(30));
                continue;
            };
            let dir = self.work.join(format!("job-{}", j.id));
            let result = std::fs::create_dir_all(&dir)
                .map_err(|e| e.to_string())
                .and_then(|_| {
                    if fs2::available_space(&dir).unwrap_or(0) < 256 * 1024 * 1024 {
                        return Err(
                            "Not enough free scratch space; free at least 256 MiB and retry".into(),
                        );
                    }
                    self.pipeline.acquire(&j.source, &dir, &c, &|p| {
                        self.change(&j.id, "acquiring", Some(p), None, None)
                    })
                });
            let mut dir_handled = false;
            match result {
                Ok(path) if !c.load(Ordering::SeqCst) => {
                    let mut i = self.inner.lock().unwrap();
                    if let Some(row) = i.snapshot.jobs.iter_mut().find(|r| r.id == j.id) {
                        if c.load(Ordering::SeqCst) {
                            let _ = std::fs::remove_dir_all(&dir);
                            dir_handled = true;
                            row.state = "cancelled".into();
                        } else {
                            row.state = "ready".into();
                            row.progress = None;
                            i.acquired_bytes +=
                                std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            i.acquired.insert(j.id.clone(), path);
                            dir_handled = true;
                        }
                    }
                }
                Ok(_) | Err(_) if c.load(Ordering::SeqCst) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    dir_handled = true;
                    self.change(&j.id, "cancelled", None, None, None)
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    dir_handled = true;
                    self.change(&j.id, "failed", None, Some(e), None)
                }
                _ => {}
            }
            if !dir_handled {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }
    fn convert_worker(&self, out: PathBuf) {
        loop {
            let next = {
                let mut i = self.inner.lock().unwrap();
                let candidate =
                    i.snapshot.jobs.iter().position(|j| {
                        i.admitted.contains(&j.id) && j.selected && j.state == "ready"
                    });
                if let Some(n) = candidate {
                    i.snapshot.jobs[n].state = "converting".into();
                    let j = i.snapshot.jobs[n].clone();
                    let c = i.tokens[&j.id].clone();
                    let path = match &j.source {
                        Source::Local { path } => PathBuf::from(path),
                        Source::Online { .. } => i.acquired.remove(&j.id).unwrap_or_default(),
                    };
                    i.acquired_bytes = i
                        .acquired_bytes
                        .saturating_sub(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0));
                    Some((j, c, path))
                } else {
                    if !i.snapshot.jobs.iter().any(|j| {
                        i.admitted.contains(&j.id)
                            && j.selected
                            && ["queued", "acquiring", "ready", "cancelling"]
                                .contains(&j.state.as_str())
                    }) {
                        return;
                    }
                    None
                }
            };
            let Some((j, c, path)) = next else {
                thread::sleep(Duration::from_millis(30));
                continue;
            };
            let result = if c.load(Ordering::SeqCst) {
                Err("cancelled".into())
            } else {
                self.pipeline.convert(&path, &out, &j, &c, &|p| {
                    self.change(&j.id, "converting", Some(p), None, None)
                })
            };
            match result {
                Ok(p) => self.change(
                    &j.id,
                    "complete",
                    Some(1.),
                    None,
                    Some(p.to_string_lossy().into_owned()),
                ),
                Err(_) if c.load(Ordering::SeqCst) => {
                    self.change(&j.id, "cancelled", None, None, None)
                }
                Err(e) => self.change(&j.id, "failed", None, Some(e), None),
            }
            if matches!(j.source, Source::Online { .. }) {
                let _ = std::fs::remove_dir_all(self.work.join(format!("job-{}", j.id)));
            }
        }
    }
    pub fn preview(&self, id: &str) -> Result<PathBuf, String> {
        self.preview_with_cancel(id, &Arc::new(AtomicBool::new(false)))
    }
    pub fn preview_with_cancel(&self, id: &str, c: &Arc<AtomicBool>) -> Result<PathBuf, String> {
        let s = self.snapshot();
        let j = s
            .jobs
            .iter()
            .find(|j| j.id == id)
            .ok_or("Item no longer exists")?;
        let dir = self.work.join(format!("preview-{}", j.id));
        let cached = {
            let mut i = self.inner.lock().unwrap();
            let old: Vec<_> = i
                .acquired
                .iter()
                .filter(|(key, path)| {
                    key.as_str() != id && path.starts_with(self.work.join(format!("preview-{key}")))
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in old {
                if let Some(path) = i.acquired.remove(&key) {
                    i.acquired_bytes = i
                        .acquired_bytes
                        .saturating_sub(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
                    let _ = std::fs::remove_dir_all(self.work.join(format!("preview-{key}")));
                }
            }
            i.acquired.get(id).filter(|p| p.is_file()).cloned()
        };
        if cached.is_none() && dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let input = match &j.source {
            Source::Local { path } => PathBuf::from(path),
            s @ Source::Online { .. } => {
                if let Some(path) = cached {
                    path
                } else {
                    let path = self.pipeline.acquire(s, &dir, c, &|_| {})?;
                    let mut i = self.inner.lock().unwrap();
                    i.acquired_bytes += std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    i.acquired.insert(id.to_string(), path.clone());
                    path
                }
            }
        };
        self.pipeline.preview(&input, &dir, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoEngine;
    impl Pipeline for NoEngine {
        fn probe(&self, _: &Path, _: &Arc<AtomicBool>) -> Result<f64, String> {
            Ok(1.)
        }
        fn acquire(
            &self,
            _: &Source,
            _: &Path,
            _: &Arc<AtomicBool>,
            _: &dyn Fn(f64),
        ) -> Result<PathBuf, String> {
            unreachable!()
        }
        fn convert(
            &self,
            _: &Path,
            _: &Path,
            _: &Job,
            _: &Arc<AtomicBool>,
            _: &dyn Fn(f64),
        ) -> Result<PathBuf, String> {
            unreachable!()
        }
        fn preview(&self, _: &Path, _: &Path, _: &Arc<AtomicBool>) -> Result<PathBuf, String> {
            unreachable!()
        }
    }
    #[test]
    fn cancelling_ready_download_releases_backpressure_slot() {
        let a = Coordinator::new(Arc::new(NoEngine), PathBuf::from("unused"));
        a.push(
            "track".into(),
            Source::Online {
                url: "https://soundcloud.com/a/b".into(),
                provider: "soundcloud".into(),
            },
            Some(1.),
            None,
        )
        .unwrap();
        let id = a.snapshot().jobs[0].id.clone();
        {
            let mut i = a.inner.lock().unwrap();
            i.snapshot.jobs[0].state = "ready".into();
            i.acquired.insert(id.clone(), PathBuf::from("nonexistent"));
        }
        a.cancel(&id).unwrap();
        assert!(a.inner.lock().unwrap().acquired.is_empty());
    }
}
