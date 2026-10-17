use crate::coordinator::{Job, Pipeline, Source};
use ditto_core::audio::Engine;
use ditto_online::{components::InstalledPack, OnlineEngine};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
};

pub struct NativePipeline {
    pub audio: Engine,
    online: Mutex<Option<OnlineEngine>>,
    work: PathBuf,
    bin: PathBuf,
    session_tag: u128,
}
impl NativePipeline {
    pub fn new(bin: PathBuf, work: PathBuf) -> Self {
        let suffix = std::env::consts::EXE_SUFFIX;
        Self {
            audio: Engine {
                ffmpeg: bin.join(format!("ffmpeg{suffix}")),
                ffprobe: bin.join(format!("ffprobe{suffix}")),
            },
            online: Mutex::new(None),
            work,
            bin,
            session_tag: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        }
    }
    pub fn activate(&self, pack: InstalledPack) {
        *self.online.lock().unwrap() = Some(OnlineEngine {
            yt_dlp: pack.yt_dlp,
            deno: pack.deno,
            work_dir: self.work.clone(),
            ffmpeg_dir: self.bin.clone(),
        });
    }
    pub fn online(&self) -> Result<OnlineEngine, String> {
        self.online.lock().unwrap().clone().ok_or_else(|| {
            "Set up the optional online tools first. Local conversion is available offline.".into()
        })
    }
}
impl Pipeline for NativePipeline {
    fn probe(&self, input: &Path, c: &Arc<AtomicBool>) -> Result<f64, String> {
        self.audio
            .probe(input, c)
            .map(|i| i.duration)
            .map_err(|e| e.to_string())
    }
    fn acquire(
        &self,
        source: &Source,
        dir: &Path,
        c: &Arc<AtomicBool>,
        progress: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        let Source::Online { url, .. } = source else {
            return Err("Not an online source".into());
        };
        self.online()?
            .acquire(url, &dir.join("media"), c, progress)
            .map_err(|e| e.to_string())
    }
    fn convert(
        &self,
        input: &Path,
        dir: &Path,
        job: &Job,
        c: &Arc<AtomicBool>,
        progress: &dyn Fn(f64),
    ) -> Result<PathBuf, String> {
        let journal_dir = self
            .work
            .parent()
            .ok_or("Invalid app work directory")?
            .join("export-journals");
        std::fs::create_dir_all(&journal_dir).map_err(|e| e.to_string())?;
        if std::fs::symlink_metadata(&journal_dir)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Export journal directory must not be a symlink".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&journal_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let journal = journal_dir.join(format!(
            "{}-{}-{}.json",
            std::process::id(),
            self.session_tag,
            job.id
        ));
        self.audio
            .convert_tracked(
                input,
                dir,
                &job.title,
                job.trim_start,
                job.trim_end,
                c,
                progress,
                &journal,
            )
            .map_err(|e| e.to_string())
    }
    fn preview(&self, input: &Path, dir: &Path, c: &Arc<AtomicBool>) -> Result<PathBuf, String> {
        self.audio.preview(input, dir, c).map_err(|e| e.to_string())
    }
}
