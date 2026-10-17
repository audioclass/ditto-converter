mod common;
use common::{engine, fixture_ffmpeg};
use ditto_core::Cancellation;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::AtomicBool, Arc},
};
fn cancel() -> Cancellation {
    Arc::new(AtomicBool::new(false))
}

fn fixture(path: &Path, codec: &str, rate: &str, channels: &str) {
    let status = Command::new(fixture_ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.5",
            "-ar",
            rate,
            "-ac",
            channels,
            "-c:a",
            codec,
            "-y",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}
fn ramp(path: &Path) {
    let samples: Vec<i16> = (0..4410).map(|n| (n % 1000) as i16 - 500).collect();
    let data_size = samples.len() as u32 * 2;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + data_size).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(44100u32.to_le_bytes());
    bytes.extend(88200u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_size.to_le_bytes());
    for sample in samples {
        bytes.extend(sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}
#[test]
fn half_sample_boundaries_round_up_and_end_remains_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    ramp(&input);
    let output = engine()
        .convert(
            &input,
            dir.path(),
            "one sample",
            0.5 / 44100.0,
            Some(1.5 / 44100.0),
            &cancel(),
            |_| {},
        )
        .unwrap();
    let info = ditto_core::audio::validate_wav(&output, &cancel()).unwrap();
    assert_eq!(info.samples, 1);
    let bytes = std::fs::read(output).unwrap();
    assert_eq!(&bytes[44..47], &((-499i32 * 256).to_le_bytes()[..3]));
}

#[test]
#[cfg(unix)]
fn destination_symlink_is_a_collision_never_a_write_target() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    ramp(&input);
    let original = std::fs::read(&input).unwrap();
    let link = dir.path().join("target.wav");
    std::os::unix::fs::symlink(&input, &link).unwrap();
    let output = engine()
        .convert(&input, dir.path(), "target", 0.0, None, &cancel(), |_| {})
        .unwrap();
    assert_ne!(output, link);
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(input).unwrap(), original);
}

#[test]
fn malformed_pcm_and_invalid_destination_leave_no_staged_exports() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    ramp(&input);
    let mut bytes = std::fs::read(&input).unwrap();
    bytes.truncate(bytes.len() - 1);
    std::fs::write(&input, bytes).unwrap();
    assert!(engine()
        .convert(
            &input,
            dir.path(),
            "truncated",
            0.0,
            None,
            &cancel(),
            |_| {}
        )
        .is_err());
    ramp(&input);
    assert!(engine()
        .convert(
            &input,
            &input,
            "not a directory",
            0.0,
            None,
            &cancel(),
            |_| {}
        )
        .is_err());
    assert!(engine().preview(&input, &input, &cancel()).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn vbr_mp3_and_multi_stream_containers_select_first_audio_and_keep_decoded_duration() {
    let dir = tempfile::tempdir().unwrap();
    let vbr = dir.path().join("vbr.mp3");
    assert!(Command::new(fixture_ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=777:duration=0.5:sample_rate=44100",
            "-c:a",
            "libmp3lame",
            "-q:a",
            "4"
        ])
        .arg(&vbr)
        .status()
        .unwrap()
        .success());
    let multistream = dir.path().join("two-audio.mkv");
    assert!(Command::new(fixture_ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=44100:cl=mono:d=0.5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=0.5:sample_rate=48000",
            "-map",
            "0:a",
            "-map",
            "1:a",
            "-c:a",
            "pcm_s16le"
        ])
        .arg(&multistream)
        .status()
        .unwrap()
        .success());
    for (input, expected_peak) in [(&vbr, true), (&multistream, false)] {
        let output = engine()
            .convert(
                input,
                dir.path(),
                "first",
                0.1,
                Some(0.3),
                &cancel(),
                |_| {},
            )
            .unwrap();
        let info = ditto_core::audio::validate_wav(&output, &cancel()).unwrap();
        assert_eq!(info.samples, 8820);
        assert_eq!(info.peak > 0, expected_peak);
        let preview = engine().preview(input, dir.path(), &cancel()).unwrap();
        assert!((engine().probe(&preview, &cancel()).unwrap().duration - 0.5).abs() < 0.0001);
    }
}

