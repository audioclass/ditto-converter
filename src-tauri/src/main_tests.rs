use super::*;

fn queue() -> Coordinator {
    Coordinator::new(
        Arc::new(NativePipeline::new(
            "unused-bin".into(),
            "unused-work".into(),
        )),
        "unused-work".into(),
    )
}
fn entry(title: &str) -> OnlineEntry {
    OnlineEntry {
        title: title.into(),
        url: "https://youtu.be/aqz-KE-bpKQ".into(),
        provider: "youtube".into(),
        duration: Some(2.),
        position: 0,
        available: true,
    }
}

#[test]
fn snapshot_event_retries_failed_delivery_then_deduplicates_success() {
    let q = queue();
    let mut old = String::new();
    let mut attempts = 0;
    publish_snapshot(&q, &mut old, |_| {
        attempts += 1;
        Err(())
    });
    assert!(old.is_empty(), "failed delivery must remain pending");
    publish_snapshot(&q, &mut old, |_| {
        attempts += 1;
        Ok::<_, ()>(())
    });
    publish_snapshot(&q, &mut old, |_| {
        attempts += 1;
        Ok::<_, ()>(())
    });
    assert_eq!(attempts, 2);
    q.add_online(vec![entry("review")]).unwrap();
    let mut events = Vec::new();
    publish_snapshot(&q, &mut old, |s| {
        events.push(s);
        Ok::<_, ()>(())
    });
    let id = events[0].jobs[0].id.clone();
    q.remove(&id).unwrap();
    publish_snapshot(&q, &mut old, |s| {
        events.push(s);
        Ok::<_, ()>(())
    });
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].jobs[0].state, "queued");
    assert!(events[1].jobs.is_empty());
}

