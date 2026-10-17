#![cfg(unix)]
use ditto_online::OnlineEngine;
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
fn engine(t: &tempfile::TempDir, script: &str) -> OnlineEngine {
    let p = t.path().join("helper");
    std::fs::write(&p, script).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
    OnlineEngine {
        yt_dlp: p,
        deno: PathBuf::from("/managed/deno"),
        work_dir: t.path().join("work"),
        ffmpeg_dir: PathBuf::from("/managed/audio"),
    }
}
#[test]
fn acquisition_requires_final_event_exit_and_contained_file_and_reports_real_progress() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(
        &t,
        r##"#!/bin/sh
printf '%s\n' "$@" > args.txt
printf 'test media' > source.webm
printf '%s\n' 'DITTO_PROGRESS {"downloaded_bytes":5,"total_bytes":10}'
printf 'DITTO_FILE "%s/source.webm"\n' "$PWD"
"##,
    );
    let c = Arc::new(AtomicBool::new(false));
    let mut progress = Vec::new();
    let path = e
        .acquire(
            "https://youtu.be/BaW_jenozKc",
            &e.work_dir.join("job"),
            &c,
            |p| progress.push(p),
        )
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"test media");
    assert!(progress.contains(&0.5));
    let args = std::fs::read_to_string(path.parent().unwrap().join("args.txt")).unwrap();
    for flag in [
        "--max-filesize",
        "--no-playlist",
        "--no-simulate",
        "--no-quiet",
        "--no-overwrites",
        "--downloader",
        "m3u8:native",
        "--concurrent-fragments",
    ] {
        assert!(args.lines().any(|a| a == flag));
    }
    assert!(e
        .acquire(
            "https://youtu.be/BaW_jenozKc",
            &e.work_dir.join("job"),
            &c,
            |_| {}
        )
        .is_err());
}
#[test]
fn explicit_preview_acquisition_reuses_secure_bounded_download_path() {
    let t = tempfile::tempdir().unwrap();
    let e=engine(&t,"#!/bin/sh\nprintf preview > source.webm\nprintf 'DITTO_FILE \"%s/source.webm\"\\n' \"$PWD\"\n");
    let path = e
        .acquire_preview(
            "https://youtu.be/BaW_jenozKc",
            &e.work_dir.join("preview"),
            &Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
    assert!(path.is_file());
}
#[test]
fn failed_or_cancelled_acquisition_cleans_only_its_own_job() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(&t, "#!/bin/sh\nprintf partial > source.part\nsleep 30\n");
    std::fs::create_dir_all(&e.work_dir).unwrap();
    std::fs::write(e.work_dir.join("keep"), b"original").unwrap();
    let c = Arc::new(AtomicBool::new(false));
    let other = c.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        other.store(true, Ordering::SeqCst);
    });
    assert!(e
        .acquire(
            "https://youtu.be/BaW_jenozKc",
            &e.work_dir.join("job"),
            &c,
            |_| {}
        )
        .is_err());
    stopper.join().unwrap();
    assert!(!e.work_dir.join("job").exists());
    assert!(e.work_dir.join("keep").is_file());
}
#[test]
fn acquisition_never_admits_a_playlist_as_a_single_download_job() {
    let t = tempfile::tempdir().unwrap();
    let e=engine(&t,"#!/bin/sh\nprintf media > source.webm\nprintf 'DITTO_FILE \"%s/source.webm\"\\n' \"$PWD\"\n");
    let c = Arc::new(AtomicBool::new(false));
    for (n, url) in [
        "https://www.youtube.com/playlist?list=abc",
        "https://soundcloud.com/artist/sets/collection",
    ]
    .iter()
    .enumerate()
    {
        let dir = e.work_dir.join(format!("job-{n}"));
        assert!(e.acquire(url, &dir, &c, |_| {}).is_err());
        assert!(!dir.exists());
    }
}
#[test]
fn oversized_partial_download_is_stopped_early_without_cancelling_other_jobs() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(
        &t,
        "#!/bin/sh\nset -e\npython3 -c 'f = open(\"source.part\", \"wb\"); f.truncate(268435457); f.close()'\nsleep 3\n",
    );
    let c = Arc::new(AtomicBool::new(false));
    let started = std::time::Instant::now();
    assert!(e
        .acquire(
            "https://youtu.be/BaW_jenozKc",
            &e.work_dir.join("job"),
            &c,
            |_| {}
        )
        .is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert!(!c.load(Ordering::SeqCst));
    assert!(!e.work_dir.join("job").exists());
}
#[test]
fn escaped_final_path_and_nonzero_exit_are_never_success() {
    for script in [
        "#!/bin/sh\nprintf 'DITTO_FILE \"/etc/passwd\"\\n'\n",
        "#!/bin/sh\nprintf media > source.webm\nprintf 'DITTO_FILE \"source.webm\"\\n'\nexit 1\n",
    ] {
        let t = tempfile::tempdir().unwrap();
        let e = engine(&t, script);
        let c = Arc::new(AtomicBool::new(false));
        assert!(e
            .acquire(
                "https://youtu.be/BaW_jenozKc",
                &e.work_dir.join("job"),
                &c,
                |_| {}
            )
            .is_err());
        assert!(!e.work_dir.join("job").exists());
    }
}
