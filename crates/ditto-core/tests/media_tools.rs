mod common;
use std::{path::PathBuf, process::Command};

// Select in isolated children rather than mutating this parallel test runner's environment.
#[test]
fn media_tool_selection_child() {
    let Some(expected) = std::env::var_os("DITTO_EXPECT_FIXTURE") else {
        return;
    };
    assert_eq!(common::fixture_ffmpeg(), PathBuf::from(expected));
    let engine = common::engine();
    assert_eq!(
        engine.ffmpeg,
        PathBuf::from(std::env::var_os("DITTO_EXPECT_FFMPEG").unwrap())
    );
    assert_eq!(
        engine.ffprobe,
        PathBuf::from(std::env::var_os("DITTO_EXPECT_FFPROBE").unwrap())
    );
}

fn selection(overrides: &[(&str, &str)], expected: [&str; 3]) {
    let mut child = Command::new(std::env::current_exe().unwrap());
    child.args(["--exact", "media_tool_selection_child"]);
    for key in [
        "DITTO_FIXTURE_FFMPEG",
        "DITTO_TEST_FFMPEG",
        "DITTO_FFMPEG",
        "DITTO_TEST_FFPROBE",
        "DITTO_FFPROBE",
    ] {
        child.env_remove(key);
    }
    child.envs(overrides.iter().copied());
    child.env("DITTO_EXPECT_FIXTURE", expected[0]);
    child.env("DITTO_EXPECT_FFMPEG", expected[1]);
    child.env("DITTO_EXPECT_FFPROBE", expected[2]);
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "selection failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fixture_override_is_independent_of_engine_overrides() {
    selection(
        &[
            ("DITTO_FIXTURE_FFMPEG", "fixture tools/full ffmpeg.exe"),
            ("DITTO_TEST_FFMPEG", "scoped tools/test ffmpeg.exe"),
            ("DITTO_FFMPEG", "fallback ffmpeg.exe"),
            ("DITTO_TEST_FFPROBE", "scoped tools/test ffprobe.exe"),
            ("DITTO_FFPROBE", "fallback ffprobe.exe"),
        ],
        [
            "fixture tools/full ffmpeg.exe",
            "scoped tools/test ffmpeg.exe",
            "scoped tools/test ffprobe.exe",
        ],
    );
}

#[test]
fn media_tools_default_to_path_commands() {
    selection(&[], ["ffmpeg", "ffmpeg", "ffprobe"]);
}

#[test]
fn engine_fallback_overrides_do_not_select_the_fixture_generator() {
    selection(
        &[
            ("DITTO_FFMPEG", "scoped ffmpeg"),
            ("DITTO_FFPROBE", "scoped ffprobe"),
        ],
        ["ffmpeg", "scoped ffmpeg", "scoped ffprobe"],
    );
}
