//! Exact, opt-in export staging journal; never scans output directories.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const LIMIT: u64 = 16 * 1024;
#[cfg(windows)]
#[path = "windows_staging.rs"]
mod windows;
fn random_name(value: &str, prefix: &str, length: usize) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|tail| tail.len() == length && tail.bytes().all(|b| b.is_ascii_alphanumeric()))
}
#[cfg(not(windows))]
fn exact_path(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    if absolute != path.canonicalize()? {
        return Err(Error::Invalid(
            "symlink or noncanonical staging path".into(),
        ));
    }
    Ok(())
}
#[cfg(unix)]
fn owned(metadata: &fs::Metadata, private: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & if private { 0o077 } else { 0o022 } != 0
    {
        return Err(Error::Invalid(
            "staging ownership or permissions mismatch".into(),
        ));
    }
    Ok(())
}
#[cfg(not(any(unix, windows)))]
fn owned(_metadata: &fs::Metadata, _private: bool) -> Result<()> {
    Err(Error::Invalid(
        "safe staging recovery requires native ownership validation on this platform".into(),
    ))
}
#[cfg(not(windows))]
fn validate_file(metadata: &fs::Metadata, private: bool) -> Result<()> {
    if !metadata.file_type().is_file() {
        return Err(Error::Invalid("staging file is not regular".into()));
    }
    owned(metadata, private)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(Error::Invalid("hard-linked staging file".into()));
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn validate_directory(metadata: &fs::Metadata, private: bool) -> Result<()> {
    if !metadata.file_type().is_dir() {
        return Err(Error::Invalid("staging path is not a directory".into()));
    }
    owned(metadata, private)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    stage: PathBuf,
    marker: String,
    wav: String,
}
pub(crate) struct Journal {
    path: PathBuf,
    stage: PathBuf,
}
impl Journal {
    pub(crate) fn create(path: &Path, stage: &Path, wav: &Path) -> Result<Self> {
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let parent = absolute
            .parent()
            .ok_or_else(|| Error::Invalid("journal requires a parent directory".into()))?;
        #[cfg(unix)]
        {
            exact_path(parent)?;
            validate_directory(&fs::symlink_metadata(parent)?, true)?;
        }
        #[cfg(windows)]
        let _windows_guards = windows::registration(parent)?;
        #[cfg(not(any(unix, windows)))]
        {
            // Other platforms retain tracking, but recovery still fails closed.
            if !fs::symlink_metadata(parent)?.file_type().is_dir() {
                return Err(Error::Invalid(
                    "journal parent must be a real directory".into(),
                ));
            }
        }
        let path = absolute.as_path();
        let marker = tempfile::Builder::new()
            .prefix(".ditto-owner-")
            .rand_bytes(32)
            .tempfile_in(stage)?;
        let marker_name = marker
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        marker.as_file().write_all(marker_name.as_bytes())?;
        marker.as_file().sync_all()?;
        marker.keep().map_err(|e| Error::Io(e.error))?;
        let stage = stage.canonicalize()?;
        let record = Record {
            version: 1,
            stage: stage.clone(),
            marker: marker_name,
            wav: wav.file_name().unwrap().to_str().unwrap().to_owned(),
        };
        let bytes = serde_json::to_vec(&record)?;
        if bytes.len() as u64 > LIMIT {
            return Err(Error::Invalid("staging journal exceeds limit".into()));
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(Self {
            path: path.to_owned(),
            stage,
        })
    }
}
impl Drop for Journal {
    fn drop(&mut self) {
        // Stage's TempDir is dropped first; retain the record if cleanup failed.
        if !self.stage.exists() {
            let _ = fs::remove_file(&self.path);
        }
    }
}
/// Recover one abandoned export. Call only at startup, before starting workers.
/// Returns false when the journal does not exist; invalid records fail closed.
#[cfg(windows)]
pub fn recover_staging(journal_path: &Path) -> Result<bool> {
    windows::recover(journal_path)
}
#[cfg(not(windows))]
pub fn recover_staging(journal_path: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(journal_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    validate_file(&metadata, true)?;
    exact_path(journal_path)?;
    if metadata.len() > LIMIT {
        return Err(Error::Invalid("invalid staging journal".into()));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(journal_path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(Error::Invalid("invalid staging journal".into()));
    }
    let record: Record = serde_json::from_slice(&bytes)?;
    let stage_name = record
        .stage
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if record.version != 1
        || !record.stage.is_absolute()
        || !random_name(stage_name, ".ditto-", 6)
        || !random_name(&record.marker, ".ditto-owner-", 32)
        || !random_name(&record.wav, ".ditto-wav-", 16)
    {
        return Err(Error::Invalid("invalid staging journal record".into()));
    }
    exact_path(&record.stage)?;
    validate_directory(&fs::symlink_metadata(&record.stage)?, true)?;
    let marker = record.stage.join(&record.marker);
    // Validate the entire bounded directory before unlinking anything. Never recurse.
    let mut files = Vec::new();
    for entry in fs::read_dir(&record.stage)?.take(4) {
        let entry = entry?;
        let name = entry.file_name();
        if name != "audio.pcm" && name != record.marker.as_str() && name != record.wav.as_str() {
            return Err(Error::Invalid("unexpected staging contents".into()));
        }
        validate_file(
            &fs::symlink_metadata(entry.path())?,
            name == record.marker.as_str(),
        )?;
        files.push(entry.path());
    }
    let marker_meta = fs::symlink_metadata(&marker)?;
    if marker_meta.len() != record.marker.len() as u64
        || fs::read(&marker)? != record.marker.as_bytes()
    {
        return Err(Error::Invalid("staging marker mismatch".into()));
    }
    for file in files {
        if file != marker {
            fs::remove_file(file)?;
        }
    }
    fs::remove_file(marker)?;
    fs::remove_dir(&record.stage)?;
    fs::remove_file(journal_path)?;
    Ok(true)
}
