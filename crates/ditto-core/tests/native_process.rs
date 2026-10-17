//! Real native child/process-tree fixtures: no shell or Python prerequisite.
//! On Windows these exercise suspended startup/job ownership, not just flag constants.
use ditto_core::process::{run, ProcessOptions};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn child(mode: &str, root: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "native_process_fixture", "--nocapture"])
        .env("DITTO_NATIVE_PROCESS_MODE", mode)
        .env("DITTO_NATIVE_PROCESS_ROOT", root);
    command
}

#[test]
fn native_process_fixture() {
    let Some(mode) = std::env::var_os("DITTO_NATIVE_PROCESS_MODE") else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os("DITTO_NATIVE_PROCESS_ROOT").unwrap());
    if mode == "descendant" {
        std::fs::write(root.join("ready"), b"started").unwrap();
        std::thread::sleep(Duration::from_millis(800));
        std::fs::write(root.join("survived"), b"unowned descendant").unwrap();
        std::process::exit(0);
    }
    if mode == "silent" {
        std::fs::write(root.join("ready"), b"started").unwrap();
        std::thread::sleep(Duration::from_secs(10));
        std::process::exit(0);
    }
    let mut descendant = child("descendant", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !root.join("ready").exists() {
        if started.elapsed() > Duration::from_secs(3) {
            let _ = descendant.kill();
            let _ = descendant.wait();
            panic!("descendant failed to start");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    use std::io::Write;
    println!("owned-ready");
    std::io::stdout().flush().unwrap();
    if mode == "parent-exit" {
        std::process::exit(0);
    }
    assert_eq!(mode, "parent-wait");
    std::thread::sleep(Duration::from_secs(10));
    std::process::exit(0);
}

fn assert_descendant_was_terminated(root: &Path) {
    assert!(
        root.join("ready").is_file(),
        "descendant must actually have started"
    );
    std::thread::sleep(Duration::from_millis(1000));
    assert!(
        !root.join("survived").exists(),
        "owned descendant survived termination"
    );
}

#[test]
fn native_normal_exit_terminates_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        &mut child("parent-exit", dir.path()),
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions {
            timeout: Duration::from_secs(5),
            ..ProcessOptions::default()
        },
        |_| {},
    )
    .unwrap();
    assert!(output.status.success());
    assert_descendant_was_terminated(dir.path());
}

#[test]
fn native_callback_cancellation_terminates_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut stdout = Vec::new();
    let error = run(
        &mut child("parent-wait", dir.path()),
        &cancel,
        ProcessOptions {
            timeout: Duration::from_secs(5),
            ..ProcessOptions::default()
        },
        |bytes| {
            stdout.extend_from_slice(bytes);
            if stdout
                .windows(b"owned-ready".len())
                .any(|w| w == b"owned-ready")
            {
                cancel.store(true, Ordering::Release);
            }
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_descendant_was_terminated(dir.path());
}

#[test]
fn native_callback_panic_terminates_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let mut stdout = Vec::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(
            &mut child("parent-wait", dir.path()),
            &Arc::new(AtomicBool::new(false)),
            ProcessOptions {
                timeout: Duration::from_secs(5),
                ..ProcessOptions::default()
            },
            |bytes| {
                stdout.extend_from_slice(bytes);
                assert!(
                    !stdout
                        .windows(b"owned-ready".len())
                        .any(|w| w == b"owned-ready"),
                    "intentional callback panic"
                );
            },
        )
    }));
    assert!(result.is_err());
    assert_descendant_was_terminated(dir.path());
}

#[test]
fn native_silent_child_obeys_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let error = run(
        &mut child("silent", dir.path()),
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions {
            timeout: Duration::from_millis(500),
            ..ProcessOptions::default()
        },
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(
        dir.path().join("ready").is_file(),
        "native child never started"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}
