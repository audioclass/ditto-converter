#![cfg(unix)]
use ditto_online::OnlineEngine;
use std::os::unix::fs::PermissionsExt;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

fn fixture(script: &str) -> (tempfile::TempDir, OnlineEngine) {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("helper");
    std::fs::write(&helper, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let engine = OnlineEngine {
        yt_dlp: helper,
        deno: PathBuf::from("/managed/deno"),
        work_dir: dir.path().join("work"),
        ffmpeg_dir: PathBuf::from("/managed/audio"),
    };
    (dir, engine)
}

#[test]
fn fragmented_utf8_rows_survive_later_malformed_output_and_abort_promptly() {
    let (_dir, engine) = fixture(
        r#"
printf '%s' '{"title":"Caf'
printf '\303'
sleep 0.02
printf '\251'
printf '%s\n' '","url":"aqz-KE-bpKQ"}'
printf '%s\n' 'broken json'
sleep 1
"#,
    );
    let mut rows = Vec::new();
    let start = std::time::Instant::now();
    let result = engine.enumerate_stream(
        "https://youtu.be/aqz-KE-bpKQ",
        &Arc::new(AtomicBool::new(false)),
        |row| rows.push(row),
    );
    assert!(result.is_err());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Café");
    assert!(
        start.elapsed() < std::time::Duration::from_millis(500),
        "parser failure must terminate helper promptly"
    );
}

#[test]
fn cancellation_in_callback_never_emits_or_replays_remaining_rows() {
    let (_dir, engine) = fixture(
        r#"
printf '%s\n' '{"title":"First","url":"aqz-KE-bpKQ"}' '{"title":"Second","url":"aqz-KE-bpKQ"}'
sleep 1
"#,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let mut rows = Vec::new();
    let result = engine.enumerate_stream("https://youtu.be/aqz-KE-bpKQ", &cancel, |row| {
        rows.push(row);
        cancel.store(true, Ordering::SeqCst);
    });
    assert!(result.is_err());
    assert_eq!(rows.len(), 1);
}

#[test]
fn cancellation_during_silent_helper_is_prompt() {
    let (_dir, engine) = fixture("sleep 1");
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        worker_cancel.store(true, Ordering::SeqCst);
    });
    let start = std::time::Instant::now();
    assert!(engine
        .enumerate_stream("https://youtu.be/aqz-KE-bpKQ", &cancel, |_| panic!(
            "no rows expected"
        ))
        .is_err());
    worker.join().unwrap();
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
}

#[test]
fn row_and_count_limits_keep_emitted_rows_without_poisoning_caller_cancel() {
    for script in [
        "i=0; while [ \"$i\" -lt 501 ]; do printf '%s\\n' '{\"title\":\"Row\",\"url\":\"aqz-KE-bpKQ\"}'; i=$((i+1)); done; sleep 1",
        "printf '%s\\n' '{\"title\":\"Row\",\"url\":\"aqz-KE-bpKQ\"}'; dd if=/dev/zero bs=1048577 count=1 2>/dev/null; sleep 1",
    ] {
        let (_dir, engine) = fixture(script);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut count = 0;
        let error = engine.enumerate_stream("https://youtu.be/aqz-KE-bpKQ", &cancel, |row| {
            assert_eq!(row.position, count);
            count += 1;
        }).unwrap_err();
        assert!(error.to_string().contains("metadata limit"), "{error}");
        assert!(count == 1 || count == 500);
        assert!(!cancel.load(Ordering::SeqCst));
    }
}

#[test]
fn streams_once_before_exit_preserving_unavailable_siblings_and_metadata_only_argv() {
    let (_dir, engine) = fixture(
        r#"
printf '%s\n' "$@" > args.txt
printf '%s\n' '{"title":"First","url":"aqz-KE-bpKQ"}'
i=0
while [ ! -f callback-seen ]; do
    i=$((i+1)); [ "$i" -lt 100 ] || exit 4
    sleep 0.01
done
printf '%s\n' '{"title":"[Deleted video]","url":"xxxxxxxxxxx"}'
printf '%s' '{"title":"Last","url":"aqz-KE-bpKQ"}'
exit 1
"#,
    );
    let mut rows = Vec::new();
    engine
        .enumerate_stream(
            "https://www.youtube.com/playlist?list=test",
            &Arc::new(AtomicBool::new(false)),
            |row| {
                std::fs::write(engine.work_dir.join("callback-seen"), "seen").unwrap();
                rows.push(row);
            },
        )
        .unwrap();
    assert_eq!(
        rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["First", "[Deleted video]", "Last"]
    );
    assert_eq!(
        rows.iter().map(|r| r.position).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(!rows[1].available);
    let args = std::fs::read_to_string(engine.work_dir.join("args.txt")).unwrap();
    for flag in [
        "--simulate",
        "--skip-download",
        "--flat-playlist",
        "--lazy-playlist",
        "--dump-json",
        "--ignore-errors",
    ] {
        assert!(args.lines().any(|a| a == flag), "{flag}");
    }
    assert!(!args.contains("--cookies"));
}
