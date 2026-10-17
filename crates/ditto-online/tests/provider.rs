use ditto_online::OnlineEngine;
#[cfg(unix)]
use ditto_online::OnlineEntry;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};
#[test]
fn unavailable_availability_flag_is_not_admitted() {
    let rows = ditto_online::parse_metadata(
        r#"{"title":"Unavailable sibling","url":"aqz-KE-bpKQ","availability":"unavailable"}"#,
        "youtube",
    )
    .unwrap();
    assert!(!rows[0].available);
}

#[test]
fn parse_metadata_preserves_positions_unavailable_and_rejects_extracted_urls() {
    let rows = ditto_online::parse_metadata(concat!(
 r#"{"title":"First","id":"BaW_jenozKc","url":"https://www.youtube.com/watch?v=BaW_jenozKc","duration":12.5}"#, "\n",
 r#"{"title":"[Deleted video]","id":"xxxxxxxxxxx","url":"https://www.youtube.com/watch?v=xxxxxxxxxxx","availability":"private"}"#, "\n",
 r#"{"title":"escape","url":"https://localhost/secret"}"#), "youtube").unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].position, 0);
    assert_eq!(rows[0].duration, Some(12.5));
    assert!(!rows[1].available);
    assert!(!rows[2].available);
    assert!(rows[2].url.is_empty());
    assert!(ditto_online::parse_metadata("not json", "youtube").is_err());
}
#[cfg(unix)]
#[test]
fn metadata_returns_valid_entries_when_one_playlist_item_fails() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let helper = t.path().join("helper");
    std::fs::write(&helper,"#!/bin/sh\nprintf '%s\\n' '{\"title\":\"Available item\",\"url\":\"https://www.youtube.com/watch?v=BaW_jenozKc\"}'\nexit 1\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let engine = OnlineEngine {
        yt_dlp: helper,
        deno: PathBuf::from("/managed/deno"),
        work_dir: t.path().join("work"),
        ffmpeg_dir: PathBuf::from("/managed/audio"),
    };
    let rows = engine
        .enumerate(
            "https://www.youtube.com/playlist?list=test",
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].available);
}
#[cfg(unix)]
#[test]
fn fully_unavailable_enumeration_is_error_even_when_helper_ignores_errors() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let helper = t.path().join("helper");
    std::fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let engine = OnlineEngine {
        yt_dlp: helper,
        deno: PathBuf::from("/managed/deno"),
        work_dir: t.path().join("work"),
        ffmpeg_dir: PathBuf::from("/managed/audio"),
    };
    assert!(engine
        .enumerate(
            "https://youtu.be/BaW_jenozKc",
            &Arc::new(AtomicBool::new(false))
        )
        .is_err());
    assert!(engine
        .search("youtube", "no results", &Arc::new(AtomicBool::new(false)))
        .unwrap()
        .is_empty());
}
#[test]
fn relative_helper_paths_are_never_looked_up_in_host_path() {
    let t = tempfile::tempdir().unwrap();
    let engine = OnlineEngine {
        yt_dlp: PathBuf::from("yt-dlp"),
        deno: PathBuf::from("deno"),
        work_dir: t.path().join("work"),
        ffmpeg_dir: PathBuf::from("ffmpeg"),
    };
    let error = engine
        .enumerate(
            "https://youtu.be/BaW_jenozKc",
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("absolute helper"), "{error}");
}
#[cfg(unix)]
#[test]
fn enumeration_and_search_are_metadata_only_and_cancel_before_spawn() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let helper = t.path().join("helper");
    std::fs::write(
        &helper,
        r##"#!/bin/sh
printf '%s\n' "$@" > args.txt
printf '%s\n' '{"title":"Public test","url":"https://www.youtube.com/watch?v=BaW_jenozKc"}'
"##,
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let engine = OnlineEngine {
        yt_dlp: helper,
        deno: PathBuf::from("/managed/deno"),
        work_dir: t.path().join("work"),
        ffmpeg_dir: PathBuf::from("/managed/audio"),
    };
    let c = Arc::new(AtomicBool::new(false));
    let rows: Vec<OnlineEntry> = engine
        .enumerate("https://www.youtube.com/watch?v=BaW_jenozKc", &c)
        .unwrap();
    assert_eq!(rows.len(), 1);
    let args = std::fs::read_to_string(engine.work_dir.join("args.txt")).unwrap();
    for flag in [
        "--simulate",
        "--flat-playlist",
        "--ignore-config",
        "--no-plugin-dirs",
        "--no-remote-components",
        "--no-update",
        "--playlist-end",
    ] {
        assert!(args.lines().any(|s| s == flag), "{flag}");
    }
    assert!(!args.contains("--cookies"));
    assert!(!args.contains("--exec"));
    engine.search("youtube", "ambient backing", &c).unwrap();
    let args = std::fs::read_to_string(engine.work_dir.join("args.txt")).unwrap();
    assert!(args.contains("ytsearch20:ambient backing"));
    assert!(engine.search("unknown", "hi", &c).is_err());
    c.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(engine
        .enumerate("https://youtu.be/BaW_jenozKc", &c)
        .is_err());
}
