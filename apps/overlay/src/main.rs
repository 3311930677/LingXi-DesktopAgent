//! LingXi overlay: a Tauri floating window over the capture/transform/write
//! pipeline.
//!
//! Flow:
//! - A background thread owns the global hotkeys (reusing the validated
//!   `assistant-windows` hotkey loop).
//! - Ctrl+Alt+Space captures the current selection, stores a snapshot, and
//!   shows the (non-activating) overlay.
//! - The frontend polls `current_selection`, previews transformations, then
//!   calls `apply_transform`.
//! - Ctrl+Alt+Backspace (or the Undo button) reverts the last write.
//!
//! NOTE: the overlay must NOT steal focus from the target control, otherwise
//! write-back would fail its focus-drift check. The window is configured with
//! `focus: false`; on Windows a fully click-through-without-activation window
//! also needs the `WS_EX_NOACTIVATE` extended style, applied on `setup`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use assistant_core::{
    diff_chars, transformer_by_name, DiffOp, InputAdapter, SelectionSnapshot, WriteReceipt,
};
use assistant_inference::{
    CloudBackend, CloudConfig, LocalBackend, ModelBackend, ModelTask,
};
use assistant_windows::{
    qq_latest_message, qq_write_draft, remember_foreground_if_qq, run_assistant_hotkey_loop,
    wait_for_trigger_release, AssistantHotkey, WindowsAdapter,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, SendMessageW, SetWindowLongPtrW, GWL_EXSTYLE,
    HTBOTTOMRIGHT, WM_NCLBUTTONDOWN, WS_EX_NOACTIVATE,
};

/// A preview result cached so "apply" can reuse it instead of running the model
/// a second time. Storing the exact `(mode, source)` it was computed for lets
/// `apply_transform` verify the preview still matches the captured selection
/// before writing it back.
#[derive(Clone)]
struct CachedPreview {
    mode: String,
    source: String,
    backend_signature: String,
    transformed: String,
}

/// Runtime model settings. The API key is session-only by design: endpoint,
/// model and backend choice are persisted, but plaintext credentials are not.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct BackendSettings {
    backend: String,
    endpoint: String,
    model: String,
    #[serde(skip)]
    api_key: String,
}

impl Default for BackendSettings {
    fn default() -> Self {
        Self {
            backend: "local".into(),
            endpoint: "https://api.openai.com".into(),
            model: "gpt-4o-mini".into(),
            api_key: std::env::var("LINGXI_OPENAI_API_KEY").unwrap_or_default(),
        }
    }
}

#[derive(Serialize)]
struct BackendSettingsView {
    backend: String,
    endpoint: String,
    model: String,
    api_key_configured: bool,
}

#[derive(Deserialize)]
struct BackendSettingsInput {
    backend: String,
    endpoint: String,
    model: String,
    // Accept both snake_case and the JS-idiomatic camelCase so a future
    // frontend key style change cannot break saving again.
    #[serde(alias = "apiKey")]
    api_key: String,
}

/// Shared state for rewrite, pet and semi-automatic chat flows.
struct AppState {
    snapshot: Mutex<Option<SelectionSnapshot>>,
    last_receipt: Mutex<Option<WriteReceipt>>,
    /// The most recent preview, reused by `apply_transform` so it doesn't re-run
    /// inference (which is slow and, being sampled, could differ from what the
    /// user just saw — and whose latency widens the focus-drift window).
    last_preview: Mutex<Option<CachedPreview>>,
    backend: Mutex<BackendSettings>,
    pet_status: Mutex<String>,
    last_qq_message: Mutex<Option<String>>,
    /// Incremented for every successful hotkey capture, even when the selected
    /// text is identical to the previous capture.
    selection_revision: AtomicU64,
    /// Once the user drags the panel, preserve that preferred position instead
    /// of snapping it back beside the cursor on every invocation.
    user_positioned: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(None),
            last_receipt: Mutex::new(None),
            last_preview: Mutex::new(None),
            backend: Mutex::new(load_backend_settings()),
            pet_status: Mutex::new("idle".into()),
            last_qq_message: Mutex::new(None),
            selection_revision: AtomicU64::new(0),
            user_positioned: AtomicBool::new(false),
        }
    }
}

