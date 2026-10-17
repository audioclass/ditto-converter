use crate::{
    provider::{check_cancel, private_dir},
    validate_url, Cancellation, OnlineEngine,
};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
const MAX_MEDIA: u64 = 256 * 1024 * 1024;
struct JobGuard {
    path: PathBuf,
    keep: bool,
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
struct DownloadWatch {
    cancel: Cancellation,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    exceeded: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl DownloadWatch {
    fn new(dir: PathBuf, external: Cancellation) -> Self {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let exceeded = Arc::new(AtomicBool::new(false));
        let (c, s, e) = (cancel.clone(), stop.clone(), exceeded.clone());
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                if external.load(Ordering::SeqCst) {
                    c.store(true, Ordering::SeqCst);
                    break;
                }
                let valid = (|| -> std::io::Result<bool> {
                    let mut total = 0u64;
                    for entry in fs::read_dir(&dir)? {
                        let entry = entry?;
                        let meta = match fs::symlink_metadata(entry.path()) {
                            Ok(meta) => meta,
                            // The downloader atomically renames .part files between scans.
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                            Err(error) => return Err(error),
                        };
                        if meta.is_file() {
                            total = total.saturating_add(meta.len());
                        }
                        if entry.file_name().to_string_lossy().starts_with("source.")
                            && (meta.file_type().is_symlink() || meta.len() > MAX_MEDIA)
                        {
                            return Ok(false);
                        }
                    }
                    Ok(total <= MAX_MEDIA * 2)
                })();
                if !matches!(valid, Ok(true)) {
                    e.store(true, Ordering::SeqCst);
                    c.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        Self {
            cancel,
            stop,
            exceeded,
            thread: Some(thread),
        }
    }
}
impl Drop for DownloadWatch {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
impl OnlineEngine {
    /// Explicit user-action preview acquisition. Returns a local source for Engine::preview;
    /// this is a bounded full-source fetch, not automatic streaming or a media-free resolve.
    pub fn acquire_preview(
        &self,
        url: &str,
        job_dir: &Path,
        cancel: &Cancellation,
        on_progress: impl FnMut(f64),
    ) -> Result<PathBuf> {
        self.acquire(url, job_dir, cancel, on_progress)
    }

    /// Explicit admission: downloads one original HTTP(S) stream, not an extracted/transcoded preview.
    /// `job_dir` MUST be a new directory beneath `work_dir`. Parent bounds concurrent acquisitions.
    pub fn acquire(
        &self,
        url: &str,
        job_dir: &Path,
        cancel: &Cancellation,
        mut on_progress: impl FnMut(f64),
    ) -> Result<PathBuf> {
        validate_url(url)?;
        let parsed = url::Url::parse(url)?;
        let parts: Vec<_> = parsed.path().split('/').filter(|p| !p.is_empty()).collect();
        ensure!(
            parts.first() != Some(&"playlist") && parts.get(1) != Some(&"sets"),
            "enumerate and review the playlist before acquiring individual entries"
        );
        check_cancel(cancel)?;
        private_dir(&self.work_dir)?;
        let root = self.work_dir.canonicalize()?;
        let parent = job_dir
            .parent()
            .context("job directory needs parent")?
            .canonicalize()?;
        ensure!(
            parent.starts_with(&root),
            "job directory must be inside online work root"
        );
        fs::create_dir(job_dir)
            .context("job directory must be new; existing files are never reused or removed")?;
        let mut guard = JobGuard {
            path: job_dir.to_path_buf(),
            keep: false,
        };
        private_dir(job_dir)?;
        let dir = job_dir.canonicalize()?;
        ensure!(
            dir.starts_with(&root) && dir != root,
            "invalid job directory"
        );
        let mut cmd = self.command(&dir)?;
        cmd.args([
            "--no-playlist",
            "--no-simulate",
            "--no-overwrites",
            "--no-mtime",
            "--max-filesize",
            "268435456",
            "--format",
            "bestaudio[protocol=https]/bestaudio[protocol=m3u8_native]/bestaudio[protocol=m3u8]/best[protocol=https]",
            "--downloader",
            "m3u8:native",
            "--concurrent-fragments",
            "1",
            "--newline",
            "--progress",
            "--progress-delta",
            "0.25",
            "--progress-template",
            "download:DITTO_PROGRESS %(progress)j",
            "--print",
            "after_move:DITTO_FILE %(filepath)j",
            "--no-quiet",
            "--output",
        ]);
        cmd.arg(dir.join("source.%(ext)s")).arg("--").arg(url);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Async-signal-safe child-only kernel limit, inherited by every helper descendant.
            unsafe {
                cmd.pre_exec(|| {
                    let limit = libc::rlimit {
                        rlim_cur: MAX_MEDIA as libc::rlim_t,
                        rlim_max: MAX_MEDIA as libc::rlim_t,
                    };
                    if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let watch = DownloadWatch::new(dir.clone(), cancel.clone());
        let mut pending = Vec::new();
        let outcome = ditto_core::process::run(
            &mut cmd,
            &watch.cancel,
            ditto_core::process::ProcessOptions {
                timeout: Duration::from_secs(3600),
                max_output: 8 * 1024 * 1024,
            },
            |bytes| {
                pending.extend_from_slice(bytes);
                while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                    let line = String::from_utf8_lossy(&pending[..end]);
                    if let Some(json) = line.trim().strip_prefix("DITTO_PROGRESS ") {
                        if let Ok(v) = serde_json::from_str::<Value>(json) {
                            // Estimated totals remain indeterminate, not fabricated precise percentages.
                            if let (Some(done), Some(total)) = (
                                v.get("downloaded_bytes").and_then(Value::as_f64),
                                v.get("total_bytes").and_then(Value::as_f64),
                            ) {
                                if done.is_finite()
                                    && total.is_finite()
                                    && total > 0.0
                                    && done >= 0.0
                                {
                                    on_progress((done / total).clamp(0.0, 1.0));
                                }
                            }
                        }
                    }
                    pending.drain(..=end);
                }
            },
        );
        ensure!(
            !watch.exceeded.load(std::sync::atomic::Ordering::SeqCst),
            "download disk/size limit exceeded"
        );
        let out = outcome?;
        check_cancel(cancel)?;
        ensure!(out.status.success(),"provider download failed (exit {:?}); permissions, authentication or provider restrictions may prevent access",out.status.code());
        let stdout = std::str::from_utf8(&out.stdout)?;
        let paths: Vec<&str> = stdout
            .lines()
            .filter_map(|s| s.strip_prefix("DITTO_FILE "))
            .collect();
        ensure!(
            paths.len() == 1,
            "missing or ambiguous final download event"
        );
        let path: PathBuf = serde_json::from_str::<String>(paths[0])?.into();
        ensure!(
            path.is_absolute() && path.parent() == Some(dir.as_path()),
            "download escaped private job directory"
        );
        ensure!(
            path.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("source.") && !s.ends_with(".part")),
            "unexpected download filename"
        );
        let meta = fs::symlink_metadata(&path)?;
        ensure!(
            meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.len() > 0
                && meta.len() <= MAX_MEDIA,
            "invalid or oversized downloaded file"
        );
        ensure!(
            path.canonicalize()?.parent() == Some(dir.as_path()),
            "download escaped private job directory"
        );
        guard.keep = true;
        on_progress(1.0);
        Ok(path)
    }
}
