mod common;
use ditto_core::process::{run, ProcessOptions};
use std::{
    process::Command,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[test]
#[cfg(unix)]
fn normal_child_exit_does_not_leave_background_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("orphan");
    let mut command = Command::new("python3");
    command.args(["-c", "import subprocess,sys; subprocess.Popen([sys.executable,'-c',\"import pathlib,sys,time; time.sleep(.3); pathlib.Path(sys.argv[1]).write_text('bad')\",sys.argv[1]])"]).arg(&marker);
    let output = run(
        &mut command,
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions::default(),
        |_| {},
    )
    .unwrap();
    assert!(output.status.success());
    std::thread::sleep(Duration::from_millis(400));
    assert!(!marker.exists());
}

#[test]
#[cfg(unix)]
fn callback_panic_still_terminates_owned_children() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("panic-survivor");
    let mut command = Command::new("python3");
    command.args(["-c", "import pathlib,sys,time; print('ready',flush=True); time.sleep(.3); pathlib.Path(sys.argv[1]).write_text('bad')"]).arg(&marker);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(
            &mut command,
            &Arc::new(AtomicBool::new(false)),
            ProcessOptions::default(),
            |_| panic!("callback panic"),
        )
    }));
    assert!(result.is_err());
    std::thread::sleep(Duration::from_millis(400));
    assert!(!marker.exists());
}

#[test]
#[cfg(unix)]
fn cancel_interrupts_an_active_silent_helper() {
    let c = Arc::new(AtomicBool::new(false));
    let flag = c.clone();
    let trigger = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(70));
        flag.store(true, std::sync::atomic::Ordering::Release);
    });
    let mut command = Command::new("python3");
    command.args(["-c", "import time; time.sleep(3)"]);
    let start = std::time::Instant::now();
    let error = run(&mut command, &c, ProcessOptions::default(), |_| {}).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert!(start.elapsed() < Duration::from_secs(1));
    trigger.join().unwrap();
}

#[test]
#[cfg(unix)]
fn a_child_that_closes_pipes_still_obeys_the_deadline() {
    let mut command = Command::new("python3");
    command.args([
        "-c",
        "import os,time; os.close(1); os.close(2); time.sleep(3)",
    ]);
    let error = run(
        &mut command,
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions {
            timeout: Duration::from_millis(70),
            max_output: 1024,
        },
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
}

#[test]
#[cfg(unix)]
fn nonzero_exit_is_returned_without_inventing_success() {
    let mut command = Command::new("python3");
    command.args([
        "-c",
        "import os; os.write(2,b'failure'); raise SystemExit(17)",
    ]);
    let output = run(
        &mut command,
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions::default(),
        |_| {},
    )
    .unwrap();
    assert_eq!(output.status.code(), Some(17));
    assert_eq!(output.stderr, b"failure");
}

#[test]
#[cfg(unix)]
fn stdout_callback_receives_all_bytes_even_across_chunks() {
    let mut command = Command::new("python3");
    command.args([
        "-c",
        "import os; os.write(1,b'x'*50000); os.write(2,b'y'*50000)",
    ]);
    let mut chunks = Vec::new();
    let output = run(
        &mut command,
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions {
            max_output: 100000,
            ..ProcessOptions::default()
        },
        |bytes| chunks.extend_from_slice(bytes),
    )
    .unwrap();
    assert_eq!(output.stdout.len(), 50000);
    assert_eq!(output.stderr.len(), 50000);
    assert_eq!(chunks, output.stdout);
}

#[test]
#[cfg(unix)]
fn timeout_terminates_descendants_and_reaps_child() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("escaped");
    let mut command = Command::new("python3");
    command.args(["-c", "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',\"import pathlib,sys,time; time.sleep(.4); pathlib.Path(sys.argv[1]).write_text('bad')\",sys.argv[1]]); time.sleep(.7)"]).arg(&marker);
    let started = std::time::Instant::now();
    let error = run(
        &mut command,
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions {
            timeout: Duration::from_millis(100),
            max_output: 4096,
        },
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_millis(400));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !marker.exists(),
        "descendant survived owned process tree termination"
    );
}

#[test]
fn cancellation_prevents_spawn() {
    let mut command = Command::new(common::engine().ffmpeg);
    command.arg("-version");
    let error = run(
        &mut command,
        &Arc::new(AtomicBool::new(true)),
        ProcessOptions::default(),
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
}

#[test]
#[cfg(unix)]
fn caps_total_output_without_pipe_deadlock() {
    let mut command = Command::new("python3");
    command.args([
        "-c",
        "import os; os.write(1,b'x'*200000); os.write(2,b'y'*200000)",
    ]);
    let result = run(
        &mut command,
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions {
            timeout: Duration::from_secs(5),
            max_output: 1024,
        },
        |_| {},
    );
    assert!(result.is_err(), "unbounded child output must fail");
}

#[test]
fn executes_direct_arguments_and_captures_both_pipes() {
    let mut command = Command::new(common::engine().ffmpeg);
    command.arg("-version");
    let result = run(
        &mut command,
        &Arc::new(AtomicBool::new(false)),
        ProcessOptions {
            timeout: Duration::from_secs(5),
            max_output: 1024 * 1024,
        },
        |_| {},
    )
    .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("ffmpeg version"));
}
