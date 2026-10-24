//! Read-only native geometry; this module exists only in macOS smoke builds.
use objc2::MainThreadMarker;
use objc2_app_kit::{NSView, NSWindow};
use objc2_foundation::NSRect;
use serde_json::{json, Value};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

struct Trace {
    origin: Instant,
    events: Vec<Value>,
}

fn trace() -> &'static Mutex<Trace> {
    static TRACE: OnceLock<Mutex<Trace>> = OnceLock::new();
    TRACE.get_or_init(|| {
        Mutex::new(Trace {
            origin: Instant::now(),
            events: Vec::new(),
        })
    })
}

pub fn event(stage: &str, detail: Value) {
    let mut trace = trace().lock().unwrap();
    let entry = json!({"sequence":trace.events.len()+1,
        "monotonicNs":trace.origin.elapsed().as_nanos().to_string(),
        "stage":stage,"mainThread":MainThreadMarker::new().is_some(),"detail":detail});
    trace.events.push(entry);
}

fn rect(rect: NSRect) -> [f64; 4] {
    [
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    ]
}

fn enqueue_snapshot(
    window: tauri::WebviewWindow,
) -> Result<mpsc::Receiver<Result<Value, String>>, String> {
    event("snapshot-enqueued", json!({}));
    let (send, receive) = mpsc::sync_channel(1);
    let runtime = window.clone();
    window.with_webview(move |platform| {
        let result = (|| -> Result<Value, String> {
            let _main = MainThreadMarker::new().ok_or("AppKit snapshot was not on the main thread")?;
            // SAFETY: Tauri's pinned PlatformWebview returns its live NSWindow
            // and WKWebView. WKWebView is an NSView subclass. Borrow only for
            // this main-thread callback; never retain/send native references.
            let native = unsafe { platform.ns_window().cast::<NSWindow>().as_ref() }
                .ok_or("Native NSWindow missing")?;
            let wk = unsafe { platform.inner().cast::<NSView>().as_ref() }
                .ok_or("Native WKWebView missing")?;
            let content = native.contentView().ok_or("Native contentView missing")?;
            let frame = native.frame();
            let minimum = native.minSize();
            let content_minimum = native.contentMinSize();
            let outer = runtime.outer_size().map_err(|e| e.to_string())?;
            let inner = runtime.inner_size().map_err(|e| e.to_string())?;
            let position = runtime.outer_position().map_err(|e| e.to_string())?;
            let scale = runtime.scale_factor().map_err(|e| e.to_string())?;
            let observation = json!({
                "units":{"appKit":"logical points; native coordinate systems","runtime":"physical pixels"},
                "nsWindowFrame":rect(frame),
                "contentRectForFrameRect":rect(native.contentRectForFrameRect(frame)),
                "contentViewFrame":rect(content.frame()),"contentViewBounds":rect(content.bounds()),
                "contentLayoutRect":rect(native.contentLayoutRect()),
                "styleMask":native.styleMask().bits(),
                "minimumFrame":[minimum.width,minimum.height],
                "minimumContent":[content_minimum.width,content_minimum.height],
                "wkWebViewFrame":rect(wk.frame()),"wkWebViewBounds":rect(wk.bounds()),
                "nativeBackingScale":native.backingScaleFactor(),
                "runtimeOuter":[position.x,position.y,outer.width,outer.height],
                "runtimeInner":[inner.width,inner.height],"runtimeScale":scale
            });
            event("snapshot-captured", observation.clone());
            let trace = trace().lock().map_err(|e| e.to_string())?;
            Ok(json!({"observation":observation,"trace":trace.events}))
        })();
        // Capacity one, exactly one send; never wait on the UI thread.
        let _ = send.try_send(result);
    }).map_err(|e| e.to_string())?;
    Ok(receive)
}

pub fn immediate_snapshot(window: tauri::WebviewWindow) -> Result<Value, String> {
    MainThreadMarker::new().ok_or("Synchronous smoke helper was not on the main thread")?;
    // Pinned tauri-runtime-wry send_user_message executes inline on its main
    // thread. Do not wait or pump events here: preserve original setter timing.
    enqueue_snapshot(window)?
        .try_recv()
        .map_err(|e| format!("Immediate native geometry snapshot unavailable: {e}"))?
}

// Convert fixture viewport points, not runtime `inner_size` points. With the
// pinned opaque/full-size titlebar, WebKit reserves contentLayoutRect's inset.
// Reuse the bounded main-thread snapshot; never guess a titlebar constant.
pub fn viewport_content_size(
    window: tauri::WebviewWindow,
    viewport: (f64, f64),
) -> Result<(f64, f64), String> {
    let snapshot = immediate_snapshot(window)?;
    let observation = &snapshot["observation"];
    let dimensions = |key: &str| -> Result<(f64, f64), String> {
        let width = observation[key][2].as_f64().ok_or("Missing native width")?;
        let height = observation[key][3]
            .as_f64()
            .ok_or("Missing native height")?;
        if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
            return Err("Invalid native content geometry".into());
        }
        Ok((width, height))
    };
    let content = dimensions("contentViewBounds")?;
    let layout = dimensions("contentLayoutRect")?;
    if layout.0 > content.0 || layout.1 > content.1 {
        return Err("Native layout exceeds content bounds".into());
    }
    let request = super::smoke_viewport::content_size(viewport, content, layout);
    event(
        "helper-viewport-content-request",
        json!({"viewportLogical":viewport,
        "contentLogical":content,"layoutLogical":layout,"requestLogical":request}),
    );
    Ok(request)
}

pub async fn snapshot(window: tauri::WebviewWindow) -> Result<Value, String> {
    let receive = enqueue_snapshot(window)?;
    // Only a blocking-pool thread waits. The main thread remains free to run
    // the callback; timeout is diagnostic failure, never fabricated geometry.
    tauri::async_runtime::spawn_blocking(move || {
        receive
            .recv_timeout(Duration::from_millis(1000))
            .map_err(|e| format!("Native geometry snapshot unavailable: {e}"))?
    })
    .await
    .map_err(|e| e.to_string())?
}