/// One diff segment, serialized for the frontend.
#[derive(Serialize)]
struct DiffSegment {
    kind: &'static str,
    text: String,
}

#[derive(Serialize)]
struct PreviewResult {
    transformed: String,
    diff: Vec<DiffSegment>,
    /// Non-blocking quality advice. Semantic drift, truncation and runaway
    /// output remain hard errors; a merely conservative rewrite is shown.
    warning: Option<String>,
    /// True only when a local model task is waiting for model preparation.
    pending: bool,
}

#[derive(Serialize)]
struct QqMessageView {
    conversation: String,
    message: String,
    is_new: bool,
}

#[derive(Serialize)]
struct DraftWriteResult {
    verified: bool,
}

fn model_task(mode: &str) -> Option<ModelTask> {
    match mode {
        "polish" => Some(ModelTask::Polish),
        "proofread" => Some(ModelTask::Proofread),
        "prompt-enhance" => Some(ModelTask::PromptEnhance),
        _ => None,
    }
}

fn backend_signature(settings: &BackendSettings) -> String {
    format!("{}|{}|{}", settings.backend, settings.endpoint, settings.model)
}

fn run_model(settings: &BackendSettings, task: ModelTask, input: &str) -> Result<String, String> {
    match settings.backend.as_str() {
        "local" => LocalBackend
            .complete(task, input)
            .map_err(|error| format!("本地模型调用失败: {error:#}")),
        "cloud" => CloudBackend::new(CloudConfig {
            endpoint: settings.endpoint.clone(),
            model: settings.model.clone(),
            api_key: settings.api_key.clone(),
        })
        .complete(task, input)
        .map_err(|error| format!("云端模型调用失败: {error:#}")),
        other => Err(format!("unknown backend: {other}")),
    }
}

fn transform_text(settings: &BackendSettings, mode: &str, input: &str) -> Result<String, String> {
    if let Some(task) = model_task(mode) {
        return run_model(settings, task, input);
    }
    transformer_by_name(mode)
        .map(|transformer| transformer.transform(input))
        .ok_or_else(|| format!("unknown mode: {mode}"))
}

fn to_segments(ops: Vec<DiffOp>) -> Vec<DiffSegment> {
    ops.into_iter()
        .map(|op| match op {
            DiffOp::Equal(text) => DiffSegment {
                kind: "equal",
                text,
            },
            DiffOp::Insert(text) => DiffSegment { kind: "ins", text },
            DiffOp::Delete(text) => DiffSegment { kind: "del", text },
        })
        .collect()
}

fn backend_settings_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lingxi").join("settings.json"))
}

fn load_backend_settings() -> BackendSettings {
    let mut settings = backend_settings_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<BackendSettings>(&bytes).ok())
        .unwrap_or_default();
    settings.api_key = std::env::var("LINGXI_OPENAI_API_KEY").unwrap_or_default();
    // A persisted cloud choice without a session/environment key cannot work
    // after restart. Fall back safely rather than opening into repeated errors.
    if settings.backend == "cloud" && settings.api_key.is_empty() {
        settings.backend = "local".into();
    }
    settings
}

