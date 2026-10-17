//! Immutable app-embedded development bootstrap. There is deliberately no unsigned updater.
use crate::{
    provider::{check_cancel, private_dir},
    Cancellation,
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    Raw,
    Zip,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentPin {
    pub version: String,
    pub url: String,
    pub asset_id: u64,
    pub length: u64,
    pub sha256: String,
    pub filename: String,
    pub archive: ArchiveKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackManifest {
    pub platform: String,
    pub tuple_id: String,
    pub yt_dlp: ComponentPin,
    pub deno: ComponentPin,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledPack {
    pub platform: String,
    pub tuple_id: String,
    pub yt_dlp: PathBuf,
    pub deno: PathBuf,
}
#[derive(Clone, Debug)]
pub struct ComponentManager {
    root: PathBuf,
}
pub fn approved_manifest(platform: &str) -> Result<PackManifest> {
    let all: Vec<PackManifest> =
        serde_json::from_str(include_str!("../assets/approved-tuples.json"))?;
    all.into_iter()
        .find(|p| p.platform == platform)
        .context("unapproved component platform")
}
pub fn verify_bytes(bytes: &[u8], pin: &ComponentPin) -> Result<()> {
    ensure!(
        bytes.len() as u64 == pin.length,
        "component length mismatch"
    );
    ensure!(
        format!("{:x}", Sha256::digest(bytes)) == pin.sha256,
        "component SHA-256 mismatch"
    );
    Ok(())
}
fn hash_file(path: &Path) -> Result<(u64, String)> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "regular component file required"
    );
    let mut f = File::open(path)?;
    let mut hash = Sha256::new();
    let mut n = 0;
    let mut buf = [0u8; 65536];
    loop {
        let read = f.read(&mut buf)?;
        if read == 0 {
            break;
        }
        n += read as u64;
        hash.update(&buf[..read]);
    }
    Ok((n, format!("{:x}", hash.finalize())))
}
fn verify_file(path: &Path, pin: &ComponentPin) -> Result<()> {
    let (n, h) = hash_file(path)?;
    ensure!(
        n == pin.length && h == pin.sha256,
        "component length/SHA-256 mismatch"
    );
    Ok(())
}
fn deno_bytes(path: &Path, pin: &ComponentPin) -> Result<Vec<u8>> {
    verify_file(path, pin)?;
    let mut zip = zip::ZipArchive::new(File::open(path)?)?;
    ensure!(zip.len() == 1, "unexpected archive members");
    let member = zip.by_index(0)?;
    ensure!(
        member.name() == pin.filename && member.enclosed_name().is_some(),
        "unexpected archive path"
    );
    ensure!(
        member.is_file() && member.size() > 0 && member.size() <= 256 * 1024 * 1024,
        "archive expansion limit"
    );
    if let Some(mode) = member.unix_mode() {
        ensure!(
            mode & 0o170000 == 0 || mode & 0o170000 == 0o100000,
            "archive link rejected"
        );
    }
    let mut bytes = Vec::new();
    member.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 256 * 1024 * 1024, "archive expansion limit");
    Ok(bytes)
}
fn download(
    pin: &ComponentPin,
    destination: &Path,
    cancel: &Cancellation,
    progress: &mut impl FnMut(u64),
) -> Result<()> {
    check_cancel(cancel)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    let result = (|| -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let transfer = async {
                let policy = reqwest::redirect::Policy::custom(|attempt| {
                    let u = attempt.url();
                    if attempt.previous().len() > 5
                        || u.scheme() != "https"
                        || !matches!(
                            u.host_str(),
                            Some(
                                "github.com"
                                    | "release-assets.githubusercontent.com"
                                    | "objects.githubusercontent.com"
                            )
                        )
                    {
                        attempt.error("unapproved component redirect")
                    } else {
                        attempt.follow()
                    }
                });
                let client = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(20))
                    .timeout(Duration::from_secs(600))
                    .redirect(policy)
                    .build()?;
                let mut response = client
                    .get(&pin.url)
                    .header("User-Agent", "DittoConverter-development-bootstrap")
                    .send()
                    .await?
                    .error_for_status()?;
                if let Some(length) = response.content_length() {
                    ensure!(length == pin.length, "component HTTP length mismatch");
                }
                let mut total = 0u64;
                let mut hash = Sha256::new();
                while let Some(bytes) = response.chunk().await? {
                    check_cancel(cancel)?;
                    total += bytes.len() as u64;
                    ensure!(total <= pin.length, "component exceeds pinned length");
                    hash.update(&bytes);
                    file.write_all(&bytes)?;
                    progress(bytes.len() as u64);
                }
                ensure!(
                    total == pin.length && format!("{:x}", hash.finalize()) == pin.sha256,
                    "component length/SHA-256 mismatch"
                );
                file.sync_all()?;
                Ok(())
            };
            let cancelled = async {
                loop {
                    if check_cancel(cancel).is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            };
            tokio::select! {biased;_ = cancelled => bail!("cancelled"),result=transfer=>result}
        })
    })();
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
struct Stage(PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl ComponentManager {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn accept_update_manifest(&self, _bytes: &[u8]) -> Result<()> {
        bail!("independent updates disabled: no configured authenticated publisher manifest/key; upgrade the application for a new pinned tuple")
    }
    fn pointer(&self) -> PathBuf {
        self.root.join("active.json")
    }
    fn read_pack(&self, path: &Path) -> Result<InstalledPack> {
        let bytes = fs::read(path)?;
        ensure!(bytes.len() < 4096, "invalid pack pointer");
        let manifest: PackManifest = approved_manifest(
            serde_json::from_slice::<InstalledPack>(&bytes)?
                .platform
                .as_str(),
        )?;
        let dir = self
            .root
            .join(format!("{}-{}", manifest.tuple_id, manifest.platform));
        let pack = InstalledPack {
            platform: manifest.platform.clone(),
            tuple_id: manifest.tuple_id.clone(),
            yt_dlp: dir.join(&manifest.yt_dlp.filename),
            deno: dir.join(&manifest.deno.filename),
        };
        // Do not trust any executable paths or checksums read from writable JSON.
        verify_file(&pack.yt_dlp, &manifest.yt_dlp)?;
        let deno = deno_bytes(&dir.join("deno.zip"), &manifest.deno)?;
        let (n, hash) = hash_file(&pack.deno)?;
        ensure!(
            n == deno.len() as u64 && hash == format!("{:x}", Sha256::digest(&deno)),
            "installed runtime hash mismatch"
        );
        Ok(pack)
    }
    pub fn status(&self) -> Result<Option<InstalledPack>> {
        if !self.pointer().exists() {
            return Ok(None);
        }
        Ok(Some(self.read_pack(&self.pointer())?))
    }
    pub fn install(
        &self,
        platform: &str,
        cancel: &Cancellation,
        mut on_progress: impl FnMut(f64),
    ) -> Result<InstalledPack> {
        check_cancel(cancel)?;
        let manifest = approved_manifest(platform)?;
        ensure!(
            platform == host_platform(),
            "cannot activate helpers for a different host platform"
        );
        private_dir(&self.root)?;
        let lock = self.root.join("install.lock");
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock)
            .context("component installation busy; stale locks require explicit recovery")?;
        let _lock = Lock(lock);
        if let Some(active) = self.status()? {
            if active.tuple_id == manifest.tuple_id && active.platform == platform {
                return Ok(active);
            }
        }
        let stage = self.root.join("staging");
        ensure!(!stage.exists(), "stale component staging requires cleanup");
        private_dir(&stage)?;
        let _stage = Stage(stage.clone());
        let total = manifest.yt_dlp.length + manifest.deno.length;
        let mut done = 0u64;
        let mut progress = |bytes| {
            done += bytes;
            on_progress(done as f64 / total as f64);
        };
        download(
            &manifest.yt_dlp,
            &stage.join(&manifest.yt_dlp.filename),
            cancel,
            &mut progress,
        )?;
        download(
            &manifest.deno,
            &stage.join("deno.zip"),
            cancel,
            &mut progress,
        )?;
        let bytes = deno_bytes(&stage.join("deno.zip"), &manifest.deno)?;
        let mut deno = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(stage.join(&manifest.deno.filename))?;
        deno.write_all(&bytes)?;
        deno.sync_all()?;
        drop(deno);
        for pin in [&manifest.yt_dlp, &manifest.deno] {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(stage.join(&pin.filename), fs::Permissions::from_mode(0o700))?;
            }
            check_cancel(cancel)?;
            let mut cmd = std::process::Command::new(stage.join(&pin.filename));
            cmd.arg("--version").current_dir(&stage);
            cmd.env("HOME", &stage)
                .env("USERPROFILE", &stage)
                .env("TMPDIR", &stage)
                .env("TMP", &stage)
                .env("TEMP", &stage)
                .env("DENO_DIR", stage.join("deno-cache"))
                .env("DENO_NO_UPDATE_CHECK", "1");
            let out = ditto_core::process::run(
                &mut cmd,
                cancel,
                ditto_core::process::ProcessOptions {
                    timeout: Duration::from_secs(30),
                    max_output: 65536,
                },
                |_| {},
            )?;
            ensure!(
                out.status.success() && String::from_utf8_lossy(&out.stdout).contains(&pin.version),
                "component version smoke test failed"
            );
        }
        check_cancel(cancel)?;
        let dest = self
            .root
            .join(format!("{}-{}", manifest.tuple_id, platform));
        ensure!(
            !dest.exists(),
            "pack directory already exists without active pointer; explicit recovery required"
        );
        fs::rename(&stage, &dest)?;
        let pack = InstalledPack {
            platform: platform.to_string(),
            tuple_id: manifest.tuple_id,
            yt_dlp: dest.join(manifest.yt_dlp.filename),
            deno: dest.join(manifest.deno.filename),
        };
        self.activate(&pack)?;
        self.status()?.context("activation failed")
    }
    fn activate(&self, pack: &InstalledPack) -> Result<()> {
        for pointer in [self.pointer(), self.root.join("previous.json")] {
            match fs::symlink_metadata(&pointer) {
                Ok(m) => ensure!(
                    m.is_file() && !m.file_type().is_symlink(),
                    "component pointer must be a regular file"
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let pending = self.root.join("pending.json");
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)?;
        f.write_all(&serde_json::to_vec(pack)?)?;
        f.sync_all()?;
        // Retain old tuple and pointer; activation failure never replaces executable bytes.
        if self.pointer().exists() {
            let backup = self.root.join("previous.pending.json");
            let mut out = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&backup)?;
            let mut input = File::open(self.pointer())?.take(4097);
            let n = std::io::copy(&mut input, &mut out)?;
            ensure!(n <= 4096, "invalid active pointer");
            out.sync_all()?;
            drop(out);
            fs::rename(backup, self.root.join("previous.json"))?;
        }
        fs::rename(&pending, self.pointer())?;
        Ok(())
    }
    pub fn rollback(&self) -> Result<InstalledPack> {
        private_dir(&self.root)?;
        let lock = self.root.join("install.lock");
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock)?;
        let _lock = Lock(lock);
        let pack = self.read_pack(&self.root.join("previous.json"))?;
        self.activate(&pack)?;
        self.status()?.context("rollback failed")
    }
}
pub fn host_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        _ => "unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_members_are_allowlisted_before_any_executable_is_materialized() {
        fn archive(names: &[&str]) -> (tempfile::TempDir, ComponentPin) {
            let t = tempfile::tempdir().unwrap();
            let file = File::create(t.path().join("runtime.zip")).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            for name in names {
                writer
                    .start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(b"fixture runtime").unwrap();
            }
            writer.finish().unwrap();
            let bytes = fs::read(t.path().join("runtime.zip")).unwrap();
            let pin = ComponentPin {
                version: "fixture".into(),
                url: "https://example.invalid".into(),
                asset_id: 0,
                length: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                filename: "deno".into(),
                archive: ArchiveKind::Zip,
            };
            (t, pin)
        }
        for names in [
            &["../deno"][..],
            &["/deno"][..],
            &["deno", "unexpected"][..],
        ] {
            let (t, pin) = archive(names);
            assert!(deno_bytes(&t.path().join("runtime.zip"), &pin).is_err());
            assert!(!t.path().join("deno").exists());
        }
        let (t, pin) = archive(&["deno"]);
        assert_eq!(
            deno_bytes(&t.path().join("runtime.zip"), &pin).unwrap(),
            b"fixture runtime"
        );
    }
    #[test]
    fn pointer_activation_retains_previous_and_failed_staging_preserves_active() {
        let t = tempfile::tempdir().unwrap();
        let manager = ComponentManager::new(t.path().to_path_buf());
        let pack = InstalledPack {
            platform: "fixture".into(),
            tuple_id: "one".into(),
            yt_dlp: t.path().join("yt"),
            deno: t.path().join("deno"),
        };
        manager.activate(&pack).unwrap();
        let first = fs::read(manager.pointer()).unwrap();
        let mut second = pack.clone();
        second.tuple_id = "two".into();
        manager.activate(&second).unwrap();
        assert_eq!(fs::read(t.path().join("previous.json")).unwrap(), first);
        let active = fs::read(manager.pointer()).unwrap();
        fs::write(t.path().join("pending.json"), b"interrupted staging").unwrap();
        assert!(manager.activate(&pack).is_err());
        assert_eq!(fs::read(manager.pointer()).unwrap(), active);
        assert!(manager.rollback().is_err()); // fixture is intentionally NOT an approved executable tuple
    }
    #[cfg(unix)]
    #[test]
    fn activation_never_follows_previous_pointer_symlink() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("components");
        private_dir(&root).unwrap();
        let victim = t.path().join("victim");
        fs::write(&victim, b"original").unwrap();
        fs::write(root.join("active.json"), b"old pointer").unwrap();
        symlink(&victim, root.join("previous.json")).unwrap();
        let manager = ComponentManager::new(root.clone());
        let pack = InstalledPack {
            platform: "fixture".into(),
            tuple_id: "fixture".into(),
            yt_dlp: root.join("yt"),
            deno: root.join("deno"),
        };
        assert!(manager.activate(&pack).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"original");
        assert_eq!(fs::read(root.join("active.json")).unwrap(), b"old pointer");
    }
    #[test]
    fn component_download_cancel_interrupts_stalled_network_and_removes_staging() {
        use std::{
            net::TcpListener,
            sync::{
                atomic::{AtomicBool, Ordering},
                Arc,
            },
            thread,
            time::Instant,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut b = [0u8; 4096];
            let _ = stream.read(&mut b);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n")
                .unwrap();
            thread::sleep(Duration::from_secs(3));
        });
        let t = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let stopper = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            c.store(true, Ordering::SeqCst);
        });
        let pin = ComponentPin {
            version: "fixture".into(),
            url: format!("http://127.0.0.1:{port}/fixture"),
            asset_id: 0,
            length: 5,
            sha256: format!("{:x}", Sha256::digest(b"hello")),
            filename: "fixture".into(),
            archive: ArchiveKind::Raw,
        };
        let started = Instant::now();
        assert!(download(&pin, &t.path().join("payload"), &cancel, &mut |_| {}).is_err());
        let elapsed = started.elapsed();
        stopper.join().unwrap();
        server.join().unwrap();
        assert!(
            elapsed < Duration::from_secs(1),
            "cancellation delayed {elapsed:?}"
        );
    }
}
