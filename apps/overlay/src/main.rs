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
use assistant_inference::{CloudBackend, CloudConfig, LocalBackend, ModelBackend, ModelTask};
use assistant_windows::{
    foreground_info, qq_write_draft, remember_foreground_if_qq, run_assistant_hotkey_loop,
    wait_for_trigger_release, AssistantHotkey, WindowsAdapter,
};
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow,
};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, SendMessageW, SetWindowLongPtrW, GWL_EXSTYLE, HTBOTTOMRIGHT,
    WM_NCLBUTTONDOWN, WS_EX_NOACTIVATE,
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
    #[allow(dead_code)]
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
    format!(
        "{}|{}|{}",
        settings.backend, settings.endpoint, settings.model
    )
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

/// Check whether QQ is the foreground window, and if so return the active
/// conversation title (the chat partner's display name). The caller (UI) then
/// asks the user to select the message they want to reply to and calls
/// `capture_qq_selection` to read it via the standard Win32 selection API.
///
/// This intentionally does NOT scan QQ's UIA tree: QQNT (Chromium / Electron)
/// does not expose sender identity per bubble, so any "auto-read latest
/// message" attempt ends up mixing own messages, the contact list preview,
/// and group member rosters. Reading the user's explicit selection is the
/// only reliable source of truth for "which message to reply to".
#[tauri::command]
async fn qq_poll_latest(_state: State<'_, AppState>) -> Result<Option<QqMessageView>, String> {
    let info = tauri::async_runtime::spawn_blocking(foreground_info)
        .await
        .map_err(|e| format!("foreground probe failed: {e}"))?;
    let info = match info {
        Ok(info) if is_qq_foreground(&info.process_name) => info,
        _ => return Ok(None),
    };
    let conversation = info.title.clone();
    Ok(Some(QqMessageView {
        conversation,
        message: String::new(),
        is_new: false,
    }))
}