fn persist_backend_settings(settings: &BackendSettings) -> Result<(), String> {
    let path = backend_settings_path().ok_or("cannot resolve config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    // `api_key` has serde(skip), so credentials never reach this file.
    let json = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_backend_settings(state: State<AppState>) -> BackendSettingsView {
    let settings = state.backend.lock().unwrap();
    BackendSettingsView {
        backend: settings.backend.clone(),
        endpoint: settings.endpoint.clone(),
        model: settings.model.clone(),
        api_key_configured: !settings.api_key.is_empty(),
    }
}

#[tauri::command]
fn save_backend_settings(
    state: State<AppState>,
    input: BackendSettingsInput,
) -> Result<BackendSettingsView, String> {
    if input.backend != "local" && input.backend != "cloud" {
        return Err("后端只能是 local 或 cloud".into());
    }
    if input.backend == "cloud"
        && (input.endpoint.trim().is_empty() || input.model.trim().is_empty())
    {
        return Err("云端 endpoint 和 model 不能为空".into());
    }
    let mut settings = state.backend.lock().unwrap();
    settings.backend = input.backend;
    settings.endpoint = input.endpoint.trim().trim_end_matches('/').to_string();
    settings.model = input.model.trim().to_string();
    // An empty field means "keep the current session key"; the frontend never
    // receives the secret back from this process.
    if !input.api_key.trim().is_empty() {
        settings.api_key = input.api_key.trim().to_string();
    }
    persist_backend_settings(&settings)?;
    let start_local = settings.backend == "local" && !assistant_inference::is_ready();
    let view = BackendSettingsView {
        backend: settings.backend.clone(),
        endpoint: settings.endpoint.clone(),
        model: settings.model.clone(),
        api_key_configured: !settings.api_key.is_empty(),
    };
    drop(settings);
    *state.last_preview.lock().unwrap() = None;
    if start_local {
        assistant_inference::prepare_in_background();
    }
    Ok(view)
}

#[tauri::command]
fn model_progress() -> assistant_inference::ProgressSnapshot {
    assistant_inference::progress_snapshot()
}

#[tauri::command]
fn pet_status(state: State<AppState>) -> String {
    state.pet_status.lock().unwrap().clone()
}

#[tauri::command]
fn set_pet_status(state: State<AppState>, status: String) -> Result<(), String> {
    if !matches!(status.as_str(), "idle" | "thinking" | "speaking" | "alert") {
        return Err("invalid pet status".into());
    }
    *state.pet_status.lock().unwrap() = status;
    Ok(())
}

#[tauri::command]
fn toggle_panel(app: AppHandle) -> Result<bool, String> {
    let window = app
        .get_webview_window("main")
        .ok_or("panel window is unavailable")?;
    let visible = window.is_visible().map_err(|error| error.to_string())?;
    if visible {
        window.hide().map_err(|error| error.to_string())?;
        if let Some(pet) = app.get_webview_window("pet") {
            let _ = pet.show();
        }
        Ok(false)
    } else {
        // Keep the two always-on-top windows from covering each other. The pet
        // returns as soon as the panel is closed.
        if let Some(pet) = app.get_webview_window("pet") {
            let _ = pet.hide();
        }
        position_overlay(&window);
        window.show().map_err(|error| error.to_string())?;
        Ok(true)
    }
}

/// Poll the foreground QQ accessibility tree. Non-QQ foreground windows return
/// `None`, so background polling stays quiet and never reads other applications.
#[tauri::command]
async fn qq_poll_latest(state: State<'_, AppState>) -> Result<Option<QqMessageView>, String> {
    let snapshot = tauri::async_runtime::spawn_blocking(qq_latest_message)
        .await
        .map_err(|error| format!("QQ scan task failed: {error}"))?;
    let Ok(snapshot) = snapshot else {
        return Ok(None);
    };
    let mut previous = state.last_qq_message.lock().unwrap();
    let is_new = previous.as_ref() != Some(&snapshot.message);
    if is_new {
        *previous = Some(snapshot.message.clone());
        *state.pet_status.lock().unwrap() = "alert".into();
    }
    Ok(Some(QqMessageView {
        conversation: snapshot.conversation,
        message: snapshot.message,
        is_new,
    }))
}

#[tauri::command]
async fn generate_qq_draft(state: State<'_, AppState>, message: String) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("没有可回复的 QQ 消息".into());
    }
    let settings = state.backend.lock().unwrap().clone();
    *state.pet_status.lock().unwrap() = "thinking".into();
    let draft = tauri::async_runtime::spawn_blocking(move || {
        run_model(&settings, ModelTask::ChatReply, &message)
    })
    .await
    .map_err(|error| format!("draft task failed: {error}"))?;
    match draft {
        Ok(draft) => {
            *state.pet_status.lock().unwrap() = "speaking".into();
            Ok(draft)
        }
        Err(error) => {
            *state.pet_status.lock().unwrap() = "idle".into();
            Err(error)
        }
    }
}

/// Write the user-confirmed draft into QQ's composer. There is deliberately no
/// send command: the user reviews the text and presses Send inside QQ.
#[tauri::command]
async fn write_qq_draft(draft: String) -> Result<DraftWriteResult, String> {
    let verified = tauri::async_runtime::spawn_blocking(move || qq_write_draft(&draft))
        .await
        .map_err(|error| format!("QQ write task failed: {error}"))?
        .map_err(|error| error.to_string())?;
    Ok(DraftWriteResult { verified })
}

#[derive(Serialize)]
struct CurrentSelection {
    text: String,
    revision: u64,
}

/// Text captured at the last hotkey press plus a monotonic capture revision.
/// The revision changes even when the user selects the same sentence again.
#[tauri::command]
fn current_selection(state: State<AppState>) -> CurrentSelection {
    CurrentSelection {
        text: state
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .map(|snapshot| snapshot.selected_text.clone())
            .unwrap_or_default(),
        revision: state.selection_revision.load(Ordering::Acquire),
    }
}

/// Preview a transformation and its diff without writing anything.
///
/// The model runs on a blocking thread pool (`spawn_blocking`) rather than the
/// Tauri command thread: local CPU decoding takes seconds, and doing it inline
/// froze the whole overlay UI. The winning preview is cached so `apply`
/// can reuse it instead of decoding a second time.
#[tauri::command]
async fn preview_transform(
    state: State<'_, AppState>,
    mode: String,
    text: String,
) -> Result<PreviewResult, String> {
    let settings = state.backend.lock().unwrap().clone();
    let is_model_task = model_task(&mode).is_some();
    let pending = is_model_task && settings.backend == "local" && !assistant_inference::is_ready();
    if pending {
        return Ok(PreviewResult {
            transformed: text.clone(),
            diff: to_segments(diff_chars(&text, &text)),
            warning: None,
            pending: true,
        });
    }

    *state.pet_status.lock().unwrap() = "thinking".into();
    let transformed = {
        let mode = mode.clone();
        let text = text.clone();
        tauri::async_runtime::spawn_blocking(move || transform_text(&settings, &mode, &text))
            .await
            .map_err(|error| format!("preview task failed: {error}"))??
    };
    *state.pet_status.lock().unwrap() = "speaking".into();

    let warning = model_task(&mode)
        .and_then(|task| assistant_inference::quality_warning(task, &text, &transformed));
    let diff = to_segments(diff_chars(&text, &transformed));
    let settings = state.backend.lock().unwrap().clone();
    // Cache this result so a subsequent `apply` for the same mode/source/backend
    // writes back exactly what the user saw, without re-running the model.
    *state.last_preview.lock().unwrap() = Some(CachedPreview {
        mode,
        source: text,
        backend_signature: backend_signature(&settings),
        transformed: transformed.clone(),
    });
    Ok(PreviewResult {
        transformed,
        diff,
        warning,
        pending: false,
    })
}

/// Apply the transformation to the captured selection and write it back.
///
/// This reuses the cached preview whenever it matches the current selection, so
/// it does NOT run the model again: a second inference would add seconds of
/// latency (widening the focus-drift window that rejects the write) and, being
/// temperature-sampled, could even differ from what the user just approved. It
/// only falls back to a fresh (blocking) inference if no matching preview
/// exists.
#[tauri::command]
async fn apply_transform(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let snapshot = state
        .snapshot
        .lock()
        .unwrap()
        .clone()
        .ok_or("no captured selection")?;

    // Fast path: reuse the preview the user actually saw, if it was computed for
    // this exact mode, selection text and model backend configuration.
    let settings = state.backend.lock().unwrap().clone();
    let signature = backend_signature(&settings);
    let cached = state.last_preview.lock().unwrap().clone();
    let new_text = match cached {
        Some(preview)
            if preview.mode == mode
                && preview.source == snapshot.selected_text
                && preview.backend_signature == signature =>
        {
            preview.transformed
        }
        _ => {
            let source = snapshot.selected_text.clone();
            tauri::async_runtime::spawn_blocking(move || transform_text(&settings, &mode, &source))
                .await
                .map_err(|error| format!("apply task failed: {error}"))??
        }
    };

    let adapter = WindowsAdapter::new();
    let receipt = adapter
        .write_back(&snapshot, &new_text)
        .map_err(|e| e.to_string())?;
    *state.last_receipt.lock().unwrap() = Some(receipt);
    Ok(())
}

/// Undo the last successful write.
#[tauri::command]
fn undo_last(state: State<AppState>) -> Result<(), String> {
    let receipt = state
        .last_receipt
        .lock()
        .unwrap()
        .clone()
        .ok_or("nothing to undo")?;
    let adapter = WindowsAdapter::new();
    adapter.undo(&receipt).map_err(|e| e.to_string())?;
    *state.last_receipt.lock().unwrap() = None;
    Ok(())
}

/// Hide the overlay (called by the close button / Esc).
#[tauri::command]
fn hide_overlay(app: AppHandle, state: State<AppState>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    if let Some(pet) = app.get_webview_window("pet") {
        let _ = pet.show();
    }
    *state.pet_status.lock().unwrap() = "idle".into();
}

/// Explicitly begin a native window drag. This custom command bypasses the
/// window-plugin permission path used by `data-tauri-drag-region`, which is
/// unreliable for a non-activating WebView window.
#[tauri::command]
fn start_window_drag(window: WebviewWindow, state: State<AppState>) -> Result<(), String> {
    state.user_positioned.store(true, Ordering::Relaxed);
    window.start_dragging().map_err(|error| error.to_string())
}

/// Begin native south-east resize dragging from the visible corner grip. A
/// WebviewWindow does not expose Tauri's Window-only resize method, so send the
/// standard non-client hit-test message directly to Win32.
#[tauri::command]
fn start_window_resize(window: WebviewWindow) -> Result<(), String> {
    let raw = window.hwnd().map_err(|error| error.to_string())?.0;
    let hwnd = HWND(raw);
    // SAFETY: `hwnd` belongs to this live Tauri window. Releasing the current
    // mouse capture and sending WM_NCLBUTTONDOWN/HTBOTTOMRIGHT delegates the
    // resize loop to Windows exactly like dragging a decorated window corner.
    unsafe {
        let _ = ReleaseCapture();
        let _ = SendMessageW(
            hwnd,
            WM_NCLBUTTONDOWN,
            windows::Win32::Foundation::WPARAM(HTBOTTOMRIGHT as usize),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
    Ok(())
}

/// Continuously remember the last foreground QQ window so the panel's "read"
/// and "write draft" buttons still target QQ after the panel takes focus. This
/// is a cheap Win32-only poll (no COM/UIA), safe to run at a modest cadence.
fn spawn_qq_foreground_sampler() {
    thread::spawn(|| loop {
        remember_foreground_if_qq();
        thread::sleep(Duration::from_millis(500));
    });
}

/// Background hotkey worker: capture on transform, revert on undo.
fn spawn_hotkey_worker(app: AppHandle) {
    thread::spawn(move || {
        let result = run_assistant_hotkey_loop(|command| {
            // Let Ctrl/Alt lift so injected keystrokes stay clean.
            wait_for_trigger_release(Duration::from_millis(800));
            match command {
                AssistantHotkey::Transform => on_transform(&app),
                AssistantHotkey::Undo => on_undo(&app),
                AssistantHotkey::Ime => on_ime(&app),
            }
        });
        if let Err(error) = result {
            eprintln!("hotkey worker stopped: {error}");
        }
    });
}

fn on_transform(app: &AppHandle) {
    let adapter = WindowsAdapter::new();
    match adapter.capture_selection() {
        Ok(snapshot) => {
            let state = app.state::<AppState>();
            *state.snapshot.lock().unwrap() = Some(snapshot);
            state.selection_revision.fetch_add(1, Ordering::AcqRel);
            *state.last_preview.lock().unwrap() = None;
            if let Some(pet) = app.get_webview_window("pet") {
                let _ = pet.hide();
            }
            if let Some(window) = app.get_webview_window("main") {
                // Respect a position chosen by dragging. Before the first drag,
                // place the panel near the selection/cursor automatically.
                if !state.user_positioned.load(Ordering::Relaxed) {
                    position_overlay(&window);
                }
                let _ = window.show();
            }
        }
        Err(error) => eprintln!("capture failed: {error}"),
    }
}

/// Position the overlay near the cursor, fully inside the work area of the
/// monitor under the cursor. Config `center: true` is unreliable for a
/// transparent, borderless window (its size isn't settled at creation), so we
/// place it explicitly on every show. Win32 (physical px), the monitor work
/// area, and Tauri's PhysicalPosition all agree under per-monitor DPI.
fn position_overlay(window: &WebviewWindow) {
    let mut cursor = POINT::default();
    // SAFETY: GetCursorPos writes the current cursor position into `cursor`.
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        let _ = window.center();
        return;
    }

    // SAFETY: MonitorFromPoint always returns a valid monitor with NEAREST.
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info.cbSize` is set as required by GetMonitorInfoW.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        let _ = window.center();
        return;
    }

    let work = info.rcWork;
    let size = window.outer_size().unwrap_or(PhysicalSize::new(520, 520));
    let w = size.width as i32;
    let h = size.height as i32;

    // Offset a little from the caret so the panel doesn't cover it, then clamp
    // the whole window inside the work area.
    let x = (cursor.x + 24).min(work.right - w).max(work.left);
    let y = (cursor.y + 24).min(work.bottom - h).max(work.top);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn position_pet(window: &WebviewWindow) {
    // Place the whole pet inside the monitor work area (excluding taskbar).
    // `current_monitor().size()` includes the taskbar and could leave the lower
    // torso clipped, especially under Windows display scaling.
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return;
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return;
    }
    let work = info.rcWork;
    let size = window.outer_size().unwrap_or(PhysicalSize::new(220, 260));
    let x = (work.right - size.width as i32 - 24).max(work.left);
    let y = (work.bottom - size.height as i32 - 24).max(work.top);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn on_undo(app: &AppHandle) {
    let state = app.state::<AppState>();
    let receipt = state.last_receipt.lock().unwrap().clone();
    if let Some(receipt) = receipt {
        let adapter = WindowsAdapter::new();
        let _ = adapter.undo(&receipt);
        *state.last_receipt.lock().unwrap() = None;
    }
}

/// Toggle the overlay's `WS_EX_NOACTIVATE` extended style and Tauri focusable
/// flag together. The non-activating state is the default: mouse clicks and drag
/// gestures still work, but keyboard/UIA focus stays in the source editor so
/// snapshot validation and write-back remain safe. It is temporarily lifted when
/// a view needs real text entry (see `set_panel_focusable`).
fn set_window_activating(window: &WebviewWindow, activating: bool) -> Result<(), String> {
    window
        .set_focusable(activating)
        .map_err(|error| error.to_string())?;
    let tauri_hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let hwnd = HWND(tauri_hwnd.0);
    // SAFETY: `hwnd` belongs to this process and remains valid for the window's
    // lifetime. We preserve every existing extended style and flip one flag.
    unsafe {
        let styles = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let updated = if activating {
            styles & !(WS_EX_NOACTIVATE.0 as isize)
        } else {
            styles | WS_EX_NOACTIVATE.0 as isize
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, updated);
    }
    Ok(())
}

/// Prevent mouse interaction with the overlay from activating it. See
/// `set_window_activating` for the rationale.
fn make_non_activating(window: &WebviewWindow) -> Result<(), String> {
    set_window_activating(window, false)
}

/// Let the panel accept keyboard focus so text fields (API key, QQ draft) can be
/// typed into, or restore the non-activating default. Views that require typing
/// call this with `true` on entry and `false` when leaving; the write-back path
/// therefore keeps running against a non-activating window as before.
#[tauri::command]
fn set_panel_focusable(app: AppHandle, focusable: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("panel window is unavailable")?;
    set_window_activating(&window, focusable)?;
    if focusable {
        // Actually pull focus to the window; otherwise the freshly focusable
        // window still has no keyboard focus and the caret never appears.
        window.set_focus().map_err(|error| error.to_string())?;
    }
    Ok(())
}

// ─── IME candidate panel commands ────────────────────────────────────────────

#[derive(Serialize)]
struct ImeCandidateView {
    text: String,
    syllables: String,
    score: f64,
}

/// Return ranked candidates for a raw pinyin string (called by the IME panel as
/// the user types). The engine is lightweight and returns in microseconds.
#[tauri::command]
fn ime_candidates(pinyin: String, context: String, limit: Option<usize>) -> Vec<ImeCandidateView> {
    use assistant_ime::{InputContext, InputEngine, PinyinInputEngine};
    let engine = PinyinInputEngine::builtin();
    let ctx = InputContext {
        preceding_text: context,
        max_candidates: limit.unwrap_or(9),
    };
    engine
        .candidates(&pinyin, &ctx)
        .into_iter()
        .map(|c| ImeCandidateView {
            text: c.text,
            syllables: c.syllables.join(" "),
            score: c.score,
        })
        .collect()
}

/// Commit a chosen candidate: write it into the target application via the
/// existing controlled-paste pipeline, then hide the IME panel. The snapshot is
/// taken at hotkey-press time (stored in `AppState.snapshot`), so focus drift is
/// still detected.
#[tauri::command]
async fn ime_commit(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<(), String> {
    // Build a minimal snapshot: the foreground at IME-hotkey time.
    let snapshot = state
        .snapshot
        .lock()
        .unwrap()
        .clone()
        .ok_or("no target captured (press Ctrl+Alt+I from a text field)")?;
    let adapter = WindowsAdapter::new();
    let receipt = adapter
        .write_back(&snapshot, &text)
        .map_err(|e| e.to_string())?;
    *state.last_receipt.lock().unwrap() = Some(receipt);
    // Hide IME panel after commit.
    if let Some(ime_win) = app.get_webview_window("ime") {
        let _ = ime_win.hide();
    }
    Ok(())
}

fn on_ime(app: &AppHandle) {
    // Capture current selection/target for write-back after candidate commit.
    let adapter = WindowsAdapter::new();
    match adapter.capture_selection() {
        Ok(snapshot) => {
            let state = app.state::<AppState>();
            *state.snapshot.lock().unwrap() = Some(snapshot);
            state.selection_revision.fetch_add(1, Ordering::AcqRel);
        }
        Err(error) => {
            eprintln!("IME capture failed (will use last known target): {error}");
        }
    }
    // Show and focus the IME panel.
    if let Some(ime_win) = app.get_webview_window("ime") {
        position_overlay(&ime_win);
        let _ = ime_win.show();
        let _ = ime_win.set_focus();
    }
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            current_selection,
            preview_transform,
            apply_transform,
            undo_last,
            hide_overlay,
            start_window_drag,
            start_window_resize,
            get_backend_settings,
            save_backend_settings,
            model_progress,
            pet_status,
            set_pet_status,
            toggle_panel,
            set_panel_focusable,
            qq_poll_latest,
            generate_qq_draft,
            write_qq_draft,
            ime_candidates,
            ime_commit
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                make_non_activating(&window).map_err(std::io::Error::other)?;
            }
            if let Some(window) = app.get_webview_window("pet") {
                make_non_activating(&window).map_err(std::io::Error::other)?;
                position_pet(&window);
            }
            // Only local users need the GGUF. A persisted cloud configuration
            // must not trigger a needless ~400MB download at startup.
            if app.state::<AppState>().backend.lock().unwrap().backend == "local" {
                assistant_inference::prepare_in_background();
            }
            spawn_hotkey_worker(app.handle().clone());
            spawn_qq_foreground_sampler();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to launch LingXi overlay");
}
