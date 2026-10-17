#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use ditto_app::{
    coordinator::{Coordinator, OnlineEntry, Snapshot},
    runtime::NativePipeline,
    settings::Settings,
};
use ditto_online::components::{approved_manifest, host_platform, ComponentManager};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{Emitter, Manager, State};

struct AppState {
    queue: Coordinator,
    pipeline: Arc<NativePipeline>,
    manager: ComponentManager,
    data: PathBuf,
    metadata_cancel: Arc<AtomicBool>,
    preview_cancel: Arc<AtomicBool>,
    setup_cancel: Arc<AtomicBool>,
    file_cancel: Arc<AtomicBool>,
    metadata_busy: Arc<AtomicBool>,
    preview_busy: Arc<AtomicBool>,
    setup_busy: Arc<AtomicBool>,
    file_busy: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    previews: Mutex<std::collections::HashMap<String, PathBuf>>,
    pipeline_gate: Mutex<()>,
    _lock: std::fs::File,
}
struct Activity {
    busy: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
}
impl Activity {
    fn begin(busy: Arc<AtomicBool>, active: Arc<AtomicUsize>) -> Result<Self, String> {
        busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "This operation is already running".to_string())?;
        active.fetch_add(1, Ordering::SeqCst);
        Ok(Self { busy, active })
    }
}
impl Drop for Activity {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::SeqCst);
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl AppState {
    fn cancel_all(&self) {
        self.queue.cancel_all();
        for t in [
            &self.metadata_cancel,
            &self.preview_cancel,
            &self.setup_cancel,
            &self.file_cancel,
        ] {
            t.store(true, Ordering::SeqCst);
        }
    }
}
fn mapped(e: ditto_online::OnlineEntry) -> OnlineEntry {
    OnlineEntry {
        title: e.title,
        url: e.url,
        provider: e.provider,
        duration: e.duration,
        position: e.position,
        available: e.available,
    }
}
#[tauri::command]
fn snapshot(s: State<'_, AppState>) -> Snapshot {
    s.queue.snapshot()
}
#[tauri::command(rename_all = "snake_case")]
async fn add_files(paths: Vec<String>, s: State<'_, AppState>) -> Result<(), String> {
    let guard = Activity::begin(s.file_busy.clone(), s.active.clone())?;
    s.file_cancel.store(false, Ordering::SeqCst);
    let cancel = s.file_cancel.clone();
    let q = s.queue.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        q.add_local_with_cancel(paths.into_iter().map(PathBuf::from).collect(), &cancel)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command(rename_all = "snake_case")]
