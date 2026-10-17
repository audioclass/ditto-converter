use ditto_app::{
    coordinator::{Coordinator, Pipeline},
    runtime::NativePipeline,
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
#[test]
fn shared_queue_uses_bundled_engine_and_preserves_original_on_trim_collision() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    #[cfg(unix)]
    let root = root.canonicalize().unwrap();
    let work = root.join(".work/integrated-queue");
    std::fs::create_dir_all(&work).unwrap();
    let input = work.join("source.wav");
    let fixture_ffmpeg =
        std::env::var_os("DITTO_FIXTURE_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let generated = std::process::Command::new(fixture_ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-y",
        ])
        .arg(&input)
        .status()
        .unwrap();
    assert!(generated.success());
    let original = std::fs::read(&input).unwrap();
    let pipeline = NativePipeline::new(root.join("src-tauri/resources/bin"), work.clone());
    assert_eq!(pipeline.probe(&input, &Arc::default()).unwrap(), 2.0);
    let a = Coordinator::new(Arc::new(pipeline), work.join("jobs"));
    a.set_output_dir(work.clone()).unwrap();
    a.add_local(vec![input.clone()]).unwrap();
    let id = a.snapshot().jobs[0].id.clone();
    a.update(&id, None, Some(0.25), Some(Some(1.25))).unwrap();
    a.start().unwrap();
    let t = Instant::now();
    while a.snapshot().running {
        assert!(t.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(20));
    }
    let j = &a.snapshot().jobs[0];
    assert_eq!(j.state, "complete", "{:?}", j.error);
    let output = PathBuf::from(j.output.as_ref().unwrap());
    assert_ne!(output, input);
    let data = std::fs::read(&output).unwrap();
    assert_eq!(&data[0..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(data[40..44].try_into().unwrap()), 132300);
    assert_eq!(std::fs::read(input).unwrap(), original);
}
