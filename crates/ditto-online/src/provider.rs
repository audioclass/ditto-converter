use crate::{validate_url, Result};
use anyhow::{ensure, Context};
use ditto_core::process::{run, ProcessOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

pub type Cancellation = Arc<AtomicBool>;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OnlineEntry {
    pub title: String,
    pub url: String,
    pub provider: String,
    pub duration: Option<f64>,
    pub position: usize,
    pub available: bool,
}
#[derive(Clone, Debug)]
pub struct OnlineEngine {
    pub yt_dlp: PathBuf,
    pub deno: PathBuf,
    pub work_dir: PathBuf,
    pub ffmpeg_dir: PathBuf,
}
pub(crate) fn check_cancel(cancel: &Cancellation) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "cancelled");
    Ok(())
}
pub(crate) fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path)?;
    }
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_dir() && !m.file_type().is_symlink(),
        "private directory required"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
impl OnlineEngine {
    pub(crate) fn command(&self, dir: &Path) -> Result<Command> {
        ensure!(
            self.yt_dlp.is_absolute() && self.deno.is_absolute() && self.ffmpeg_dir.is_absolute(),
            "managed absolute helper paths required"
        );
        private_dir(dir)?;
        let mut cmd = Command::new(&self.yt_dlp);
        cmd.current_dir(dir);
        cmd.env("HOME", dir)
            .env("USERPROFILE", dir)
            .env("XDG_CONFIG_HOME", dir)
            .env("XDG_CACHE_HOME", dir)
            .env("TMPDIR", dir)
            .env("TEMP", dir)
            .env("TMP", dir);
        cmd.env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME")
            .env("DENO_DIR", dir.join("deno-cache"))
            .env("DENO_NO_UPDATE_CHECK", "1");
        cmd.args([
            "--ignore-config",
            "--no-config-locations",
            "--no-plugin-dirs",
            "--no-update",
            "--no-remote-components",
            "--no-js-runtimes",
            "--js-runtimes",
        ]);
        cmd.arg(format!("deno:{}", self.deno.display()))
            .arg("--ffmpeg-location")
            .arg(&self.ffmpeg_dir);
        cmd.args([
            "--no-mark-watched",
            "--color",
            "never",
            "--socket-timeout",
            "15",
            "--retries",
            "2",
            "--fragment-retries",
            "2",
            "--extractor-retries",
            "1",
            "--no-cache-dir",
        ]);
        Ok(cmd)
    }
    pub fn enumerate(&self, url: &str, cancel: &Cancellation) -> Result<Vec<OnlineEntry>> {
        let mut entries = Vec::new();
        self.enumerate_stream(url, cancel, |entry| entries.push(entry))?;
        Ok(entries)
    }
    /// Emits metadata in order, once per row, before the helper exits.
    /// Callbacks run synchronously and must not block. Rows already emitted
    /// remain valid on a later error; callers must not re-add a final vector.
    pub fn enumerate_stream(
        &self,
        url: &str,
        cancel: &Cancellation,
        on_entry: impl FnMut(OnlineEntry),
    ) -> Result<()> {
        let provider = validate_url(url)?;
        self.metadata_stream(url, provider, cancel, on_entry)
    }
    pub fn search(
        &self,
        provider: &str,
        query: &str,
        cancel: &Cancellation,
    ) -> Result<Vec<OnlineEntry>> {
        ensure!(
            !query.trim().is_empty() && query.len() <= 512 && !query.chars().any(char::is_control),
            "invalid search query"
        );
        let prefix = match provider {
            "youtube" => "ytsearch20:",
            "soundcloud" => "scsearch20:",
            _ => anyhow::bail!("unsupported provider"),
        };
        self.metadata(&format!("{prefix}{}", query.trim()), provider, cancel)
    }
    fn metadata(
        &self,
        target: &str,
        provider: &str,
        cancel: &Cancellation,
    ) -> Result<Vec<OnlineEntry>> {
        let mut entries = Vec::new();
        self.metadata_stream(target, provider, cancel, |entry| entries.push(entry))?;
        Ok(entries)
    }
    fn metadata_stream(
        &self,
        target: &str,
        provider: &str,
        cancel: &Cancellation,
        mut on_entry: impl FnMut(OnlineEntry),
    ) -> Result<()> {
        check_cancel(cancel)?;
        let mut cmd = self.command(&self.work_dir)?;
        cmd.args([
            "--flat-playlist",
            "--lazy-playlist",
            "--dump-json",
            "--simulate",
            "--skip-download",
            "--ignore-errors",
            "--playlist-end",
            "500",
            "--",
            target,
        ]);
        let mut parser = MetadataLines::new(provider);
        let mut parse_error = None;
        let mut emit = |entry| {
            // A callback can cancel while several rows share one pipe chunk.
            if !cancel.load(Ordering::SeqCst) {
                on_entry(entry);
            }
        };
        let runner_cancel = Arc::new(AtomicBool::new(false));
        let finished = AtomicBool::new(false);
        // The runner callback cannot return an error. A private token stops its
        // process tree on parser failure without poisoning the caller's token.
        struct FinishOnDrop<'a>(&'a AtomicBool);
        impl Drop for FinishOnDrop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let out = std::thread::scope(|scope| {
            scope.spawn(|| {
                while !finished.load(Ordering::SeqCst) {
                    if cancel.load(Ordering::SeqCst) {
                        runner_cancel.store(true, Ordering::SeqCst);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            });
            let _finished = FinishOnDrop(&finished);
            run(
                &mut cmd,
                &runner_cancel,
                ProcessOptions {
                    timeout: Duration::from_secs(120),
                    max_output: 16 * 1024 * 1024,
                },
                |chunk| {
                    if parse_error.is_none() {
                        parse_error = parser.push(chunk, &mut emit).err();
                        if parse_error.is_some() {
                            runner_cancel.store(true, Ordering::SeqCst);
                        }
                    }
                },
            )
        });
        if let Some(error) = parse_error {
            return Err(error);
        }
        let out = out?;
        check_cancel(cancel)?;
        parser.finish(&mut emit)?;
        check_cancel(cancel)?;
        ensure!((out.status.success() && (target.starts_with("ytsearch20:") || target.starts_with("scsearch20:"))) || parser.emitted > 0,"provider metadata request failed (exit {:?}); login, region, rate limiting or provider changes may prevent access",out.status.code());
        Ok(())
    }
}
struct MetadataLines<'a> {
    provider: &'a str,
    pending: Vec<u8>,
    emitted: usize,
}
impl<'a> MetadataLines<'a> {
    fn new(provider: &'a str) -> Self {
        Self {
            provider,
            pending: Vec::new(),
            emitted: 0,
        }
    }
    fn push(&mut self, chunk: &[u8], emit: &mut impl FnMut(OnlineEntry)) -> Result<()> {
        for part in chunk.split_inclusive(|b| *b == b'\n') {
            let complete = part.last() == Some(&b'\n');
            let bytes = if complete {
                &part[..part.len() - 1]
            } else {
                part
            };
            ensure!(
                self.pending.len().saturating_add(bytes.len()) <= 1024 * 1024,
                "metadata limit exceeded"
            );
            self.pending.extend_from_slice(bytes);
            if complete {
                self.finish(emit)?;
            }
        }
        Ok(())
    }
    fn finish(&mut self, emit: &mut impl FnMut(OnlineEntry)) -> Result<()> {
        let text = std::str::from_utf8(&self.pending).context("provider returned invalid UTF-8")?;
        for mut entry in parse_metadata(text, self.provider)? {
            ensure!(self.emitted < 500, "metadata limit exceeded");
            entry.position = self.emitted;
            self.emitted += 1;
            emit(entry);
        }
        self.pending.clear();
        Ok(())
    }
}

