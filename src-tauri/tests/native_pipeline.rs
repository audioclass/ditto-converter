use ditto_app::{
    coordinator::{Coordinator, OnlineEntry, Pipeline, Source},
    runtime::NativePipeline,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

struct Fixture {
    dir: PathBuf,
    source: PathBuf,
    pipeline: Arc<NativePipeline>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        let dir = root.join(".work/native-pipeline-tests").join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        // A real, deterministic two-second stereo PCM fixture; no host ffmpeg
        // is used to generate it. Production bundled helpers must decode it.
        let samples = 96_000u32;
        let size = samples * 4;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&192_000u32.to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&size.to_le_bytes());
        for n in 0..samples {
            let sample = ((n % 100) as i16 - 50) * 100;
            bytes.extend_from_slice(&sample.to_le_bytes());
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(&source, bytes).unwrap();
        let pipeline = Arc::new(NativePipeline::new(
            root.join("src-tauri/resources/bin"),
            dir.join("session"),
        ));
        Self {
            dir,
            source,
            pipeline,
        }
    }
    fn queue(&self) -> Coordinator {
        // Same Arc<NativePipeline> -> Arc<dyn Pipeline> wiring as app setup.
        let queue = Coordinator::new(self.pipeline.clone(), self.dir.join("session"));
        queue.set_output_dir(self.dir.clone()).unwrap();
        queue
    }
    fn wait(&self, queue: &Coordinator) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while queue.snapshot().running {
            assert!(
                Instant::now() < deadline,
                "native conversion did not finish"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        for job in queue.snapshot().jobs {
            assert_eq!(job.state, "complete", "{:?}", job.error);
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn native_pipeline_offline_probe_preview_and_conversion_do_not_need_optional_tools() {
    let f = Fixture::new();
    assert!(f.pipeline.online().is_err());
    assert!(f
        .pipeline
        .probe(&f.source, &Arc::new(AtomicBool::new(true)))
        .is_err());
    assert_eq!(f.pipeline.probe(&f.source, &Arc::default()).unwrap(), 2.);
    let original = std::fs::read(&f.source).unwrap();
    let q = f.queue();
    q.add_local(vec![f.source.clone()]).unwrap();
    let id = q.snapshot().jobs[0].id.clone();
    let preview = q.preview(&id).unwrap();
    assert!(preview.is_file());
    assert_ne!(preview, f.source);
    assert_eq!(f.pipeline.probe(&preview, &Arc::default()).unwrap(), 2.);
    q.update(&id, None, Some(0.25), Some(Some(1.25))).unwrap();
    q.start().unwrap();
    f.wait(&q);
    let output = PathBuf::from(q.snapshot().jobs[0].output.as_ref().unwrap());
    assert_ne!(output, f.source);
    let info = ditto_core::audio::validate_wav(&output, &Arc::default()).unwrap();
    assert_eq!(info.samples, 44_100);
    assert_eq!(std::fs::read(&f.source).unwrap(), original);
    q.remove(&id).unwrap();
    assert!(output.is_file());
    assert!(!preview.exists());
}

#[test]
fn broken_optional_provider_does_not_block_real_offline_work_in_the_same_queue() {
    let f = Fixture::new();
    let original = std::fs::read(&f.source).unwrap();
    // Actual spawn failures, not a substitute Pipeline implementation. No
    // network or optional installation is needed to reproduce provider failure.
    f.pipeline
        .activate(ditto_online::components::InstalledPack {
            platform: "controlled-missing".into(),
            tuple_id: "controlled-missing".into(),
            yt_dlp: f.dir.join("missing-extractor"),
            deno: f.dir.join("missing-deno"),
        });
    assert!(f
        .pipeline
        .online()
        .unwrap()
        .enumerate_stream("https://youtu.be/aqz-KE-bpKQ", &Arc::default(), |_| {
            panic!("failed metadata helper must not emit review rows")
        })
        .is_err());
    let q = f.queue();
    q.component_status(true, "broken optional fixture".into());
    q.add_online(vec![OnlineEntry {
        title: "provider failure".into(),
        url: "https://youtu.be/aqz-KE-bpKQ".into(),
        provider: "youtube".into(),
        duration: Some(2.),
        position: 0,
        available: true,
    }])
    .unwrap();
    q.add_local(vec![f.source.clone()]).unwrap();
    q.start().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while q.snapshot().running {
        assert!(
            Instant::now() < deadline,
            "provider failure blocked local work"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let rows = q.snapshot().jobs;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].state, "failed");
    assert!(rows[0].error.is_some());
    assert!(rows[0].output.is_none());
    assert_eq!(rows[1].state, "complete", "{:?}", rows[1].error);
    let output = PathBuf::from(rows[1].output.as_ref().unwrap());
    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(
        ditto_core::audio::validate_wav(&output, &Arc::default())
            .unwrap()
            .samples,
        88_200
    );
    assert_eq!(std::fs::read(&f.source).unwrap(), original);
    assert!(!f.dir.join(format!("session/job-{}", rows[0].id)).exists());
    q.remove(&rows[0].id).unwrap();
    q.remove(&rows[1].id).unwrap();
    assert_eq!(std::fs::read(output).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn activated_native_pipeline_forwards_acquisition_progress_then_uses_real_audio_engine() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let helper = f.dir.join("extractor");
    std::fs::write(
        &helper,
        r#"#!/bin/sh
printf '%s\n' "$@" > args.txt
cp "${0%/*}/source.wav" source.wav
printf '%s\n' 'DITTO_PROGRESS {"downloaded_bytes":5,"total_bytes":10}'
printf 'DITTO_FILE "%s/source.wav"\n' "$PWD"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    f.pipeline
        .activate(ditto_online::components::InstalledPack {
            platform: "controlled".into(),
            tuple_id: "controlled".into(),
            yt_dlp: helper.clone(),
            deno: f.dir.join("deno"),
        });
    let online = f.pipeline.online().unwrap();
    assert_eq!(online.yt_dlp, helper);
    assert_eq!(online.work_dir, f.dir.join("session"));
    assert_eq!(online.ffmpeg_dir, f.pipeline.audio.ffmpeg.parent().unwrap());
    let source = Source::Online {
        url: "https://youtu.be/aqz-KE-bpKQ".into(),
        provider: "youtube".into(),
    };
    let mut progress = Vec::new();
    std::fs::create_dir_all(f.dir.join("session/explicit-acquisition")).unwrap();
    std::fs::create_dir_all(f.dir.join("session/progress-acquisition")).unwrap();
    let acquired = f
        .pipeline
        .acquire(
            &source,
            &f.dir.join("session/explicit-acquisition"),
            &Arc::default(),
            &|p| {
                // Fn rather than FnMut is the actual production callback contract.
                assert!((0. ..=1.).contains(&p));
            },
        )
        .unwrap();
    assert_eq!(
        std::fs::read(acquired).unwrap(),
        std::fs::read(&f.source).unwrap()
    );
    let seen = std::sync::Mutex::new(&mut progress);
    f.pipeline
        .acquire(
            &source,
            &f.dir.join("session/progress-acquisition"),
            &Arc::default(),
            &|p| seen.lock().unwrap().push(p),
        )
        .unwrap();
    assert!(progress.contains(&0.5));
    assert!(f
        .pipeline
        .acquire(
            &Source::Local {
                path: f.source.to_string_lossy().into()
            },
            &f.dir.join("invalid"),
            &Arc::default(),
            &|_| {}
        )
        .is_err());
    let q = f.queue();
    q.component_status(true, "controlled optional tools".into());
    q.add_online(vec![OnlineEntry {
        title: "online fixture".into(),
        url: "https://youtu.be/aqz-KE-bpKQ".into(),
        provider: "youtube".into(),
        duration: Some(2.),
        position: 0,
        available: true,
    }])
    .unwrap();
    q.start().unwrap();
    f.wait(&q);
    let output = PathBuf::from(q.snapshot().jobs[0].output.as_ref().unwrap());
    assert_eq!(
        ditto_core::audio::validate_wav(&output, &Arc::default())
            .unwrap()
            .samples,
        88_200
    );
    assert!(!f.dir.join("session/job-1").exists());
}