fn set_output_dir(path: String, s: State<'_, AppState>) -> Result<(), String> {
    s.queue.set_output_dir(PathBuf::from(path))?;
    Settings {
        output_dir: s.queue.snapshot().output_dir,
    }
    .save(&s.data.join("settings.json"))
}
#[tauri::command]
fn start_queue(s: State<'_, AppState>) -> Result<(), String> {
    let _gate = s.pipeline_gate.lock().unwrap();
    if s.preview_busy.load(Ordering::SeqCst) {
        return Err("Stop or finish preview preparation before converting".into());
    }
    if s.metadata_busy.load(Ordering::SeqCst)
        && s.queue.snapshot().jobs.iter().any(|j| {
            j.selected && matches!(j.source, ditto_app::coordinator::Source::Online { .. })
        })
    {
        return Err(
            "Finish or cancel metadata import, then review the online queue before converting"
                .into(),
        );
    }
    s.queue.start()
}
#[tauri::command(rename_all = "snake_case")]
fn update_job(
    id: String,
    selected: Option<bool>,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    request: tauri::ipc::Request<'_>,
    s: State<'_, AppState>,
) -> Result<(), String> {
    let explicit_end = match request.body() {
        tauri::ipc::InvokeBody::Json(v) => v.get("trim_end").map(|_| trim_end),
        _ => None,
    };
    s.queue.update(&id, selected, trim_start, explicit_end)
}
#[tauri::command]
fn cancel_job(id: String, s: State<'_, AppState>) -> Result<(), String> {
    s.preview_cancel.store(true, Ordering::SeqCst);
    s.queue.cancel(&id)
}
#[tauri::command]
fn remove_job(id: String, app: tauri::AppHandle, s: State<'_, AppState>) -> Result<(), String> {
    s.preview_cancel.store(true, Ordering::SeqCst);
    if let Some(path) = s.previews.lock().unwrap().remove(&id) {
        let _ = app.asset_protocol_scope().forbid_file(&path);
        let _ = std::fs::remove_file(path);
    }
    s.queue.remove(&id)
}
#[tauri::command]
fn retry_job(id: String, s: State<'_, AppState>) -> Result<(), String> {
    s.queue.retry(&id)
}
#[tauri::command]
fn add_online(entries: Vec<OnlineEntry>, s: State<'_, AppState>) -> Result<(), String> {
    for e in &entries {
        let provider = ditto_online::validate_url(&e.url).map_err(|e| e.to_string())?;
        if provider != e.provider {
            return Err("Provider identity mismatch".into());
        }
    }
    s.queue.add_online(entries)
}
#[tauri::command]
async fn import_url(url: String, s: State<'_, AppState>) -> Result<(), String> {
    let guard = Activity::begin(s.metadata_busy.clone(), s.active.clone())?;
    let engine = s.pipeline.online()?;
    s.metadata_cancel.store(false, Ordering::SeqCst);
    let c = s.metadata_cancel.clone();
    let q = s.queue.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let mut admission_error = None;
        let outcome = engine.enumerate_stream(&url, &c, |entry| {
            if admission_error.is_none() {
                if let Err(e) = q.add_online(vec![mapped(entry)]) {
                    admission_error = Some(e);
                    c.store(true, Ordering::SeqCst);
                }
            }
        });
        if let Some(error) = admission_error {
            return Err(error);
        }
        outcome.map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn search(
    provider: String,
    query: String,
    s: State<'_, AppState>,
) -> Result<Vec<OnlineEntry>, String> {
    let guard = Activity::begin(s.metadata_busy.clone(), s.active.clone())?;
    let engine = s.pipeline.online()?;
    s.metadata_cancel.store(false, Ordering::SeqCst);
    let c = s.metadata_cancel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        engine
            .search(&provider, &query, &c)
            .map(|v| v.into_iter().map(mapped).collect())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn cancel_metadata(s: State<'_, AppState>) {
    s.metadata_cancel.store(true, Ordering::SeqCst);
}
#[tauri::command]
fn cancel_preview(s: State<'_, AppState>) {
    s.preview_cancel.store(true, Ordering::SeqCst);
}
#[tauri::command]
async fn preview_job(
    id: String,
    app: tauri::AppHandle,
    s: State<'_, AppState>,
) -> Result<String, String> {
    if s.queue.snapshot().running {
        return Err("Pause listening while the conversion queue is running".into());
    }
    if let Some(path) = s
        .previews
        .lock()
        .unwrap()
        .get(&id)
        .filter(|p| p.is_file())
        .cloned()
    {
        app.asset_protocol_scope()
            .allow_file(&path)
            .map_err(|e| e.to_string())?;
        return Ok(path.to_string_lossy().into_owned());
    }
    let guard = {
        let _gate = s.pipeline_gate.lock().unwrap();
        if s.queue.snapshot().running {
            return Err("Pause listening while the conversion queue is running".into());
        }
        Activity::begin(s.preview_busy.clone(), s.active.clone())?
    };
    for (_, path) in s.previews.lock().unwrap().drain() {
        let _ = app.asset_protocol_scope().forbid_file(&path);
        let _ = std::fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    s.preview_cancel.store(false, Ordering::SeqCst);
    let cancel = s.preview_cancel.clone();
    let q = s.queue.clone();
    let p = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        q.preview_with_cancel(&id, &cancel).map(|p| (id, p))
    })
    .await
    .map_err(|e| e.to_string())??;
    app.asset_protocol_scope()
        .allow_file(&p.1)
        .map_err(|e| e.to_string())?;
    s.previews.lock().unwrap().insert(p.0, p.1.clone());
    Ok(p.1.to_string_lossy().into_owned())
}
#[tauri::command]
fn component_info(s: State<'_, AppState>) -> serde_json::Value {
    let pin = approved_manifest(host_platform()).ok();
    serde_json::json!({"status":s.queue.snapshot().component_status,"available":s.queue.snapshot().online_available,"platform":host_platform(),"download_bytes":pin.as_ref().map(|p|p.yt_dlp.length+p.deno.length),"tuple":pin.map(|p|p.tuple_id),"updates":"Only app-embedded immutable pins enabled. Independent updates require authenticated publisher metadata.","hardware_tested":false})
}
#[tauri::command]
async fn install_components(s: State<'_, AppState>) -> Result<(), String> {
    let guard = Activity::begin(s.setup_busy.clone(), s.active.clone())?;
    s.setup_cancel.store(false, Ordering::SeqCst);
    let cancel = s.setup_cancel.clone();
    let manager = s.manager.clone();
    let pipeline = s.pipeline.clone();
    let q = s.queue.clone();
    let already = q.snapshot().online_available;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        q.component_status(
            already,
            "Downloading and verifying the approved online tools…".into(),
        );
        match manager.install(host_platform(), &cancel, |p| {
            q.component_status(
                already,
                format!(
                    "Online tools: {:.0}% of pinned download bytes received",
                    p * 100.
                ),
            )
        }) {
            Ok(pack) => {
                pipeline.activate(pack);
                q.component_status(
                    true,
                    "Online tools installed · yt-dlp + Deno · hash-pinned development tuple".into(),
                );
                Ok(())
            }
            Err(e) => {
                q.component_status(
                    already,
                    format!(
                        "Online setup did not complete: {e}. Local conversion is still available."
                    ),
                );
                Err(e.to_string())
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn cancel_components(s: State<'_, AppState>) {
    s.setup_cancel.store(true, Ordering::SeqCst);
}
#[tauri::command]
fn repair_components(s: State<'_, AppState>) -> Result<(), String> {
    let _gate = s.pipeline_gate.lock().unwrap();
    let _repair = Activity::begin(s.setup_busy.clone(), s.active.clone())?;
    if s.queue.snapshot().running || s.active.load(Ordering::SeqCst) != 1 {
        return Err("Finish or cancel active work before repairing online tools".into());
    }
    let verified = s.manager.status().ok().flatten().is_some();
    ditto_app::component_recovery::quarantine_failed_setup(&s.data.join("components"), verified)?;
    s.queue.component_status(
        false,
        "Failed setup quarantined. Set up online tools to download the approved tuple again."
            .into(),
    );
    Ok(())
}
#[tauri::command]
fn open_output(s: State<'_, AppState>) -> Result<(), String> {
    let dir = PathBuf::from(
        s.queue
            .snapshot()
            .output_dir
            .ok_or("Choose an output folder first")?,
    )
    .canonicalize()
    .map_err(|e| e.to_string())?;
    if !dir.is_dir() {
        return Err("Output folder unavailable".into());
    }
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("/usr/bin/open");
    #[cfg(target_os = "linux")]
    let mut cmd = std::process::Command::new("/usr/bin/xdg-open");
    cmd.arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
#[cfg(feature = "smoke")]
#[tauri::command]
fn smoke_report(
    report: serde_json::Value,
    app: tauri::AppHandle,
    s: State<'_, AppState>,
) -> Result<(), String> {
    std::fs::write(
        s.data.join("native-smoke.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    app.exit(
        if report.get("passed").and_then(|v| v.as_bool()) == Some(true) {
            0
        } else {
            1
        },
    );
    Ok(())
}
fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_page_load(|webview, payload| {
            #[cfg(feature = "smoke")]
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                if let (Ok(input), Ok(output)) = (
                    std::env::var("DITTO_SMOKE_INPUT"),
                    std::env::var("DITTO_SMOKE_OUTPUT"),
                ) {
                    let config = serde_json::json!({"input":input,"output":output,"online":std::env::var("DITTO_SMOKE_ONLINE").as_deref()==Ok("1")});
                    let script = include_str!("../../scripts/native-smoke.js")
                        .replace("__SMOKE_CONFIG__", &config.to_string());
                    let _ = webview.eval(script);
                }
            }
            #[cfg(not(feature = "smoke"))]
            let _ = (webview, payload);
        })
        .setup(|app| {
            let data = std::env::var_os("DITTO_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(app.path().app_local_data_dir()?);
            std::fs::create_dir_all(&data)?;
            let data=data.canonicalize()?;
            #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(&data,std::fs::Permissions::from_mode(0o700))?;}
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(data.join("instance.lock"))?;
            fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| {
                "Another Ditto Converter instance is already using this data folder"
            })?;
            let mut recovery_errors=Vec::new();
            let journals=data.join("export-journals");
            if journals.is_dir(){for entry in std::fs::read_dir(&journals)?.take(1000){let path=entry?.path();if path.extension().and_then(|e|e.to_str())==Some("json"){if let Err(e)=ditto_core::recover_staging(&path){recovery_errors.push(e.to_string());}}}}
            let work = data.join("session");
            if work.exists() {
                std::fs::remove_dir_all(&work)?;
            }
            std::fs::create_dir_all(&work)?;
            let bin = app.path().resource_dir()?.join("resources/bin");
            let pipeline = Arc::new(NativePipeline::new(bin, work.clone()));
            let queue = Coordinator::new(pipeline.clone(), work);
            if !recovery_errors.is_empty(){queue.notice(format!("Some interrupted export staging was preserved because safe recovery could not validate it: {}. New local conversions remain available.",recovery_errors.join("; ")));}
            if let Some(out) = Settings::load(&data.join("settings.json")).output_dir {
                let _ = queue.set_output_dir(PathBuf::from(out));
            }
            let manager = ComponentManager::new(data.join("components"));
            match manager.status() {
                Ok(Some(pack)) => {
                    pipeline.activate(pack);
                    queue.component_status(
                        true,
                        "Online tools installed · pinned tuple verified".into(),
                    );
                }
                Ok(None) => {}
                Err(e) => queue
                    .component_status(false, format!("Online component verification failed: {e}")),
            }
            app.manage(AppState {
                queue: queue.clone(),
                pipeline,
                manager,
                data,
                metadata_cancel: Arc::default(),
                preview_cancel: Arc::default(),
                setup_cancel: Arc::default(),
                file_cancel: Arc::default(),
                metadata_busy: Arc::default(),
                preview_busy: Arc::default(),
                setup_busy: Arc::default(),
                file_busy: Arc::default(),
                active: Arc::new(AtomicUsize::new(0)),
                previews: Mutex::default(),
                pipeline_gate: Mutex::default(),
                _lock: lock,
            });
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut old = String::new();
                loop {
                    let snapshot = queue.snapshot();
                    let new = serde_json::to_string(&snapshot).unwrap_or_default();
                    if new != old {
                        let _ = handle.emit("queue-changed", snapshot);
                        old = new;
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
            });
            Ok(())
        })
        .invoke_handler(|invoke| {
            #[cfg(feature = "smoke")]
            if invoke.message.command() == "smoke_report" {
                let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool =
                    tauri::generate_handler![smoke_report];
                return handler(invoke);
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                snapshot,
                add_files,
                set_output_dir,
                start_queue,
                update_job,
                cancel_job,
                remove_job,
                retry_job,
                add_online,
                import_url,
                search,
                cancel_metadata,
                cancel_preview,
                preview_job,
                component_info,
                install_components,
                cancel_components,
                repair_components,
                open_output
            ];
            handler(invoke)
        })
        .build(tauri::generate_context!())
        .expect("Could not launch Ditto Converter");
    let closing = Arc::new(AtomicBool::new(false));
    app.run(move |app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let s = app.state::<AppState>();
            if !closing.swap(true, Ordering::SeqCst) {
                api.prevent_exit();
                s.cancel_all();
                let handle = app.clone();
                std::thread::spawn(move || {
                    loop {
                        let state = handle.state::<AppState>();
                        if !state.queue.snapshot().running
                            && state.active.load(Ordering::SeqCst) == 0
                        {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    handle.exit(0);
                });
            }
        }
    });
}