#[test]
fn rejects_missing_corrupt_video_only_and_indirect_inputs_without_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let corrupt = dir.path().join("corrupt.wav");
    std::fs::write(&corrupt, b"RIFFbroken").unwrap();
    let indirect = dir.path().join("playlist.ffconcat");
    std::fs::write(
        &indirect,
        b"ffconcat version 1.0\nfile 'http://127.0.0.1:9/unwanted'\n",
    )
    .unwrap();
    let video = dir.path().join("video.mkv");
    assert!(Command::new(fixture_ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=16x16:d=0.1",
            "-c:v",
            "ffv1"
        ])
        .arg(&video)
        .status()
        .unwrap()
        .success());
    let e = engine();
    for input in [
        corrupt,
        indirect,
        video,
        dir.path().join("absent.wav"),
        dir.path().to_path_buf(),
    ] {
        assert!(
            e.probe(&input, &cancel()).is_err(),
            "accepted {}",
            input.display()
        );
        assert!(e
            .convert(
                &input,
                dir.path(),
                "must-not-appear",
                0.0,
                None,
                &cancel(),
                |_| {}
            )
            .is_err());
        assert!(e.preview(&input, dir.path(), &cancel()).is_err());
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
}

#[test]
fn nonzero_input_timestamps_do_not_shift_the_sample_trim_timebase() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("offset.mka");
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
            "pcm_s16le",
            "-output_ts_offset",
            "5"
        ])
        .arg(&input)
        .status()
        .unwrap()
        .success());
    let output = engine()
        .convert(
            &input,
            dir.path(),
            "offset",
            0.1,
            Some(0.3),
            &cancel(),
            |_| {},
        )
        .unwrap();
    let info = ditto_core::audio::validate_wav(&output, &cancel()).unwrap();
    assert_eq!(info.samples, 8820);
    assert!(info.peak > 0);
    let preview = engine().preview(&input, dir.path(), &cancel()).unwrap();
    assert!((engine().probe(&preview, &cancel()).unwrap().duration - 0.5).abs() < 0.0001);
}

#[test]
fn inherited_ffreport_cannot_create_unrequested_files() {
    if let Some(dir) = std::env::var_os("DITTO_REPORT_CHILD") {
        let dir = PathBuf::from(dir);
        let input = dir.join("source.wav");
        ramp(&input);
        engine()
            .convert(&input, &dir, "normal", 0.0, None, &cancel(), |_| {})
            .unwrap();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("unrequested-report.log");
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "inherited_ffreport_cannot_create_unrequested_files",
        ])
        .env("DITTO_REPORT_CHILD", dir.path())
        .env("FFREPORT", format!("file={}", report.display()))
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        !report.exists(),
        "inherited FFREPORT created an uncontrolled helper report"
    );
}

#[test]
#[cfg(unix)]
fn rejects_oversize_decode_before_starting_audio_helper() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    ramp(&input);
    let probe = dir.path().join("large-probe");
    std::fs::write(&probe, "#!/usr/bin/env python3\nprint('{\"streams\":[{\"sample_rate\":\"44100\",\"channels\":1,\"duration\":\"100000\"}]}')\n").unwrap();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut e = engine();
    e.ffprobe = probe;
    e.ffmpeg = dir.path().join("must-not-be-spawned");
    let error = e
        .convert(
            &input,
            dir.path(),
            "too large",
            0.0,
            None,
            &cancel(),
            |_| {},
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("size limit"),
        "wrong failure: {error}"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    let error = e.preview(&input, dir.path(), &cancel()).unwrap_err();
    assert!(
        error.to_string().contains("size limit"),
        "wrong failure: {error}"
    );
}

#[test]
fn stereo_downmix_preserves_equal_channels_and_cancels_opposite_phase() {
    let dir = tempfile::tempdir().unwrap();
    for (name, expression, expected) in [
        ("equal", "aevalsrc=0.75|0.75:s=44100:d=0.1", 6291456u32),
        ("opposite", "aevalsrc=0.75|-0.75:s=44100:d=0.1", 0),
    ] {
        let input = dir.path().join(format!("{name}.wav"));
        let status = Command::new(fixture_ffmpeg())
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                expression,
                "-c:a",
                "pcm_f32le",
            ])
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success());
        let output = engine()
            .convert(&input, dir.path(), name, 0.0, None, &cancel(), |_| {})
            .unwrap();
        let info = ditto_core::audio::validate_wav(&output, &cancel()).unwrap();
        assert!(
            info.peak.abs_diff(expected) <= 1,
            "{name}: peak {} != {expected}",
            info.peak
        );
    }
}

