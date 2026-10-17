#![allow(dead_code)]
use ditto_core::Engine;
use std::path::PathBuf;

pub fn engine() -> Engine {
    Engine {
        ffmpeg: std::env::var_os("DITTO_TEST_FFMPEG")
            .or_else(|| std::env::var_os("DITTO_FFMPEG"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("ffmpeg")),
        ffprobe: std::env::var_os("DITTO_TEST_FFPROBE")
            .or_else(|| std::env::var_os("DITTO_FFPROBE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("ffprobe")),
    }
}

pub fn fixture_ffmpeg() -> PathBuf {
    std::env::var_os("DITTO_FIXTURE_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}