pub fn parse_metadata(text: &str, provider: &str) -> Result<Vec<OnlineEntry>> {
    ensure!(
        matches!(provider, "youtube" | "soundcloud"),
        "unsupported provider"
    );
    ensure!(
        text.len() <= 16 * 1024 * 1024,
        "metadata size limit exceeded"
    );
    let mut entries = Vec::new();
    for line in text.lines().filter(|s| !s.trim().is_empty()) {
        ensure!(
            entries.len() < 500 && line.len() <= 1024 * 1024,
            "metadata limit exceeded"
        );
        let v: Value = serde_json::from_str(line)?;
        let mut url = v
            .get("webpage_url")
            .or_else(|| v.get("url"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if provider == "youtube"
            && !url.contains(":")
            && url.len() == 11
            && url
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            url = format!("https://www.youtube.com/watch?v={url}");
        }
        let safe = validate_url(&url).is_ok_and(|p| p == provider);
        if !safe {
            url.clear();
        }
        let title = v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Unavailable item")
            .chars()
            .filter(|c| !c.is_control())
            .take(512)
            .collect::<String>();
        let available =
            safe && !matches!(
                v.get("availability").and_then(Value::as_str),
                Some("private" | "needs_auth" | "premium_only" | "subscriber_only" | "unavailable")
            ) && !matches!(
                title.as_str(),
                "[Deleted video]" | "[Private video]" | "Unavailable item"
            );
        let duration = v
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.0);
        entries.push(OnlineEntry {
            title,
            url,
            provider: provider.to_string(),
            duration,
            position: entries.len(),
            available,
        });
    }
    Ok(entries)
}