#[test]
fn real_codec_corpus_probes_previews_trims_and_decodes_valid_exports() {
    let cases = [
        ("u8.wav", "pcm_u8", "22050", "1"),
        ("s16.wav", "pcm_s16le", "44100", "2"),
        ("s24.wav", "pcm_s24le", "48000", "1"),
        ("s32.wav", "pcm_s32le", "96000", "2"),
        ("float.wav", "pcm_f32le", "48000", "2"),
        ("double.wav", "pcm_f64le", "44100", "1"),
        ("track.mp3", "libmp3lame", "44100", "2"),
        ("track.m4a", "aac", "44100", "2"),
        ("track.aac", "aac", "48000", "1"),
        ("track.flac", "flac", "96000", "2"),
        ("vorbis.ogg", "libvorbis", "44100", "2"),
        ("opus.ogg", "libopus", "48000", "2"),
        ("track.webm", "libopus", "48000", "2"),
        ("surround.wav", "pcm_s24le", "48000", "6"),
        ("track.aiff", "pcm_s16be", "44100", "1"),
    ];
    let dir = tempfile::tempdir().unwrap();
    let e = engine();
    for (name, codec, rate, channels) in cases {
        let input = dir.path().join(name);
        fixture(&input, codec, rate, channels);
        let original = std::fs::read(&input).unwrap();
        let info = e
            .probe(&input, &cancel())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(
            info.duration >= 0.45 && info.duration < 0.7,
            "{name}: {}",
            info.duration
        );
        let values = std::sync::Mutex::new(Vec::new());
        let output = e
            .convert(
                &input,
                dir.path(),
                &format!("export {name}"),
                0.1,
                Some(0.3),
                &cancel(),
                |p| values.lock().unwrap().push(p),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let valid = ditto_core::audio::validate_wav(&output, &cancel()).unwrap();
        assert!(
            valid.samples.abs_diff(8820) <= 1,
            "{name}: {} samples",
            valid.samples
        );
        assert!(valid.peak > 0, "{name}: unexpected silence");
        let decoded = Command::new(fixture_ffmpeg())
            .args(["-nostdin", "-v", "error", "-xerror", "-i"])
            .arg(&output)
            .args(["-f", "s24le", "pipe:1"])
            .output()
            .unwrap();
        assert!(decoded.status.success(), "{name}");
        assert_eq!(decoded.stdout.len() as u64, valid.samples * 3);
        let preview = e
            .preview(&input, dir.path(), &cancel())
            .unwrap_or_else(|error| panic!("preview {name}: {error}"));
        let preview_info = e.probe(&preview, &cancel()).unwrap();
        assert_eq!(preview_info.sample_rate, info.sample_rate);
        assert_eq!(preview_info.channels, info.channels.min(2));
        assert_eq!(std::fs::read(&input).unwrap(), original);
        let values = values.lock().unwrap();
        assert_eq!(values.last(), Some(&1.0));
        assert!(values.windows(2).all(|w| w[0] <= w[1]));
        assert!(values
            .iter()
            .all(|p| p.is_finite() && (0.0..=1.0).contains(p)));
        println!(
            "CORPUS {name}: {codec} {rate}Hz {channels}ch -> {} samples, preview {:.6}s",
            valid.samples, preview_info.duration
        );
    }
}

#[test]
#[cfg(unix)]
fn successful_helper_exit_is_not_enough_to_publish_short_output() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    ramp(&input);
    let helper = dir.path().join("short-helper");
    std::fs::write(&helper, "#!/usr/bin/env python3\nimport pathlib,sys\npathlib.Path(sys.argv[-1]).write_bytes(bytes(3))\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut e = engine();
    e.ffmpeg = helper;
    assert!(e
        .convert(
            &input,
            dir.path(),
            "bad",
            0.01,
            Some(0.02),
            &cancel(),
            |_| {}
        )
        .is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn real_progress_cancellation_cleans_staging_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    fixture(&input, "pcm_s16le", "48000", "2");
    let c = cancel();
    let values = std::sync::Mutex::new(Vec::new());
    let result = engine().convert(&input, dir.path(), "cancelled", 0.0, None, &c, |progress| {
        values.lock().unwrap().push(progress);
        if progress > 0.0 && progress < 1.0 {
            c.store(true, std::sync::atomic::Ordering::Release);
        }
    });
    assert!(
        matches!(result, Err(ditto_core::Error::Cancelled)),
        "cancel from real helper progress must return typed cancellation: {result:?}"
    );
    assert!(values.lock().unwrap().iter().any(|&p| p > 0.0 && p < 1.0));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn preview_is_decoded_pcm_with_the_export_sample_timebase() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    ramp(&input);
    let preview = engine().preview(&input, dir.path(), &cancel()).unwrap();
    assert_ne!(preview, input);
    assert!(preview.starts_with(dir.path()));
    let decoded = Command::new(fixture_ffmpeg())
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(&preview)
        .args(["-f", "s16le", "-c:a", "pcm_s16le", "pipe:1"])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let source = std::fs::read(&input).unwrap();
    assert_eq!(decoded.stdout, source[44..]);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    std::fs::remove_file(preview).unwrap();
}

#[test]
fn validator_decodes_samples_and_rejects_corrupt_or_truncated_bytes() {
    use ditto_core::audio::validate_wav;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.wav");
    ramp(&input);
    let output = engine()
        .convert(
            &input,
            dir.path(),
            "valid",
            0.01,
            Some(0.02),
            &cancel(),
            |_| {},
        )
        .unwrap();
    let info = validate_wav(&output, &cancel()).unwrap();
    assert_eq!(info.samples, 441);
    assert_eq!(info.peak, 381 * 256);
    let original = std::fs::read(&output).unwrap();
    for index in [0, 4, 8, 16, 20, 22, 24, 28, 32, 34, 36, 40] {
        let mut bytes = original.clone();
        bytes[index] ^= 0xff;
        std::fs::write(&output, bytes).unwrap();
        assert!(
            validate_wav(&output, &cancel()).is_err(),
            "accepted corrupt byte {index}"
        );
    }
    std::fs::write(&output, &original[..original.len() - 1]).unwrap();
    assert!(validate_wav(&output, &cancel()).is_err());
    std::fs::write(&output, original).unwrap();
    let c = cancel();
    c.store(true, std::sync::atomic::Ordering::Release);
    assert!(validate_wav(&output, &c).is_err());
}

#[test]
fn filenames_are_bounded_single_components_and_avoid_reserved_names() {
    use ditto_core::audio::safe_stem;
    for title in [
        "../../escape",
        "C:\\bad\\name",
        "\0\n\r",
        "CON",
        "prn.wav",
        "COM1",
        "LPT9",
        "b",
        "BT",
        "LOOP",
        " . ",
        "音楽",
        "a|b:*?\"<>",
    ] {
        let stem = safe_stem(title);
        assert!(!stem.is_empty());
        assert!(stem.len() <= 80);
        assert!(
            stem.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b" _-".contains(&b)),
            "{title:?} -> {stem}"
        );
        assert!(!["CON", "PRN", "COM1", "LPT9", "B", "BT", "LOOP"]
            .contains(&stem.to_uppercase().as_str()));
    }
    assert_eq!(safe_stem("A nice track.wav"), "A nice track");
    assert_eq!(safe_stem(&"a".repeat(300)).len(), 80);
}

#[test]
fn concurrent_exports_never_clobber_source_or_existing_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("same.wav");
    ramp(&input);
    let original = std::fs::read(&input).unwrap();
    let paths = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    engine()
                        .convert(&input, dir.path(), "same", 0.0, None, &cancel(), |_| {})
                        .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    let unique: std::collections::HashSet<_> = paths.iter().collect();
    assert_eq!(unique.len(), 4);
    assert!(!paths.contains(&input));
    assert_eq!(std::fs::read(input).unwrap(), original);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 5);
}

