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

fn tree_snapshot(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn collect(
        root: &std::path::Path,
        path: &std::path::Path,
        entries: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
    ) {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        let bytes = if metadata.is_symlink() {
            format!("link:{:?}", std::fs::read_link(path).unwrap()).into_bytes()
        } else if metadata.is_dir() {
            b"directory".to_vec()
        } else {
            std::fs::read(path).unwrap()
        };
        entries.insert(path.strip_prefix(root).unwrap().to_owned(), bytes);
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                collect(root, &entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = std::collections::BTreeMap::new();
    collect(root, root, &mut entries);
    entries
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
        "public_marker",
        "public_raw",
        "public_wav",
        "not_staging",
        "marker_escape",
        "wav_escape",
        "wav_symlink",
        "journal_hardlink",
        "marker_hardlink",
        "wav_hardlink",
        "marker_directory",
        "raw_directory",
        "wav_directory",
        "unexpected_directory",
    ] {
        let dir = private_dir();
        let (mut journal, mut record) = abandon(dir.path());
        let stage = PathBuf::from(record["stage"].as_str().unwrap());
        let marker = stage.join(record["marker"].as_str().unwrap());
        let raw = stage.join("audio.pcm");
        let wav = stage.join(record["wav"].as_str().unwrap());
        match scenario {
            "wav_symlink" => {
                let moved = dir.path().join("original.wav");
                std::fs::rename(&wav, &moved).unwrap();
                symlink(&moved, &wav).unwrap();
            }
            "journal_hardlink" | "marker_hardlink" | "wav_hardlink" => {
                let target = match scenario {
                    "journal_hardlink" => &journal,
                    "marker_hardlink" => &marker,
                    _ => &wav,
                };
                std::fs::hard_link(target, dir.path().join("outside-link")).unwrap();
            }
            "marker_directory" | "raw_directory" | "wav_directory" | "unexpected_directory" => {
                let target = match scenario {
                    "marker_directory" => marker.clone(),
                    "raw_directory" => raw.clone(),
                    "wav_directory" => wav.clone(),
                    _ => stage.join("originals"),
                };
                if target.is_file() {
                    std::fs::rename(&target, dir.path().join("saved-file")).unwrap();
                }
                std::fs::create_dir(&target).unwrap();
                std::fs::write(target.join("original.wav"), b"nested original").unwrap();
            }
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
            "public_marker" | "public_raw" | "public_wav" => {
                let (path, mode) = match scenario {
                    "public_marker" => (&marker, 0o644),
                    "public_raw" => (&raw, 0o666),
                    _ => (&wav, 0o666),
                };
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
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
        let before = tree_snapshot(dir.path());
        assert!(recover_staging(&journal).is_err(), "accepted {scenario}");
        assert_eq!(tree_snapshot(dir.path()), before, "changed {scenario}");
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
#[cfg(unix)]
fn recovery_refuses_a_journal_parent_that_became_public_before_deleting_anything() {
    use std::os::unix::fs::PermissionsExt;
    let dir = private_dir();
    let (journal, record) = abandon(dir.path());
    let stage = PathBuf::from(record["stage"].as_str().unwrap());
    let journal_bytes = std::fs::read(&journal).unwrap();
    let raw_bytes = std::fs::read(stage.join("audio.pcm")).unwrap();
    let original_bytes = std::fs::read(dir.path().join("source.wav")).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();

    let result = recover_staging(&journal);
    // Restore the private fixture even if the assertion fails.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        result.is_err(),
        "accepted publicly writable journal storage"
    );
    assert_eq!(std::fs::read(&journal).unwrap(), journal_bytes);
    assert_eq!(std::fs::read(stage.join("audio.pcm")).unwrap(), raw_bytes);
    assert_eq!(
        std::fs::read(dir.path().join("source.wav")).unwrap(),
        original_bytes
    );
    assert!(recover_staging(&journal).unwrap());
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
fn recovery_enforces_the_exact_journal_byte_limit() {
    for length in [16 * 1024, 16 * 1024 + 1] {
        let dir = private_dir();
        let (journal, _) = abandon(dir.path());
        let mut bytes = std::fs::read(&journal).unwrap();
        bytes.resize(length, b' '); // Both records are valid JSON, differing only in size.
        std::fs::write(&journal, bytes).unwrap();
        let before = tree_snapshot(dir.path());
        if length == 16 * 1024 {
            assert!(recover_staging(&journal).unwrap());
            assert!(!journal.exists());
        } else {
            assert!(recover_staging(&journal).is_err());
            assert_eq!(tree_snapshot(dir.path()), before);
        }
    }
}

#[test]
fn recovery_rejects_invalid_record_identity_and_missing_owner_without_mutation() {
    for scenario in [
        "version",
        "unknown_field",
        "missing_field",
        "relative_stage",
        "stage_name",
        "marker_name",
        "wav_name",
        "missing_marker",
        "oversized_marker",
    ] {
        let dir = private_dir();
        let (journal, mut record) = abandon(dir.path());
        let stage = PathBuf::from(record["stage"].as_str().unwrap());
        let marker = stage.join(record["marker"].as_str().unwrap());
        match scenario {
            "version" => record["version"] = 2.into(),
            "unknown_field" => record["output"] = "source.wav".into(),
            "missing_field" => {
                record.as_object_mut().unwrap().remove("wav");
            }
            "relative_stage" => {
                record["stage"] = stage.file_name().unwrap().to_str().unwrap().into()
            }
            "stage_name" => {
                let moved = dir.path().join(".ditto-not-random");
                std::fs::rename(&stage, &moved).unwrap();
                record["stage"] = serde_json::json!(moved);
            }
            "marker_name" => record["marker"] = format!(".ditto-owner-{}", "_".repeat(32)).into(),
            "wav_name" => record["wav"] = format!(".ditto-wav-{}", "_".repeat(16)).into(),
            "missing_marker" => std::fs::remove_file(marker).unwrap(),
            "oversized_marker" => std::fs::write(marker, vec![b'x'; 16 * 1024 + 1]).unwrap(),
            _ => unreachable!(),
        }
        std::fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
        let before = tree_snapshot(dir.path());
        assert!(recover_staging(&journal).is_err(), "accepted {scenario}");
        assert_eq!(tree_snapshot(dir.path()), before, "changed {scenario}");
    }
}

#[test]
fn recovery_accepts_only_the_owned_subset_of_optional_pcm_and_wav_files() {
    for (pcm, wav) in [(false, false), (true, false), (false, true), (true, true)] {
        let dir = private_dir();
        let (journal, record) = abandon(dir.path());
        let stage = PathBuf::from(record["stage"].as_str().unwrap());
        if !pcm {
            std::fs::remove_file(stage.join("audio.pcm")).unwrap();
        }
        if !wav {
            std::fs::remove_file(stage.join(record["wav"].as_str().unwrap())).unwrap();
        }
        let input = dir.path().join("source.wav");
        let original = std::fs::read(&input).unwrap();
        assert!(recover_staging(&journal).unwrap());
        assert!(!stage.exists());
        assert!(!journal.exists());
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert!(!recover_staging(&journal).unwrap());
    }
}

#[test]
fn recovering_one_journal_preserves_another_owned_stage_and_existing_exports() {
    let dir = private_dir();
    let (journal, first) = abandon(dir.path());
    let first_journal = dir.path().join("first.json");
    std::fs::rename(journal, &first_journal).unwrap();
    let (second_journal, second) = abandon(dir.path());
    let first_stage = PathBuf::from(first["stage"].as_str().unwrap());
    let second_stage = PathBuf::from(second["stage"].as_str().unwrap());
    assert_ne!(first_stage, second_stage);
    std::fs::write(dir.path().join("export.wav"), b"existing successful export").unwrap();
    std::fs::create_dir(dir.path().join(".ditto-unowned")).unwrap();
    std::fs::write(
        dir.path().join(".ditto-unowned/original.wav"),
        b"unowned original",
    )
    .unwrap();
    let mut expected = tree_snapshot(dir.path());
    expected.retain(|path, _| {
        !path.starts_with(first_stage.strip_prefix(dir.path()).unwrap())
            && path != std::path::Path::new("first.json")
    });
    assert!(recover_staging(&first_journal).unwrap());
    assert_eq!(tree_snapshot(dir.path()), expected);
    assert!(second_stage.exists());
    assert!(recover_staging(&second_journal).unwrap());
    assert_eq!(
        std::fs::read(dir.path().join("export.wav")).unwrap(),
        b"existing successful export"
    );
}

#[test]
fn crash_after_publication_preserves_successful_export() {
    let dir = private_dir();
    let input = source(dir.path());
    let input_bytes = std::fs::read(&input).unwrap();
    let existing = dir.path().join("export.wav");
    std::fs::write(&existing, b"existing export must not be clobbered").unwrap();
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
    let output = dir.path().join("export (2).wav");
    let original = std::fs::read(&output).unwrap();
    assert!(recover_staging(&dir.path().join("journal.json")).unwrap());
    assert_eq!(std::fs::read(&output).unwrap(), original);
    assert_eq!(std::fs::read(&input).unwrap(), input_bytes);
    assert_eq!(
        std::fs::read(existing).unwrap(),
        b"existing export must not be clobbered"
    );
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
