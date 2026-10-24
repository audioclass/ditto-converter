//! Export publication contracts exercised with real helpers and independent fixtures.
mod common;

use common::{engine, fixture_ffmpeg};
use ditto_core::{audio::validate_wav, Cancellation, Error};
use std::{
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

fn cancel() -> Cancellation {
    Arc::new(AtomicBool::new(false))
}

fn fixture(path: &Path, codec: &str) {
    assert!(Command::new(fixture_ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.5:sample_rate=44100",
            "-c:a",
            codec,
        ])
        .arg(path)
        .status()
        .unwrap()
        .success());
}

fn entries(path: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

#[test]
fn short_real_decode_is_validated_before_publication_or_collision_search() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.flac");
    fixture(&input, "flac");
    let mut bytes = std::fs::read(&input).unwrap();
    assert_eq!(&bytes[..4], b"fLaC");
    assert_eq!(bytes[4] & 0x7f, 0, "first block must be STREAMINFO");
    // STREAMINFO's low 36 bits at bytes 18..26 declare the total samples.
    // Keep the real encoded frames intact, but declare twice their duration.
    let packed = u64::from_be_bytes(bytes[18..26].try_into().unwrap());
    assert_eq!(packed & 0x0f_ffff_ffff, 22050);
    bytes[18..26].copy_from_slice(&((packed & !0x0f_ffff_ffff) | 44100).to_be_bytes());
    std::fs::write(&input, &bytes).unwrap();
    let e = engine();
    assert_eq!(e.probe(&input, &cancel()).unwrap().duration, 1.0);
    let output_dir = dir.path().join("exports");
    std::fs::create_dir(&output_dir).unwrap();
    // Exhaust every candidate: content rejection must win over collisions.
    for number in 1..=10000 {
        let name = candidate(number);
        std::fs::write(output_dir.join(&name), name.as_bytes()).unwrap();
    }
    let before = entries(&output_dir);
    let progress = Mutex::new(Vec::new());
    let error = e
        .convert(
            &input,
            &output_dir,
            "target",
            0.0,
            Some(0.75),
            &cancel(),
            |p| progress.lock().unwrap().push(p),
        )
        .unwrap_err();
    assert!(
        matches!(&error, Error::Invalid(message) if message == "decoded audio did not fill the requested sample range"),
        "expected content validation rather than helper/collision failure: {error:?}"
    );
    assert!(!progress.lock().unwrap().contains(&1.0));
    assert_eq!(entries(&output_dir), before);
    assert_eq!(std::fs::read(&input).unwrap(), bytes);
    assert_candidates_preserved(&output_dir);
}

fn candidate(number: usize) -> String {
    if number == 1 {
        "target.wav".into()
    } else {
        format!("target ({number}).wav")
    }
}

fn assert_candidates_preserved(path: &Path) {
    for number in 1..=10000 {
        let name = candidate(number);
        assert_eq!(std::fs::read(path.join(&name)).unwrap(), name.as_bytes());
    }
}

#[test]
fn exhausted_collision_budget_cleans_staging_without_replacing_any_file() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    fixture(&input, "pcm_s16le");
    let original = std::fs::read(&input).unwrap();
    let output_dir = dir.path().join("exports");
    std::fs::create_dir(&output_dir).unwrap();
    for number in 1..=10000 {
        let name = candidate(number);
        std::fs::write(output_dir.join(&name), name.as_bytes()).unwrap();
    }
    let before = entries(&output_dir);
    let progress = Mutex::new(Vec::new());
    let error = engine()
        .convert(&input, &output_dir, "target", 0.0, None, &cancel(), |p| {
            progress.lock().unwrap().push(p);
        })
        .unwrap_err();
    assert!(
        matches!(&error, Error::Invalid(message) if message == "too many filename collisions"),
        "{error:?}"
    );
    assert!(!progress.lock().unwrap().contains(&1.0));
    assert_eq!(entries(&output_dir), before);
    assert_eq!(std::fs::read(input).unwrap(), original);
    assert_candidates_preserved(&output_dir);
}

#[test]
fn occupied_files_and_directories_are_skipped_before_validated_completion() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("target.wav");
    fixture(&input, "pcm_s16le");
    let original = std::fs::read(&input).unwrap();
    let directory = dir.path().join("target (2).wav");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("keep"), b"directory contents").unwrap();
    let occupied = dir.path().join("target (3).wav");
    std::fs::write(&occupied, b"not audio; never overwrite").unwrap();
    let expected = dir.path().join("target (4).wav");
    let progress = Mutex::new(Vec::new());
    let output = engine()
        .convert(
            &input,
            dir.path(),
            "target.wav",
            0.0,
            Some(1.0 / 44100.0),
            &cancel(),
            |p| {
                progress.lock().unwrap().push(p);
                if p == 1.0 {
                    // Completion must observe a published, fully validated file,
                    // including the RIFF pad required by a single 24-bit sample.
                    assert_eq!(validate_wav(&expected, &cancel()).unwrap().samples, 1);
                    let bytes = std::fs::read(&expected).unwrap();
                    assert_eq!(bytes.len(), 48);
                    assert_eq!(bytes[47], 0);
                    let decoded = Command::new(fixture_ffmpeg())
                        .args(["-nostdin", "-v", "error", "-xerror", "-i"])
                        .arg(&expected)
                        .args(["-c:a", "pcm_s24le", "-f", "s24le", "pipe:1"])
                        .output()
                        .unwrap();
                    assert!(decoded.status.success());
                    assert_eq!(decoded.stdout, bytes[44..47]);
                } else {
                    assert!(!expected.exists(), "published before completion");
                }
            },
        )
        .unwrap();
    assert_eq!(output, expected);
    assert_eq!(
        progress
            .lock()
            .unwrap()
            .iter()
            .filter(|&&p| p == 1.0)
            .count(),
        1
    );
    assert_eq!(std::fs::read(input).unwrap(), original);
    assert_eq!(
        std::fs::read(occupied).unwrap(),
        b"not audio; never overwrite"
    );
    assert_eq!(
        std::fs::read(directory.join("keep")).unwrap(),
        b"directory contents"
    );
    assert_eq!(entries(dir.path()).len(), 4, "staging must be removed");
}