#[test]
fn rejects_invalid_or_subsample_trim_without_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.wav");
    ramp(&input);
    let original = std::fs::read(&input).unwrap();
    for (start, end) in [
        (-0.1, None),
        (f64::NAN, None),
        (f64::INFINITY, None),
        (0.02, Some(0.01)),
        (0.01, Some(0.01)),
        (0.0, Some(f64::NAN)),
        (0.0, Some(f64::INFINITY)),
        (0.0, Some(0.2)),
        (0.2, None),
        (0.0, Some(0.000001)),
    ] {
        assert!(
            engine()
                .convert(&input, dir.path(), "invalid", start, end, &cancel(), |_| {})
                .is_err(),
            "accepted {start:?}..{end:?}"
        );
    }
    assert_eq!(std::fs::read(&input).unwrap(), original);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn exports_exact_sample_trim_in_classic_pcm24_wav() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.wav");
    ramp(&input);
    let output = engine()
        .convert(
            &input,
            dir.path(),
            "Trim",
            0.01,
            Some(0.02),
            &cancel(),
            |_| {},
        )
        .unwrap();
    let bytes = std::fs::read(output).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 16);
    assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 1);
    assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 44100);
    assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 24);
    assert_eq!(&bytes[36..40], b"data");
    assert_eq!(
        u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
        441 * 3
    );
    assert_eq!(&bytes[44..47], &((-59i32 * 256).to_le_bytes()[..3]));
    assert_eq!(
        bytes.len(),
        44 + 441 * 3 + 1,
        "odd data size must have RIFF padding"
    );
}

#[test]
fn probe_inspects_audio_content_instead_of_extension() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("音 source.wav");
    fixture(&source, "pcm_s16le", "48000", "2");
    let disguised = dir.path().join("not-audio.bin");
    std::fs::rename(source, &disguised).unwrap();
    let info = engine().probe(&disguised, &cancel()).unwrap();
    assert_eq!(info.sample_rate, 48000);
    assert_eq!(info.channels, 2);
    assert!((info.duration - 0.5).abs() < 0.0001);
}