/// Capture the user's current text selection inside QQ. The user must select
/// the message they want to reply to (double-click or drag-select) before
/// calling this. Returns the selected text verbatim.
#[tauri::command]
async fn capture_qq_selection(_state: State<'_, AppState>) -> Result<QqMessageView, String> {
    let snapshot = tauri::async_runtime::spawn_blocking(|| {
        let adapter = WindowsAdapter::new();
        adapter.capture_selection()
    })
    .await
    .map_err(|e| format!("selection capture failed: {e}"))?
    .map_err(|e| format!("selection capture: {e}"))?;
    let message = snapshot.selected_text.trim().to_string();
    if message.is_empty() {
        return Err("没有选中文字。请在 QQ 里双击或拖选对方的消息后再试。".into());
    }
    let info = foreground_info().map_err(|e| format!("foreground probe: {e}"))?;
    Ok(QqMessageView {
        conversation: info.title,
        message,
        is_new: true,
    })
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

/// Quit the entire LingXi process (called by the "退出灵犀" button).
/// Unlike `hide_overlay` which only hides the panel, this fully exits the
/// application so the user does not need to find the hidden tray icon or use
/// Task Manager.
#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
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
/// Whether a process image name is a QQ client (QQ.exe or QQNT.exe).
fn is_qq_foreground(process_name: &str) -> bool {
    let p = process_name.to_ascii_lowercase();
    p == "qq.exe" || p == "qqnt.exe"
}

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

/* IME mode removed: OwO is the only system input method. Keeping LingXi's former
WH_KEYBOARD_LL implementation here disabled would still risk accidental reactivation, so the
entire implementation is excluded and will be deleted after the migration remains stable. */
#[cfg(any())]
mod removed_ime_hook {
    use super::*;
    use std::sync::Arc;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW as GetMsg, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION,
        KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
    };

    /// Shared IME state between the hook thread, Tauri commands, and the frontend.
    #[derive(Debug, Clone, Serialize)]
    struct ImeStateView {
        active: bool,
        pinyin: String,
        candidates: Vec<ImeCandidateView>,
        /// `large` when connected to ime-server+rime-ice, `basic` on fallback.
        backend: &'static str,
    }

    #[derive(Debug, Serialize, Clone)]
    struct ImeCandidateView {
        text: String,
        score: f64,
    }

    struct ImeShared {
        active: bool,
        pinyin: String,
        candidates: Vec<ImeCandidateView>,
        committed_context: String,
        /// Monotonic input revision. The worker computes candidates outside the
        /// hook and only publishes them if this revision is still current.
        revision: u64,
        /// Candidate selected by Space/Enter/1-9/click. The worker takes this and
        /// performs the clipboard paste outside the low-level hook callback.
        pending_commit: Option<String>,
        /// Selection pressed before the async candidate response arrived. The
        /// worker applies it as soon as the matching revision is published.
        pending_selection: Option<usize>,
        /// Whether the last candidate query came from ime-server+rime-ice.
        server_connected: bool,
    }

    impl Default for ImeShared {
        fn default() -> Self {
            Self {
                // Launch directly in Chinese input mode. Ctrl+Alt+I remains an
                // explicit toggle, but the first run no longer leaks raw pinyin.
                active: true,
                pinyin: String::new(),
                candidates: Vec::new(),
                committed_context: String::new(),
                revision: 0,
                pending_commit: None,
                pending_selection: None,
                server_connected: false,
            }
        }
    }

    static IME: std::sync::OnceLock<Arc<Mutex<ImeShared>>> = std::sync::OnceLock::new();

    fn ime_shared() -> &'static Arc<Mutex<ImeShared>> {
        IME.get_or_init(|| Arc::new(Mutex::new(ImeShared::default())))
    }

    /// Poll IME state (called by the frontend every ~30ms).
    #[tauri::command]
    fn ime_state() -> ImeStateView {
        let s = ime_shared().lock().unwrap();
        ImeStateView {
            active: s.active,
            pinyin: s.pinyin.clone(),
            candidates: s.candidates.clone(),
            backend: if s.server_connected { "large" } else { "basic" },
        }
    }

    /// Queue a candidate chosen by mouse. Keyboard choices use the same queue from
    /// the hook; the worker performs the actual paste without stealing focus.
    #[tauri::command]
    fn ime_commit(index: usize) -> Result<(), String> {
        let mut s = ime_shared().lock().unwrap();
        let text = s
            .candidates
            .get(index)
            .map(|candidate| candidate.text.clone())
            .ok_or("invalid candidate index")?;
        s.committed_context.push_str(&text);
        s.pending_commit = Some(text);
        s.pinyin.clear();
        s.candidates.clear();
        s.revision = s.revision.wrapping_add(1);
        Ok(())
    }

    /// Toggle IME mode on/off.
    #[tauri::command]
    fn ime_toggle() -> bool {
        let mut s = ime_shared().lock().unwrap();
        s.active = !s.active;
        if !s.active {
            s.pinyin.clear();
            s.candidates.clear();
            s.committed_context.clear();
            s.pending_commit = None;
            s.pending_selection = None;
        }
        s.revision = s.revision.wrapping_add(1);
        s.active
    }

    /// Compute candidates outside the keyboard hook. The caller publishes the
    /// result only when the pinyin revision is still current.
    fn compute_candidates(pinyin: &str, context: &str) -> (Vec<ImeCandidateView>, bool) {
        if let Some(results) = ipc_query(pinyin, context, 9) {
            return (results, true);
        }
        use assistant_ime::{InputContext, InputEngine, PinyinInputEngine};
        let engine = PinyinInputEngine::builtin();
        let ctx = InputContext {
            preceding_text: context.to_string(),
            max_candidates: 9,
        };
        let candidates = engine
            .candidates(pinyin, &ctx)
            .into_iter()
            .map(|candidate| ImeCandidateView {
                text: candidate.text,
                score: candidate.score,
            })
            .collect();
        (candidates, false)
    }

    /// TCP call to the ime-server.
    fn ipc_query(pinyin: &str, context: &str, limit: usize) -> Option<Vec<ImeCandidateView>> {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpStream;

        let mut stream = TcpStream::connect_timeout(
            &"127.0.0.1:9527".parse().unwrap(),
            std::time::Duration::from_millis(100),
        )
        .ok()?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .ok()?;

        let request = format!(
            "{{\"type\":\"query\",\"pinyin\":\"{}\",\"context\":\"{}\",\"limit\":{}}}\n",
            pinyin.replace('\\', "\\\\").replace('"', "\\\""),
            context.replace('\\', "\\\\").replace('"', "\\\""),
            limit
        );
        stream.write_all(request.as_bytes()).ok()?;
        stream.flush().ok()?;

        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;

        let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        let arr = value.get("candidates")?.as_array()?;
        let results = arr
            .iter()
            .filter_map(|item| {
                Some(ImeCandidateView {
                    text: item.get("text")?.as_str()?.to_string(),
                    score: item.get("score")?.as_f64().unwrap_or(0.0),
                })
            })
            .collect();
        Some(results)
    }

    /// Spawn the global low-level keyboard hook thread.
    fn spawn_ime_hook_thread() {
        thread::spawn(move || {
            unsafe {
                let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ime_hook_proc), None, 0)
                    .expect("install keyboard hook");

                // Must pump messages to keep the hook alive.
                let mut msg = MSG::default();
                while GetMsg(&mut msg, None, 0, 0).0 > 0 {}

                let _ = UnhookWindowsHookEx(hook);
            }
        });
    }

    /// Slow IME worker: resolves candidates, commits text, and manages the panel.
    /// Keeping all of this out of `ime_hook_proc` prevents Windows hook timeouts.
    fn spawn_ime_worker(app: AppHandle) {
        let shared = ime_shared().clone();
        thread::spawn(move || {
            let mut seen_revision = u64::MAX;
            let mut visible = false;
            loop {
                let pending = shared.lock().unwrap().pending_commit.take();
                if let Some(text) = pending {
                    if let Err(error) = insert_text_at_caret(&text) {
                        eprintln!("IME commit failed: {error}");
                    }
                }

                let (active, revision, pinyin, context) = {
                    let s = shared.lock().unwrap();
                    (
                        s.active,
                        s.revision,
                        s.pinyin.clone(),
                        s.committed_context.clone(),
                    )
                };

                if !active || pinyin.is_empty() {
                    if visible {
                        if let Some(window) = app.get_webview_window("ime") {
                            let _ = window.hide();
                        }
                        visible = false;
                    }
                    seen_revision = revision;
                } else if revision != seen_revision {
                    let (candidates, server_connected) = compute_candidates(&pinyin, &context);
                    let mut s = shared.lock().unwrap();
                    // Discard a stale server response if another key arrived.
                    if s.active && s.revision == revision && s.pinyin == pinyin {
                        s.candidates = candidates;
                        s.server_connected = server_connected;
                        let queued = if let Some(index) = s.pending_selection.take() {
                            queue_candidate(&mut s, index);
                            s.pending_commit.is_some()
                        } else {
                            false
                        };
                        seen_revision = revision;
                        drop(s);
                        if !queued {
                            if let Some(window) = app.get_webview_window("ime") {
                                position_ime_window(&window);
                                let _ = window.show();
                                visible = true;
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
    }

    /// Position at the real application caret when available, otherwise at cursor.
    fn position_ime_window(window: &WebviewWindow) {
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};

        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let mut point = POINT::default();
        let caret_found =
            unsafe { GetGUIThreadInfo(0, &mut info) }.is_ok() && !info.hwndCaret.is_invalid() && {
                point.x = info.rcCaret.left;
                point.y = info.rcCaret.bottom;
                unsafe { ClientToScreen(info.hwndCaret, &mut point) }.as_bool()
            };
        if !caret_found && unsafe { GetCursorPos(&mut point) }.is_err() {
            return;
        }

        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return;
        }
        let work = info.rcWork;
        let size = window.outer_size().unwrap_or(PhysicalSize::new(720, 92));
        let x = point.x.min(work.right - size.width as i32).max(work.left);
        let y = (point.y + 8)
            .min(work.bottom - size.height as i32)
            .max(work.top);
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }

    /// The low-level hook callback. It must return quickly: doing TCP or clipboard
    /// work here makes Windows time out/remove the hook and leaks letters into the
    /// target. This function only updates state; the worker does all slow work.
    unsafe extern "system" fn ime_hook_proc(
        code: i32,
        wparam: windows::Win32::Foundation::WPARAM,
        lparam: windows::Win32::Foundation::LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        if code as u32 != HC_ACTION {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // Never intercept SendInput generated by our controlled paste; otherwise
        // the injected Ctrl+V would become another pinyin `v`.
        if kb.flags.0 & 0x10 != 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        // Preserve Ctrl/Alt/Win shortcuts, including Ctrl+Alt+I which toggles mode.
        let modifier_down = GetAsyncKeyState(VK_CONTROL.0 as i32) < 0
            || GetAsyncKeyState(VK_MENU.0 as i32) < 0
            || GetAsyncKeyState(VK_LWIN.0 as i32) < 0
            || GetAsyncKeyState(VK_RWIN.0 as i32) < 0;
        if modifier_down {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let shared = ime_shared();
        if !shared.lock().unwrap().active || wparam.0 != 0x0100 {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let vk = kb.vkCode as u16;
        let handled = {
            let mut s = shared.lock().unwrap();
            if (0x41..=0x5A).contains(&vk) {
                s.pinyin.push((vk as u8 as char).to_ascii_lowercase());
                s.candidates.clear();
                s.pending_selection = None;
                s.revision = s.revision.wrapping_add(1);
                true
            } else if vk == VK_BACK.0 && !s.pinyin.is_empty() {
                s.pinyin.pop();
                s.candidates.clear();
                s.pending_selection = None;
                s.revision = s.revision.wrapping_add(1);
                true
            } else if vk == VK_ESCAPE.0 {
                // Escape always exits IME mode, even with an active composition.
                s.active = false;
                s.pinyin.clear();
                s.candidates.clear();
                s.committed_context.clear();
                s.pending_commit = None;
                s.pending_selection = None;
                s.revision = s.revision.wrapping_add(1);
                true
            } else if (vk == VK_SPACE.0 || vk == VK_RETURN.0) && !s.pinyin.is_empty() {
                if s.candidates.is_empty() {
                    s.pending_selection = Some(0);
                } else {
                    queue_candidate(&mut s, 0);
                }
                true
            } else if (0x31..=0x39).contains(&vk) && !s.pinyin.is_empty() {
                let index = (vk - 0x31) as usize;
                if s.candidates.is_empty() {
                    s.pending_selection = Some(index);
                } else {
                    queue_candidate(&mut s, index);
                }
                true
            } else {
                // Only eat non-letter keys while composing; ordinary keys pass
                // through when the buffer is empty.
                !s.pinyin.is_empty()
            }
        };

        if handled {
            windows::Win32::Foundation::LRESULT(1)
        } else {
            CallNextHookEx(None, code, wparam, lparam)
        }
    }

    fn queue_candidate(state: &mut ImeShared, index: usize) {
        if let Some(candidate) = state.candidates.get(index).cloned() {
            state.committed_context.push_str(&candidate.text);
            state.pending_commit = Some(candidate.text);
            state.pinyin.clear();
            state.candidates.clear();
            state.revision = state.revision.wrapping_add(1);
        }
    }

    fn on_ime(app: &AppHandle) {
        let active = {
            let mut s = ime_shared().lock().unwrap();
            s.active = !s.active;
            s.pinyin.clear();
            s.candidates.clear();
            s.pending_commit = None;
            s.pending_selection = None;
            if !s.active {
                s.committed_context.clear();
            }
            s.revision = s.revision.wrapping_add(1);
            s.active
        };
        // Activating mode does not show an empty panel; the worker shows it on the
        // first letter. Deactivation hides immediately.
        if !active {
            if let Some(window) = app.get_webview_window("ime") {
                let _ = window.hide();
            }
        }
    }
}

/// Install a system tray icon with show / hide / quit actions. Until this
/// lands the user had to kill `overlay.exe` from Task Manager to close the
/// LingXi panel, which is unfriendly for a personal tool that lives in the
/// background. The tray icon reuses the bundled window icon so there is no
/// extra asset to ship.
fn install_tray(app: &AppHandle) -> tauri::Result<()> {
    eprintln!("[lingxi] install_tray: creating menu...");
    let show_panel = MenuItem::with_id(app, "tray:show", "显示面板", true, None::<&str>)?;
    let hide_panel = MenuItem::with_id(app, "tray:hide", "隐藏面板", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray:quit", "退出灵犀", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show_panel, &hide_panel, &separator, &quit])?;
    eprintln!("[lingxi] install_tray: menu created, getting icon...");
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;
    eprintln!("[lingxi] install_tray: icon ok, building tray...");
    let _tray = TrayIconBuilder::with_id("lingxi-tray")
        .icon(icon)
        .tooltip("灵犀 · L3 跨应用 AI 助手")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray:show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                }
            }
            "tray:hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "tray:quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                    }
                }
            }
        })
        .build(app)?;
    eprintln!("[lingxi] install_tray: tray built successfully");
    Ok(())
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
            capture_qq_selection,
            generate_qq_draft,
            write_qq_draft,
            quit_app
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
            install_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to launch LingXi overlay");
}