#[test]
fn empty_real_decode_is_rejected_without_publication_or_completion() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.flac");
    fixture(&input, "flac");
    let mut bytes = std::fs::read(&input).unwrap();
    // Retain complete real metadata, corrupt only the encoded audio frames.
    let offset = flac_frames_start(&bytes);
    bytes[offset..].fill(0);
    std::fs::write(&input, &bytes).unwrap();
    let e = engine();
    assert_eq!(e.probe(&input, &cancel()).unwrap().duration, 0.5);
    let existing = dir.path().join("target.wav");
    std::fs::write(&existing, b"existing export").unwrap();
    let before = entries(dir.path());
    let progress = Mutex::new(Vec::new());
    let error = e
        .convert(&input, dir.path(), "target", 0.0, None, &cancel(), |p| {
            progress.lock().unwrap().push(p);
        })
        .unwrap_err();
    assert!(
        matches!(&error, Error::Invalid(message) if message == "empty or incomplete PCM output"),
        "expected content rejection after a successful empty decode: {error:?}"
    );
    assert!(!progress.lock().unwrap().contains(&1.0));
    assert_eq!(entries(dir.path()), before);
    assert_eq!(std::fs::read(input).unwrap(), bytes);
    assert_eq!(std::fs::read(existing).unwrap(), b"existing export");
}

fn flac_frames_start(bytes: &[u8]) -> usize {
    assert_eq!(&bytes[..4], b"fLaC");
    let mut offset = 4;
    loop {
        let last = bytes[offset] & 0x80 != 0;
        let length = (usize::from(bytes[offset + 1]) << 16)
            | (usize::from(bytes[offset + 2]) << 8)
            | usize::from(bytes[offset + 3]);
        offset += 4 + length;
        if last {
            assert!(offset < bytes.len());
            return offset;
        }
    }
}

#[test]
fn real_decoder_failure_after_probe_preserves_input_and_existing_export() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.flac");
    fixture(&input, "flac");
    let mut bytes = std::fs::read(&input).unwrap();
    let offset = flac_frames_start(&bytes);
    // Flip the FLAC subframe's mandatory zero padding bit. Metadata remains
    // readable, but the real decoder must fail, rather than fail at probing.
    bytes[offset + 6] ^= 0x80;
    std::fs::write(&input, &bytes).unwrap();
    let e = engine();
    assert_eq!(e.probe(&input, &cancel()).unwrap().duration, 0.5);
    let existing = dir.path().join("target.wav");
    std::fs::write(&existing, b"existing export").unwrap();
    let before = entries(dir.path());
    let progress = Mutex::new(Vec::new());
    let error = e
        .convert(&input, dir.path(), "target", 0.0, None, &cancel(), |p| {
            progress.lock().unwrap().push(p);
        })
        .unwrap_err();
    assert!(
        matches!(error, Error::Helper(_)),
        "expected actual decoder failure: {error:?}"
    );
    assert!(!progress.lock().unwrap().contains(&1.0));
    assert_eq!(entries(dir.path()), before);
    assert_eq!(std::fs::read(input).unwrap(), bytes);
    assert_eq!(std::fs::read(existing).unwrap(), b"existing export");
}

#[test]
fn cancellation_preserves_source_and_collisions_without_completion_or_staging() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("target.wav");
    fixture(&input, "pcm_s16le");
    let original = std::fs::read(&input).unwrap();
    let existing = dir.path().join("target (2).wav");
    std::fs::write(&existing, b"existing export").unwrap();
    let before = entries(dir.path());
    for already_cancelled in [true, false] {
        let c = cancel();
        c.store(already_cancelled, Ordering::Release);
        let progress = Mutex::new(Vec::new());
        let result = engine().convert(&input, dir.path(), "target", 0.0, None, &c, |p| {
            progress.lock().unwrap().push(p);
            assert!(p < 1.0, "cancelled export signalled completion");
            assert!(!dir.path().join("target (3).wav").exists());
            assert!(
                entries(dir.path())
                    .iter()
                    .any(|name| name.to_string_lossy().starts_with(".ditto-")),
                "active cancellation must exercise allocated staging"
            );
            c.store(true, Ordering::Release);
        });
        assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
        assert_eq!(progress.lock().unwrap().is_empty(), already_cancelled);
        assert_eq!(entries(dir.path()), before);
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert_eq!(std::fs::read(&existing).unwrap(), b"existing export");
    }
}
