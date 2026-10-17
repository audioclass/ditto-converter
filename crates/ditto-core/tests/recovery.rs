mod common;
use common::{engine, fixture_ffmpeg};
use ditto_core::{recover_staging, Cancellation};
use std::{
    path::PathBuf,
    process::Command,
    sync::{atomic::AtomicBool, Arc},
};

fn private_dir() -> tempfile::TempDir {
    // macOS commonly exposes the temp root through /var -> /private/var.
    // Journal validation intentionally rejects noncanonical parents.
    let root = std::env::temp_dir();
    let root = root.canonicalize().unwrap();
    #[cfg(windows)]
    let root = PathBuf::from(root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    let dir = tempfile::tempdir_in(root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

fn cancel() -> Cancellation {
    Arc::new(AtomicBool::new(false))
}
fn source(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("source.wav");
    assert!(Command::new(fixture_ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=0.5",
            "-y"
        ])
        .arg(&path)
        .status()
        .unwrap()
        .success());
    path
}

fn abandon(root: &std::path::Path) -> (PathBuf, serde_json::Value) {
    source(root);
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_leaves_exact_recoverable_stage_not_unrelated_prefix_or_originals",
        ])
        .env("DITTO_CRASH_ROOT", root)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    let journal = root.join("journal.json");
    let record = serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
    (journal, record)
}

#[test]
#[cfg(unix)]
fn recovery_refuses_symlinks_hardlinks_and_unexpected_contents_before_deleting_anything() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for scenario in [
        "stage_symlink",
        "marker_symlink",
        "raw_symlink",
        "journal_symlink",
        "ancestor_symlink",
        "hardlink",
        "unexpected",
        "public_stage",
        "public_journal",
        "not_staging",
        "marker_escape",
        "wav_escape",
    ] {
        let dir = private_dir();
        let (mut journal, mut record) = abandon(dir.path());
        let stage = PathBuf::from(record["stage"].as_str().unwrap());
        let marker = stage.join(record["marker"].as_str().unwrap());
        let raw = stage.join("audio.pcm");
        let saved = std::fs::read(&raw).unwrap();
        match scenario {
            "stage_symlink" => {
                let moved = dir.path().join("moved");
                std::fs::rename(&stage, &moved).unwrap();
                symlink(&moved, &stage).unwrap();
            }
            "marker_symlink" => {
                let moved = dir.path().join("marker");
                std::fs::rename(&marker, &moved).unwrap();
                symlink(&moved, &marker).unwrap();
            }
            "raw_symlink" => {
                let moved = dir.path().join("original.pcm");
                std::fs::rename(&raw, &moved).unwrap();
                symlink(&moved, &raw).unwrap();
            }
            "journal_symlink" => {
                let moved = dir.path().join("real-journal");
                std::fs::rename(&journal, &moved).unwrap();
                symlink(&moved, &journal).unwrap();
            }
            "ancestor_symlink" => {
                let alias = dir.path().join("alias");
                symlink(dir.path(), &alias).unwrap();
                journal = alias.join("journal.json");
            }
            "hardlink" => {
                std::fs::hard_link(&raw, dir.path().join("original.pcm")).unwrap();
            }
            "unexpected" => {
                std::fs::write(stage.join("original.wav"), b"original").unwrap();
            }
            "public_stage" => {
                std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o777)).unwrap();
            }
            "public_journal" => {
                std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o666)).unwrap();
            }
            "not_staging" => {
                let moved = dir.path().join("originals");
                std::fs::rename(&stage, &moved).unwrap();
                record["stage"] = serde_json::json!(moved);
                std::fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
            }
            "marker_escape" => {
                record["marker"] = serde_json::json!(format!(
                    "../{}/{}",
                    stage.file_name().unwrap().to_str().unwrap(),
                    marker.file_name().unwrap().to_str().unwrap()
                ));
                std::fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
            }
            "wav_escape" => {
                let wav = record["wav"].as_str().unwrap();
                record["wav"] = serde_json::json!(format!(
                    "../{}/{wav}",
                    stage.file_name().unwrap().to_str().unwrap()
                ));
                std::fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(recover_staging(&journal).is_err(), "accepted {scenario}");
        let actual_stage = if scenario == "stage_symlink" {
            dir.path().join("moved")
        } else if scenario == "not_staging" {
            dir.path().join("originals")
        } else {
            stage
        };
        assert_eq!(
            std::fs::read(actual_stage.join("audio.pcm")).unwrap(),
            saved,
            "changed {scenario}"
        );
    }
}