#[test]
fn activity_guard_rejects_overlap_and_releases_busy_and_shutdown_count() {
    let busy = Arc::new(AtomicBool::new(false));
    let active = Arc::new(AtomicUsize::new(0));
    let guard = Activity::begin(busy.clone(), active.clone()).unwrap();
    assert_eq!(active.load(Ordering::SeqCst), 1);
    assert!(Activity::begin(busy.clone(), active.clone()).is_err());
    assert_eq!(active.load(Ordering::SeqCst), 1);
    drop(guard);
    assert!(!busy.load(Ordering::SeqCst));
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[cfg(unix)]
mod metadata {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, time::Instant};
    struct Fixture {
        dir: PathBuf,
        pipeline: NativePipeline,
    }
    impl Fixture {
        fn new(ending: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../.work/metadata-callback-tests")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::SeqCst)
                ));
            std::fs::create_dir_all(&dir).unwrap();
            let helper = dir.join("extractor");
            std::fs::write(
                &helper,
                format!(
                    r#"#!/bin/sh
printf '%s\n' "$@" > args.txt
printf '%s\n' '{{"title":"First","url":"aqz-KE-bpKQ"}}'
i=0
while [ ! -f release ]; do
    i=$((i+1)); [ "$i" -lt 500 ] || exit 9
    sleep 0.01
done
{ending}
"#
                ),
            )
            .unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            let pipeline = NativePipeline::new(dir.join("bin"), dir.join("work"));
            pipeline.activate(ditto_online::components::InstalledPack {
                platform: "controlled".into(),
                tuple_id: "controlled".into(),
                yt_dlp: helper,
                deno: dir.join("deno"),
            });
            Self { dir, pipeline }
        }
        fn first(&self, q: &Coordinator) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while q.snapshot().jobs.is_empty() {
                assert!(
                    Instant::now() < deadline,
                    "metadata callback did not publish progressively"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        fn release(&self) {
            std::fs::write(self.dir.join("work/release"), b"go").unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn progressive_import_removal_does_not_replay_rows_on_late_parser_error() {
        let f = Fixture::new(
            "printf '%s\\n' '{\"title\":\"Last\",\"url\":\"aqz-KE-bpKQ\"}' 'malformed'",
        );
        let q = queue();
        let worker_queue = q.clone();
        let engine = f.pipeline.online().unwrap();
        let task = std::thread::spawn(move || {
            import_metadata(
                &engine,
                "https://www.youtube.com/playlist?list=test",
                &Arc::default(),
                &worker_queue,
            )
        });
        f.first(&q);
        let first = q.snapshot().jobs[0].clone();
        assert_eq!(first.title, "First");
        assert_eq!(first.state, "queued");
        assert!(!q.snapshot().running);
        q.remove(&first.id).unwrap();
        f.release();
        assert!(task.join().unwrap().is_err());
        let rows = q.snapshot().jobs;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Last");
        assert_ne!(rows[0].id, first.id);
        assert!(rows[0].position > first.position);
        let args = std::fs::read_to_string(f.dir.join("work/args.txt")).unwrap();
        for flag in [
            "--simulate",
            "--skip-download",
            "--flat-playlist",
            "--lazy-playlist",
        ] {
            assert!(args.lines().any(|s| s == flag));
        }
        assert!(!args.contains("--cookies"));
    }

    #[test]
    fn successful_metadata_completion_keeps_removal_and_unavailable_sibling_review() {
        let f = Fixture::new(
            "printf '%s\\n' '{\"title\":\"[Deleted video]\",\"url\":\"xxxxxxxxxxx\"}' '{\"title\":\"Last\",\"url\":\"aqz-KE-bpKQ\"}'",
        );
        let q = queue();
        let worker_queue = q.clone();
        let engine = f.pipeline.online().unwrap();
        let task = std::thread::spawn(move || {
            import_metadata(
                &engine,
                "https://www.youtube.com/playlist?list=test",
                &Arc::default(),
                &worker_queue,
            )
        });
        f.first(&q);
        let first = q.snapshot().jobs[0].id.clone();
        q.remove(&first).unwrap();
        f.release();
        task.join().unwrap().unwrap();
        let rows = q.snapshot().jobs;
        assert_eq!(
            rows.iter().map(|j| j.title.as_str()).collect::<Vec<_>>(),
            ["[Deleted video]", "Last"]
        );
        assert_eq!(rows[0].state, "failed");
        assert!(!rows[0].selected);
        assert_eq!(rows[1].state, "queued");
        assert!(rows[1].selected);
        assert!(rows[0].position < rows[1].position);
        assert!(rows.iter().all(|j| j.id != first && j.output.is_none()));
        assert!(!q.snapshot().running);
    }

    #[test]
    fn metadata_cancellation_preserves_reviewed_rows_without_final_replay() {
        let f = Fixture::new("printf '%s\\n' '{\"title\":\"Late\",\"url\":\"aqz-KE-bpKQ\"}'");
        let q = queue();
        let worker_queue = q.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let engine = f.pipeline.online().unwrap();
        let task = std::thread::spawn(move || {
            import_metadata(
                &engine,
                "https://youtu.be/aqz-KE-bpKQ",
                &worker_cancel,
                &worker_queue,
            )
        });
        f.first(&q);
        q.update(&q.snapshot().jobs[0].id, Some(false), None, None)
            .unwrap();
        cancel.store(true, Ordering::SeqCst);
        f.release();
        assert!(task.join().unwrap().is_err());
        assert_eq!(q.snapshot().jobs.len(), 1);
        assert!(!q.snapshot().jobs[0].selected);
    }

    #[test]
    fn full_queue_reports_admission_error_instead_of_generic_cancellation() {
        let f = Fixture::new("exit 0");
        let q = queue();
        for _ in 0..2 {
            q.add_online((0..1000).map(|_| entry("existing")).collect())
                .unwrap();
        }
        let cancel = Arc::default();
        let error = import_metadata(
            &f.pipeline.online().unwrap(),
            "https://youtu.be/aqz-KE-bpKQ",
            &cancel,
            &q,
        )
        .unwrap_err();
        assert!(error.contains("queue holds at most 2000"), "{error}");
        assert!(cancel.load(Ordering::SeqCst));
        assert_eq!(q.snapshot().jobs.len(), 2000);
    }
}