#[test]
#[cfg(unix)]
fn journal_cannot_be_created_through_a_symlinked_or_public_parent() {
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        for public in [false, true] {
            let dir = private_dir();
            let input = source(dir.path());
            let app = dir.path().join("app");
            std::fs::create_dir(&app).unwrap();
            std::fs::set_permissions(
                &app,
                std::fs::Permissions::from_mode(if public { 0o777 } else { 0o700 }),
            )
            .unwrap();
            let alias = dir.path().join("alias");
            symlink(&app, &alias).unwrap();
            let journal = if public {
                app.join("journal")
            } else {
                alias.join("journal")
            };
            assert!(engine()
                .convert_tracked(
                    &input,
                    dir.path(),
                    "export",
                    0.0,
                    None,
                    &cancel(),
                    |_| {},
                    &journal
                )
                .is_err());
            assert!(!journal.exists());
            assert!(!dir.path().join("export.wav").exists());
        }
    }
}

#[test]
fn normal_success_and_progress_cancellation_remove_journal_and_staging() {
    for cancelled in [false, true] {
        let dir = private_dir();
        let input = source(dir.path());
        let journal = dir.path().join("journal.json");
        let c = cancel();
        let result = engine().convert_tracked(
            &input,
            dir.path(),
            "export",
            0.0,
            None,
            &c,
            |p| {
                if p < 1.0 {
                    assert!(journal.exists());
                    if cancelled {
                        c.store(true, std::sync::atomic::Ordering::Release);
                    }
                }
            },
            &journal,
        );
        if cancelled {
            assert!(matches!(result, Err(ditto_core::Error::Cancelled)));
        } else {
            ditto_core::audio::validate_wav(&result.unwrap(), &cancel()).unwrap();
        }
        assert!(!journal.exists());
        assert!(!recover_staging(&journal).unwrap());
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            if cancelled { 1 } else { 2 }
        );
    }
}

#[test]
fn corrupt_mismatched_and_existing_journals_are_preserved() {
    for scenario in ["corrupt", "oversize", "mismatch"] {
        let dir = private_dir();
        let (journal, record) = abandon(dir.path());
        let stage = PathBuf::from(record["stage"].as_str().unwrap());
        match scenario {
            "corrupt" => std::fs::write(&journal, b"not JSON").unwrap(),
            "oversize" => std::fs::write(&journal, vec![b'x'; 16385]).unwrap(),
            "mismatch" => std::fs::write(
                stage.join(record["marker"].as_str().unwrap()),
                b"wrong marker",
            )
            .unwrap(),
            _ => unreachable!(),
        }
        let saved = std::fs::read(&journal).unwrap();
        assert!(recover_staging(&journal).is_err());
        assert_eq!(std::fs::read(&journal).unwrap(), saved);
        assert!(stage.join("audio.pcm").exists());
        assert!(engine()
            .convert_tracked(
                &dir.path().join("source.wav"),
                dir.path(),
                "export",
                0.0,
                None,
                &cancel(),
                |_| {},
                &journal
            )
            .is_err());
        assert_eq!(std::fs::read(&journal).unwrap(), saved);
        assert!(!dir.path().join("export.wav").exists());
    }
}

#[test]
fn crash_after_publication_preserves_successful_export() {
    let dir = private_dir();
    let input = source(dir.path());
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_leaves_exact_recoverable_stage_not_unrelated_prefix_or_originals",
        ])
        .env("DITTO_CRASH_ROOT", dir.path())
        .env("DITTO_CRASH_PUBLISHED", "1")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    let output = dir.path().join("export.wav");
    let original = std::fs::read(&output).unwrap();
    assert!(recover_staging(&dir.path().join("journal.json")).unwrap());
    assert_eq!(std::fs::read(&output).unwrap(), original);
    assert!(input.exists());
    ditto_core::audio::validate_wav(&output, &cancel()).unwrap();
}

#[test]
fn crash_leaves_exact_recoverable_stage_not_unrelated_prefix_or_originals() {
    if let Some(root) = std::env::var_os("DITTO_CRASH_ROOT") {
        let root = PathBuf::from(root);
        engine()
            .convert_tracked(
                &root.join("source.wav"),
                &root,
                "export",
                0.0,
                None,
                &cancel(),
                |p| {
                    if std::env::var_os("DITTO_CRASH_PUBLISHED").is_none() || p == 1.0 {
                        std::process::exit(73);
                    }
                },
                &root.join("journal.json"),
            )
            .unwrap();
        panic!("progress did not run");
    }
    let dir = private_dir();
    let input = source(dir.path());
    let original = std::fs::read(&input).unwrap();
    let unrelated = dir.path().join(".ditto-unrelated");
    std::fs::create_dir(&unrelated).unwrap();
    std::fs::write(unrelated.join("original.wav"), b"preserve").unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_leaves_exact_recoverable_stage_not_unrelated_prefix_or_originals",
        ])
        .env("DITTO_CRASH_ROOT", dir.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    let journal = dir.path().join("journal.json");
    assert!(journal.exists());
    assert!(recover_staging(&journal).unwrap());
    assert!(!journal.exists());
    assert!(!recover_staging(&journal).unwrap());
    assert_eq!(std::fs::read(&input).unwrap(), original);
    assert_eq!(
        std::fs::read(unrelated.join("original.wav")).unwrap(),
        b"preserve"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
