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

mod secret_store;
mod widgets;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use assistant_core::{
    diff_chars, transformer_by_name, DiffOp, InputAdapter, SelectionSnapshot, WriteReceipt,
};
use assistant_inference::{
    CloudAgentBackend, CloudBackend, CloudConfig, LocalBackend, ModelBackend, ModelTask,
};
use assistant_windows::{
    capture_qq_selection_text, qq_write_draft, remember_foreground_if_qq, resolve_qq_window,
    run_assistant_hotkey_loop, wait_for_trigger_release, AssistantHotkey, WindowsAdapter,
};
use lingxi_agent::{AgentBackend, AgentEngine, AgentRunReport, Session};
use lingxi_tools::{ConfirmGate, DenyAll, RiskLevel, ToolRegistry};
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, SubmenuBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow,
};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, ReleaseCapture, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_O,
    VK_T, VK_C, VK_V, VK_OEM_PLUS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetMessageW, GetWindowLongPtrW, SendMessageW, SetWindowLongPtrW, GWL_EXSTYLE,
    HTBOTTOMRIGHT, MSG, WM_HOTKEY, WM_NCLBUTTONDOWN, WS_EX_NOACTIVATE,
};

/// Extension trait so every `Mutex::lock().unwrap()` in the app recovers from
/// poisoning instead of panicking. If one thread panics while holding a lock,
/// the Mutex becomes "poisoned" and subsequent `.safe_lock()` calls would
/// cascade-panic — making the entire overlay unusable after a single error.
/// Recovering the inner data keeps the app running with the last known state.
trait MutexExt<T> {
    fn safe_lock(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn safe_lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

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
    /// Whether the key is persisted using Windows DPAPI for this user.
    remember_api_key: bool,
}

impl Default for BackendSettings {
    fn default() -> Self {
        Self {
            backend: "local".into(),
            endpoint: "https://api.openai.com".into(),
            model: "gpt-4o-mini".into(),
            api_key: std::env::var("LINGXI_OPENAI_API_KEY").unwrap_or_default(),
            remember_api_key: false,
        }
    }
}

#[derive(Serialize)]
struct BackendSettingsView {
    backend: String,
    endpoint: String,
    model: String,
    api_key_configured: bool,
    remember_api_key: bool,
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
    #[serde(alias = "rememberApiKey")]
    remember_api_key: bool,
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
    /// Agent tool registry, initialized with all Windows tools at startup.
    tool_registry: std::sync::Mutex<ToolRegistry>,
    /// Agent conversation session, persists across messages within a session.
    agent_session: std::sync::Mutex<Session>,
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
            tool_registry: std::sync::Mutex::new({
                let mut reg = ToolRegistry::new();
                lingxi_tools_windows::register_default_tools(&mut reg);
                reg
            }),
            agent_session: std::sync::Mutex::new(load_agent_session()),
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
    let env_key = std::env::var("LINGXI_OPENAI_API_KEY").unwrap_or_default();
    if !env_key.is_empty() {
        settings.api_key = env_key;
    } else if settings.remember_api_key {
        match secret_store::load_api_key() {
            Ok(Some(key)) => settings.api_key = key,
            Ok(None) | Err(_) => {
                settings.api_key.clear();
                settings.remember_api_key = false;
            }
        }
    }
    // Keep the user's selected backend even when the credential is missing;
    // the UI can now explain the exact problem instead of silently switching
    // backends and making Agent behavior appear inconsistent.
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

fn default_agent_working_dir() -> std::path::PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default()
}

fn agent_session_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lingxi").join("agent-session.json"))
}

fn load_agent_session() -> Session {
    let mut session = agent_session_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Session>(&bytes).ok())
        .unwrap_or_else(|| {
            Session::new(
                uuid::Uuid::new_v4().to_string(),
                default_agent_working_dir(),
            )
        });
    if !session.working_dir.is_dir() {
        session.working_dir = default_agent_working_dir();
    }
    session.trim_history(40);
    session
}

fn persist_agent_session(session: &Session) -> Result<(), String> {
    let path = agent_session_path().ok_or("无法解析灵犀配置目录")?;
    let parent = path.parent().ok_or("无法解析会话目录")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let json = serde_json::to_vec_pretty(session).map_err(|error| error.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, json).map_err(|error| error.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_backend_settings(state: State<AppState>) -> BackendSettingsView {
    let settings = state.backend.safe_lock();
    BackendSettingsView {
        backend: settings.backend.clone(),
        endpoint: settings.endpoint.clone(),
        model: settings.model.clone(),
        api_key_configured: !settings.api_key.is_empty(),
        remember_api_key: settings.remember_api_key,
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
    let mut settings = state.backend.safe_lock();
    settings.backend = input.backend;
    settings.endpoint = input.endpoint.trim().trim_end_matches('/').to_string();
    settings.model = input.model.trim().to_string();
    // An empty field means "keep the current session key"; the frontend never
    // receives the secret back from this process.
    if !input.api_key.trim().is_empty() {
        settings.api_key = input.api_key.trim().to_string();
    }
    if settings.backend == "cloud" && settings.api_key.is_empty() {
        return Err("云端模型需要 API Key".into());
    }
    if input.remember_api_key {
        if settings.api_key.is_empty() {
            return Err("请先填写 API Key 再启用安全记忆".into());
        }
        secret_store::save_api_key(&settings.api_key)?;
    } else {
        secret_store::delete_api_key()?;
    }
    settings.remember_api_key = input.remember_api_key;
    persist_backend_settings(&settings)?;
    let start_local = settings.backend == "local" && !assistant_inference::is_ready();
    let view = BackendSettingsView {
        backend: settings.backend.clone(),
        endpoint: settings.endpoint.clone(),
        model: settings.model.clone(),
        api_key_configured: !settings.api_key.is_empty(),
        remember_api_key: settings.remember_api_key,
    };
    drop(settings);
    *state.last_preview.safe_lock() = None;
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
    state.pet_status.safe_lock().clone()
}

#[tauri::command]
fn set_pet_status(state: State<AppState>, status: String) -> Result<(), String> {
    if !matches!(status.as_str(), "idle" | "thinking" | "speaking" | "alert") {
        return Err("invalid pet status".into());
    }
    *state.pet_status.safe_lock() = status;
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
    let info = tauri::async_runtime::spawn_blocking(resolve_qq_window)
        .await
        .map_err(|e| format!("foreground probe failed: {e}"))?;
    // `resolve_qq_window` already falls back to the last remembered QQ window
    // (recorded by the background sampler every 500ms) when the live foreground
    // is something else — typically this LingXi panel after the user clicked a
    // button. Only when neither is available do we tell the UI QQ isn't around.
    match info {
        Ok(info) => Ok(Some(QqMessageView {
            conversation: info.title,
            message: String::new(),
            is_new: false,
        })),
        Err(_) => Ok(None),
    }
}

/// Capture the user's current text selection inside QQ. The user must select
/// the message they want to reply to (double-click or drag-select) before
/// calling this. Returns the selected text verbatim.
///
/// Unlike `capture_selection` (which needs QQ in the foreground), this scans
/// the remembered QQ window's UIA tree for a TextPattern element with a
/// non-empty selection — so the DOM selection survives even after the panel
/// steals keyboard focus.
#[tauri::command]
async fn capture_qq_selection(_state: State<'_, AppState>) -> Result<QqMessageView, String> {
    let (info, message) = tauri::async_runtime::spawn_blocking(|| {
        let info = resolve_qq_window();
        let text = capture_qq_selection_text();
        (info, text)
    })
    .await
    .map_err(|e| format!("selection capture failed: {e}"))?;

    let message = message.map_err(|e| format!("selection capture: {e}"))?;
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("没有选中文字。请在 QQ 里双击或拖选对方的消息后再试。".into());
    }
    let conversation = match info {
        Ok(i) => i.title,
        Err(_) => String::new(),
    };
    Ok(QqMessageView {
        conversation,
        message,
        is_new: true,
    })
}

#[tauri::command]
async fn generate_qq_draft(state: State<'_, AppState>, message: String) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("没有可回复的 QQ 消息".into());
    }
    let settings = state.backend.safe_lock().clone();
    *state.pet_status.safe_lock() = "thinking".into();
    // Flatten JoinError and model error into one Result so the match below
    // resets pet_status on every failure path (including task panic).
    let draft = tauri::async_runtime::spawn_blocking(move || {
        run_model(&settings, ModelTask::ChatReply, &message)
    })
    .await
    .map_err(|error| format!("draft task failed: {error}"))
    .and_then(|inner| inner);
    match draft {
        Ok(draft) => {
            *state.pet_status.safe_lock() = "speaking".into();
            Ok(draft)
        }
        Err(error) => {
            *state.pet_status.safe_lock() = "idle".into();
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

// ---------------------------------------------------------------------------
// Agent commands
// ---------------------------------------------------------------------------

/// A tool's metadata for the frontend.
#[derive(Serialize)]
struct ToolView {
    name: String,
    description: String,
    risk_level: String,
    enabled: bool,
}

/// Send a message to the agent and get a reply. The agent may call tools
/// internally before replying. Requires a configured cloud backend.
#[tauri::command]
async fn agent_chat(
    state: State<'_, AppState>,
    message: String,
) -> Result<AgentRunReport, String> {
    let settings = state.backend.safe_lock().clone();
    if settings.backend != "cloud" {
        return Err("Agent 对话暂需云端模型，请在模型设置中切换到 OpenAI 兼容云端。".into());
    }
    if settings.endpoint.is_empty() || settings.api_key.is_empty() {
        return Err("请先在设置页配置云端模型的 Endpoint 和 API Key。".into());
    }

    let config = CloudConfig {
        endpoint: settings.endpoint.clone(),
        model: settings.model.clone(),
        api_key: settings.api_key.clone(),
    };
    let backend = CloudAgentBackend::new(config);
    let backend_box: Box<dyn AgentBackend> = Box::new(backend);

    // Build a fresh registry for this run — tools are stateless, so this is
    // cheap. We copy the user's enabled/disabled state from the stored registry.
    let mut registry = ToolRegistry::new();
    lingxi_tools_windows::register_default_tools(&mut registry);
    {
        let reg = state.tool_registry.safe_lock();
        for schema in reg.all_schemas() {
            if !reg.is_enabled(&schema.name) {
                registry.set_enabled(&schema.name, false);
            }
        }
    }

    let engine = AgentEngine::new(backend_box, std::sync::Arc::new(registry));
    // Dangerous tools are denied until the overlay gains a per-invocation
    // approval flow. They are also disabled in the default registry, providing
    // defense in depth against accidental shell/file mutations.
    let confirm = std::sync::Arc::new(DenyAll) as std::sync::Arc<dyn ConfirmGate>;

    // Clone the session so we never lose the original if the future is
    // cancelled or panics. A cheap deep copy is far safer than `mem::take`,
    // which would replace the Mutex contents with `Session::default()` and
    // lose all user history if the awaited future is dropped.
    let mut session = state.agent_session.safe_lock().clone();
    let result = engine.run_with_trace(&message, &mut session, confirm).await;
    // Persist and restore the session regardless of model success so the user
    // does not lose prior turns after a transient network error.
    let persist_result = persist_agent_session(&session);
    // Only write back on success to avoid clobbering good history with a
    // partial/corrupted session from a failed run.
    if result.is_ok() {
        *state.agent_session.safe_lock() = session;
    }

    let report = result.map_err(|e| e.to_string())?;
    if let Err(e) = persist_result {
        eprintln!("[lingxi] warning: failed to persist agent session: {e}");
    }
    Ok(report)
}

#[derive(Serialize)]
struct AgentHistoryItem {
    role: String,
    content: String,
}

/// Return user-visible messages from the persisted Agent conversation.
#[tauri::command]
fn agent_history(state: State<AppState>) -> Vec<AgentHistoryItem> {
    use lingxi_agent::Role;

    state
        .agent_session
        .lock()
        .unwrap()
        .messages
        .iter()
        .filter_map(|message| {
            let role = match message.role {
                Role::User => "user",
                Role::Assistant if message.tool_calls.is_empty() => "assistant",
                _ => return None,
            };
            (!message.content.trim().is_empty()).then(|| AgentHistoryItem {
                role: role.to_string(),
                content: message.content.clone(),
            })
        })
        .collect()
}

/// Reset the agent conversation session (start a new chat).
#[tauri::command]
fn agent_reset(state: State<AppState>) -> Result<(), String> {
    let session = Session::new(
        uuid::Uuid::new_v4().to_string(),
        default_agent_working_dir(),
    );
    persist_agent_session(&session)?;
    *state.agent_session.safe_lock() = session;
    Ok(())
}

/// List all registered tools with their metadata.
#[tauri::command]
fn list_tools(state: State<AppState>) -> Vec<ToolView> {
    let reg = state.tool_registry.safe_lock();
    reg.all_schemas()
        .iter()
        .map(|s| {
            let risk = reg.risk_of(&s.name).unwrap_or(RiskLevel::Safe);
            ToolView {
                name: s.name.clone(),
                description: s.description.clone(),
                risk_level: format!("{:?}", risk).to_lowercase(),
                enabled: reg.is_enabled(&s.name),
            }
        })
        .collect()
}

/// Enable or disable a tool by name.
#[tauri::command]
fn toggle_tool(state: State<AppState>, name: String, enabled: bool) -> Result<(), String> {
    let mut reg = state.tool_registry.safe_lock();
    let risk = reg
        .risk_of(&name)
        .ok_or_else(|| format!("未知工具: {name}"))?;
    if enabled && risk == RiskLevel::Dangerous {
        return Err("危险工具需要逐次确认，当前安全模式下不可启用。".into());
    }
    reg.set_enabled(&name, enabled);
    Ok(())
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
    let settings = state.backend.safe_lock().clone();
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

    *state.pet_status.safe_lock() = "thinking".into();
    let transformed = {
        let mode = mode.clone();
        let text = text.clone();
        tauri::async_runtime::spawn_blocking(move || transform_text(&settings, &mode, &text))
            .await
            .map_err(|error| format!("preview task failed: {error}"))
            .and_then(|inner| inner)
    };
    let transformed = match transformed {
        Ok(t) => {
            *state.pet_status.safe_lock() = "speaking".into();
            t
        }
        Err(e) => {
            // Reset pet status on any error (task panic or model failure) so
            // the pet is not stuck in "thinking" forever after a transient error.
            *state.pet_status.safe_lock() = "idle".into();
            return Err(e);
        }
    };

    let warning = model_task(&mode)
        .and_then(|task| assistant_inference::quality_warning(task, &text, &transformed));
    let diff = to_segments(diff_chars(&text, &transformed));
    let settings = state.backend.safe_lock().clone();
    // Cache this result so a subsequent `apply` for the same mode/source/backend
    // writes back exactly what the user saw, without re-running the model.
    *state.last_preview.safe_lock() = Some(CachedPreview {
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
    let settings = state.backend.safe_lock().clone();
    let signature = backend_signature(&settings);
    let cached = state.last_preview.safe_lock().clone();
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
    *state.last_receipt.safe_lock() = Some(receipt);
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
    *state.last_receipt.safe_lock() = None;
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
    *state.pet_status.safe_lock() = "idle".into();
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

/// Widget hotkey IDs. Each widget with a global shortcut gets its own ID,
/// registered on a dedicated background thread so it cannot interfere with the
/// assistant transform/undo hotkey loop.
const WIDGET_HK_OCR: i32 = 0x10;
const WIDGET_HK_TRANSLATE: i32 = 0x11;
const WIDGET_HK_COLORPICKER: i32 = 0x12;
const WIDGET_HK_CLIPBOARD: i32 = 0x13;
const WIDGET_HK_CALCULATOR: i32 = 0x14;

/// Register widget global shortcuts on a background thread and pump messages.
///
/// Each widget's shortcut (from its `WidgetManifest.shortcut`) is mapped to a
/// fixed hotkey ID; the message loop dispatches back to `widgets::open_widget`.
/// Failures to register individual hotkeys are logged but do not abort the
/// loop, since another app may already own the key combination.
fn spawn_widget_hotkey_worker(app: AppHandle) {
    thread::spawn(move || {
        let modifiers = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
        // (id, vk, widget_id) — keep in sync with `WidgetManifest.shortcut`.
        let bindings: [(i32, u16, &str); 5] = [
            (WIDGET_HK_OCR, VK_O.0, "widget-ocr"),
            (WIDGET_HK_TRANSLATE, VK_T.0, "widget-translate"),
            (WIDGET_HK_COLORPICKER, VK_C.0, "widget-colorpicker"),
            (WIDGET_HK_CLIPBOARD, VK_V.0, "widget-clipboard"),
            (WIDGET_HK_CALCULATOR, VK_OEM_PLUS.0, "widget-calculator"),
        ];

        let mut registered: Vec<i32> = Vec::new();
        for (id, vk, _) in &bindings {
            match unsafe { RegisterHotKey(None, *id, modifiers, (*vk).into()) } {
                Ok(_) => registered.push(*id),
                Err(e) => eprintln!("[lingxi] widget hotkey {id} register failed: {e}"),
            }
        }

        let mut msg = MSG::default();
        loop {
            let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if ret.0 <= 0 {
                break;
            }
            if msg.message == WM_HOTKEY {
                let id = msg.wParam.0 as i32;
                if let Some((_, _, widget_id)) = bindings.iter().find(|(hid, _, _)| *hid == id) {
                    if let Some(manifest) = widgets::builtin_widgets()
                        .into_iter()
                        .find(|w| w.id == *widget_id)
                    {
                        let app = app.clone();
                        // Open on the main thread so Tauri can manipulate
                        // windows; spawning avoids blocking the message pump.
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = widgets::open_widget(&app, &manifest) {
                                eprintln!("[lingxi] open widget from hotkey: {e}");
                            }
                        });
                    }
                }
            }
        }

        for id in &registered {
            let _ = unsafe { UnregisterHotKey(None, *id) };
        }
    });
}

fn on_transform(app: &AppHandle) {
    let adapter = WindowsAdapter::new();
    match adapter.capture_selection() {
        Ok(snapshot) => {
            let state = app.state::<AppState>();
            *state.snapshot.safe_lock() = Some(snapshot);
            state.selection_revision.fetch_add(1, Ordering::AcqRel);
            *state.last_preview.safe_lock() = None;
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
    let receipt = state.last_receipt.safe_lock().clone();
    if let Some(receipt) = receipt {
        let adapter = WindowsAdapter::new();
        let _ = adapter.undo(&receipt);
        *state.last_receipt.safe_lock() = None;
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
        let s = ime_shared().safe_lock();
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
        let mut s = ime_shared().safe_lock();
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
        let mut s = ime_shared().safe_lock();
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
                let pending = shared.safe_lock().pending_commit.take();
                if let Some(text) = pending {
                    if let Err(error) = insert_text_at_caret(&text) {
                        eprintln!("IME commit failed: {error}");
                    }
                }

                let (active, revision, pinyin, context) = {
                    let s = shared.safe_lock();
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
                    let mut s = shared.safe_lock();
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
        if !shared.safe_lock().active || wparam.0 != 0x0100 {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let vk = kb.vkCode as u16;
        let handled = {
            let mut s = shared.safe_lock();
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
            let mut s = ime_shared().safe_lock();
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

// ===========================================================================
// Widget Tauri commands — each widget is a small independent window.
// ===========================================================================

#[tauri::command]
fn list_widgets() -> Vec<widgets::WidgetManifest> {
    widgets::builtin_widgets()
}

/// Open a widget window. MUST NOT run on the main thread: while the main
/// thread is inside an IPC dispatch or tray-menu callback it cannot pump
/// messages, and `WebviewWindowBuilder::build()` needs the main thread to
/// complete — calling it synchronously deadlocks the whole app (window
/// never appears, tray quit stops working). The background-thread path
/// (hotkey worker / verify mode) never deadlocks, so route every caller
/// through a spawned thread.
#[tauri::command]
async fn open_widget(app: AppHandle, id: String) -> Result<(), String> {
    let manifest = widgets::builtin_widgets()
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| format!("未知小工具: {id}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        widgets::open_widget(&app, &manifest).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("打开小工具任务失败: {e}"))?
}

#[tauri::command]
async fn close_widget(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        widgets::close_widget(&app, &id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("关闭小工具任务失败: {e}"))?
}

/// Return the ids of widget windows currently open. Used by the tools grid to
/// mark already-open widgets without re-querying all webview windows.
#[tauri::command]
fn list_open_widgets(app: AppHandle) -> Vec<String> {
    widgets::open_widget_ids(&app)
}

/// Capture the full screen and return as a PNG data URL.
///
/// Runs on a blocking thread (spawn_blocking) so the widget WebView does not
/// freeze while BitBlt + PNG encode runs. A 10s timeout guards against GDI hangs.
#[tauri::command]
async fn widget_capture_screen() -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        let result = tauri::async_runtime::spawn_blocking(|| {
            lingxi_tools_windows::screen_capture::capture_screen_as_data_url()
        })
        .await
        .map_err(|e| format!("截图任务失败: {e}"))?;
        let url = tokio::time::timeout(Duration::from_secs(10), async move { result })
            .await
            .map_err(|_| "截图超时（10秒）".to_string())?;
        url.map(|u| serde_json::json!({ "image": u }))
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".to_string())
    }
}

/// Capture a screen region and run OCR on it.
///
/// OCR launches PowerShell + WinRT OcrEngine which takes 2-6 seconds. Running
/// this on the main thread would freeze the widget window; spawn_blocking +
/// 20s timeout keeps the UI responsive and prevents indefinite hangs.
#[tauri::command]
async fn widget_ocr(x: i32, y: i32, w: i32, h: i32) -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        let result = tauri::async_runtime::spawn_blocking(move || {
            // First capture the region as a data URL so the frontend can display it.
            let data_url = lingxi_tools_windows::screen_capture::capture_region_as_data_url(x, y, w, h)?;

            // Try WinRT OCR via PowerShell (Windows 10+ has built-in OCR).
            let temp_path = std::env::temp_dir().join("lingxi_ocr_temp.png");
            let png_data = lingxi_tools_windows::screen_capture::capture_region(x, y, w, h)?;
            let png_bytes = lingxi_tools_windows::screen_capture::encode_png(&png_data)?;
            std::fs::write(&temp_path, &png_bytes).map_err(|e| format!("写入临时文件失败: {e}"))?;

            let script = format!(
                r#"
            Add-Type -AssemblyName System.Windows.Media
            $bmp = [System.Windows.Media.Imaging.BitmapFrame]::Create([System.IO.File]::OpenRead('{}'))
            $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage([Windows.Globalization.Language]::CreateLanguage('zh-Hans-CN'))
            if (-not $engine) {{ $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages() }}
            if (-not $engine) {{ Write-Output ''; exit }}
            $result = $engine.RecognizeAsync($bmp).AwaitResult()
            Write-Output $result.Text
            "#,
                temp_path.display()
            );

            let output = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .output();

            let text = match output {
                Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
                Err(_) => String::new(),
            };

            let _ = std::fs::remove_file(&temp_path);

            Ok::<_, String>(serde_json::json!({
                "text": text,
                "image": data_url,
            }))
        })
        .await
        .map_err(|e| format!("OCR 任务失败: {e}"))?;
        tokio::time::timeout(Duration::from_secs(20), async move { result })
            .await
            .map_err(|_| "OCR 超时（20秒），WinRT OCR 引擎可能未安装".to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (x, y, w, h);
        Err("仅支持 Windows".to_string())
    }
}

/// Read the pixel color at the current cursor position.
#[tauri::command]
async fn widget_pick_color(app: AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON};

        // Interactive picking: hide the picker window first, let the user move
        // the mouse anywhere, and read the pixel where they left-click. The
        // old version read the pixel at the cursor the instant the button was
        // pressed — which was always the button's own color.
        let window = app
            .get_webview_window("widget-colorpicker")
            .ok_or("取色器窗口未打开")?;
        window.hide().map_err(|e| format!("隐藏窗口失败: {e}"))?;
        std::thread::sleep(Duration::from_millis(120));

        let result = tauri::async_runtime::spawn_blocking(move || {
            // Wait for the left button that triggered this command to be
            // released, so we don't instantly pick the button pixel.
            for _ in 0..200 {
                if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }

            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            loop {
                if unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } < 0 {
                    return Err("已取消取色".to_string());
                }
                if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0 {
                    let mut point = POINT::default();
                    unsafe { GetCursorPos(&mut point).map_err(|e| format!("GetCursorPos: {e}"))? };
                    let (r, g, b) =
                        lingxi_tools_windows::screen_capture::read_pixel(point.x, point.y)?;
                    return Ok(serde_json::json!({
                        "r": r, "g": g, "b": b, "x": point.x, "y": point.y,
                    }));
                }
                if std::time::Instant::now() > deadline {
                    return Err("取色超时（60秒未点击）".to_string());
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        })
        .await
        .map_err(|e| format!("取色任务失败: {e}"))?;

        window.show().map_err(|e| format!("恢复窗口失败: {e}"))?;
        let _ = window.set_focus();
        result
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("仅支持 Windows".to_string())
    }
}

/// Fetch weather from Open-Meteo (free, no API key required).
///
/// Location resolution order: explicit city name (geocoding API) → IP
/// geolocation (ip-api.com works in mainland China, ipapi.co as fallback) →
/// default Beijing. Each HTTP call has a 10s PowerShell timeout; the overall
/// timeout wraps the blocking task so the widget can never hang.
#[tauri::command]
async fn widget_get_weather(city: Option<String>) -> Result<serde_json::Value, String> {
    let city = city
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let (lat, lon, city_label) = match &city {
            Some(name) => geocode_city(name)?,
            None => locate_by_ip(),
        };

        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current_weather=true&daily=weather_code,temperature_2m_max,temperature_2m_min,relative_humidity_2m_max,wind_speed_10m_max&timezone=auto"
        );
        let resp = http_get_text(&url)?;
        let data: serde_json::Value = serde_json::from_str(&resp)
            .map_err(|e| format!("解析天气失败: {e}"))?;

        let cw = data.get("current_weather").ok_or("无当前天气数据")?;
        let daily = data.get("daily").ok_or("无预报数据")?;

        let weather_code = cw.get("weather_code").and_then(|v| v.as_i64()).unwrap_or(0);
        let description = weather_description(weather_code);

        let mut forecast = Vec::new();
        if let Some(dates) = daily.get("time").and_then(|v| v.as_array()) {
            if let Some(codes) = daily.get("weather_code").and_then(|v| v.as_array()) {
                if let Some(maxs) = daily.get("temperature_2m_max").and_then(|v| v.as_array()) {
                    if let Some(mins) = daily.get("temperature_2m_min").and_then(|v| v.as_array()) {
                        for i in 0..dates.len().min(3) {
                            forecast.push(serde_json::json!({
                                "date": dates[i].as_str().unwrap_or(""),
                                "weather_code": codes[i].as_i64().unwrap_or(0),
                                "max": maxs[i].as_f64().unwrap_or(0.0),
                                "min": mins[i].as_f64().unwrap_or(0.0),
                            }));
                        }
                    }
                }
            }
        }

        Ok::<_, String>(serde_json::json!({
            "city": city_label,
            "current": {
                "temperature": cw.get("temperature").and_then(|v| v.as_f64()).unwrap_or(0.0),
                "weather_code": weather_code,
                "description": description,
                "wind_speed": cw.get("windspeed").and_then(|v| v.as_f64()).unwrap_or(0.0),
                "humidity": daily.get("relative_humidity_2m_max")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            },
            "daily": forecast,
        }))
    });
    // The timeout must wrap the blocking task itself — the old code awaited
    // first and timed out on an already-finished future, so a hung PowerShell
    // request froze the widget forever.
    tokio::time::timeout(Duration::from_secs(30), task)
        .await
        .map_err(|_| "天气查询超时（30秒），请检查网络".to_string())?
        .map_err(|e| format!("天气任务失败: {e}"))?
}

/// Evaluate a mathematical expression.
#[tauri::command]
async fn widget_calculate(expression: String) -> Result<serde_json::Value, String> {
    use lingxi_tools::{builtin::calc::CalculateTool, Tool, ToolContext, AutoConfirm};
    let tool = CalculateTool;
    let ctx = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        confirm: std::sync::Arc::new(AutoConfirm),
        session_id: String::new(),
    };
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tool.execute(serde_json::json!({ "expression": expression }), &ctx),
    )
    .await
    .map_err(|_| "计算超时（5秒）".to_string())?;
    if result.success {
        Ok(serde_json::json!({ "result": result.output.trim() }))
    } else {
        Err(result.output)
    }
}

/// Resolve translation provider config: the settings page (AppState) wins
/// when an API key is saved there; otherwise fall back to the
/// `LINGXI_OPENAI_*` environment variables, then DeepSeek defaults.
fn translation_config(state: &AppState) -> (String, String, String) {
    const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
    const DEFAULT_MODEL: &str = "deepseek-chat";

    let settings = state.backend.safe_lock();
    if !settings.api_key.trim().is_empty() {
        let endpoint = if settings.endpoint.trim().is_empty() {
            DEFAULT_BASE_URL.to_string()
        } else {
            settings.endpoint.clone()
        };
        let model = if settings.model.trim().is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            settings.model.clone()
        };
        return (settings.api_key.trim().to_string(), endpoint, model);
    }
    drop(settings);

    let key = std::env::var("LINGXI_OPENAI_API_KEY").unwrap_or_default();
    let base = std::env::var("LINGXI_OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into());
    let model = std::env::var("LINGXI_OPENAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
    (key, base, model)
}

/// Shared translation core used by both the widget command and the
/// capture-and-translate flow.
async fn translate_text(
    state: &AppState,
    text: String,
    from: String,
    to: String,
) -> Result<String, String> {
    let (api_key, base_url, model) = translation_config(state);
    if api_key.is_empty() {
        return Err(
            "翻译服务未配置：请在主面板「设置」中填写云端 API Key（或设置 \
             LINGXI_OPENAI_API_KEY 环境变量）"
                .to_string(),
        );
    }
    let translated = tauri::async_runtime::spawn_blocking(move || {
        lingxi_tools::builtin::translate::translate_with_config(
            &api_key, &base_url, &model, &text, &from, &to,
        )
    })
    .await
    .map_err(|e| format!("翻译任务失败: {e}"))??;
    Ok(translated)
}

/// Translate text using the settings-page cloud config.
#[tauri::command]
async fn widget_translate(
    state: tauri::State<'_, AppState>,
    text: String,
    from: String,
    to: String,
) -> Result<serde_json::Value, String> {
    let translated = tokio::time::timeout(
        Duration::from_secs(20),
        translate_text(&state, text, from, to),
    )
    .await
    .map_err(|_| "翻译超时（20秒），请检查网络或稍后重试".to_string())??;
    Ok(serde_json::json!({ "translated": translated.trim() }))
}

/// Capture screen, let user select region, OCR and translate in one step.
#[tauri::command]
async fn widget_capture_and_translate(
    state: tauri::State<'_, AppState>,
    target_lang: String,
) -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        // OCR on a blocking thread (PowerShell + WinRT takes seconds).
        let source = tauri::async_runtime::spawn_blocking(|| {
            let img = lingxi_tools_windows::screen_capture::capture_screen()?;
            let temp_path = std::env::temp_dir().join("lingxi_ocr_translate.png");
            let png_bytes = lingxi_tools_windows::screen_capture::encode_png(&img)?;
            std::fs::write(&temp_path, &png_bytes).map_err(|e| format!("写入临时文件失败: {e}"))?;

            let script = format!(
                r#"
            Add-Type -AssemblyName System.Windows.Media
            $bmp = [System.Windows.Media.Imaging.BitmapFrame]::Create([System.IO.File]::OpenRead('{}'))
            $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
            if (-not $engine) {{ Write-Output ''; exit }}
            $result = $engine.RecognizeAsync($bmp).AwaitResult()
            Write-Output $result.Text
            "#,
                temp_path.display()
            );

            let output = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .output();
            let _ = std::fs::remove_file(&temp_path);
            Ok::<_, String>(match output {
                Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
                Err(_) => String::new(),
            })
        })
        .await
        .map_err(|e| format!("截图翻译任务失败: {e}"))??;

        if source.is_empty() {
            return Ok(serde_json::json!({
                "source": "",
                "translated": "(未识别到文字)",
            }));
        }

        // Translate the recognized text.
        let translated = match translate_text(&state, source.clone(), "auto".into(), target_lang).await {
            Ok(t) => t,
            Err(e) => format!("翻译失败: {e}"),
        };

        Ok(serde_json::json!({
            "source": source,
            "translated": translated,
        }))
    }
    #[cfg(not(windows))]
    {
        let _ = target_lang;
        Err("仅支持 Windows".to_string())
    }
}

/// In-memory clipboard history (per app session).
static CLIPBOARD_HISTORY: std::sync::OnceLock<Mutex<Vec<ClipboardEntry>>> = std::sync::OnceLock::new();

#[derive(Clone, serde::Serialize)]
struct ClipboardEntry {
    text: String,
    time: String,
}

fn clipboard_history() -> &'static Mutex<Vec<ClipboardEntry>> {
    CLIPBOARD_HISTORY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Background clipboard watcher. A real WM_CLIPBOARDUPDATE listener needs a
/// message-only window on the event loop; a 1.5s poll is pragmatic for a
/// history tool: no window plumbing, no risk of deadlocking the UI, and new
/// text shows up within a couple of seconds.
fn spawn_clipboard_listener() {
    std::thread::spawn(|| {
        use assistant_windows::read_clipboard_text;
        let mut last_seen: Option<String> = None;
        loop {
            if let Ok(raw) = read_clipboard_text() {
                let text = raw.trim().to_string();
                if !text.is_empty() && last_seen.as_deref() != Some(text.as_str()) {
                    last_seen = Some(text.clone());
                    // Cap each entry at ~10k chars so a huge copy cannot bloat
                    // the in-memory history.
                    let text: String = text.chars().take(10_000).collect();
                    if let Ok(mut history) = clipboard_history().lock() {
                        let duplicate = history
                            .first()
                            .map(|e| e.text == text)
                            .unwrap_or(false);
                        if !duplicate {
                            history.insert(0, ClipboardEntry {
                                text,
                                time: chrono_like_now(),
                            });
                            if history.len() > 50 {
                                history.truncate(50);
                            }
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    });
}

#[tauri::command]
fn widget_clipboard_history() -> Result<Vec<ClipboardEntry>, String> {
    let history = clipboard_history().lock().map_err(|e| format!("锁失败: {e}"))?;
    Ok(history.clone())
}

#[tauri::command]
fn widget_clipboard_write(text: String) -> Result<(), String> {
    use assistant_windows::write_clipboard_text;
    let now = chrono_like_now();
    let mut history = clipboard_history().lock().map_err(|e| format!("锁失败: {e}"))?;
    // Avoid duplicates of consecutive identical entries (newest is at index 0).
    if history.first().map(|e| e.text == text).unwrap_or(false) {
        return Ok(());
    }
    history.insert(0, ClipboardEntry { text: text.clone(), time: now });
    if history.len() > 50 {
        history.truncate(50);
    }
    drop(history);
    write_clipboard_text(&text).map_err(|e| format!("写入剪贴板失败: {e}"))
}

#[tauri::command]
fn widget_clipboard_clear() -> Result<(), String> {
    let mut history = clipboard_history().lock().map_err(|e| format!("锁失败: {e}"))?;
    history.clear();
    Ok(())
}

/// Remove a single entry (first match by text) so the delete button in the
/// clipboard widget actually persists.
#[tauri::command]
fn widget_clipboard_remove(text: String) -> Result<(), String> {
    let mut history = clipboard_history().lock().map_err(|e| format!("锁失败: {e}"))?;
    if let Some(pos) = history.iter().position(|e| e.text == text) {
        history.remove(pos);
    }
    Ok(())
}

/// Read current system clipboard text (for clipboard-history polling).
#[tauri::command]
fn widget_read_clipboard() -> Result<String, String> {
    use assistant_windows::read_clipboard_text;
    read_clipboard_text().map_err(|e| format!("读取剪贴板失败: {e}"))
}

// --- Helpers for widget commands ---

fn http_get_text(url: &str) -> Result<String, String> {
    // -TimeoutSec is critical: without it Invoke-RestMethod waits forever on
    // stalled connections, which froze the weather widget.
    let cmd = format!(
        "(Invoke-RestMethod -Uri '{}' -UseBasicParsing -TimeoutSec 10) | ConvertTo-Json -Depth 10 -Compress",
        url
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(&cmd)
        .output()
        .map_err(|e| format!("HTTP 请求失败: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("HTTP 请求错误: {}", err.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // PowerShell sometimes wraps strings in quotes; unwrap them.
    if stdout.starts_with('"') && stdout.ends_with('"') {
        Ok(stdout[1..stdout.len()-1].replace("\\\"", "\""))
    } else {
        Ok(stdout)
    }
}

/// Minimal percent-encoding for URL query values (UTF-8, non-ASCII included).
fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Resolve a city name (Chinese OK) to coordinates via Open-Meteo geocoding.
fn geocode_city(name: &str) -> Result<(f64, f64, String), String> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=zh&format=json",
        url_encode(name)
    );
    let body = http_get_text(&url)?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析城市数据失败: {e}"))?;
    let first = v
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| format!("未找到城市「{name}」，请换个名称试试"))?;
    let lat = first
        .get("latitude")
        .and_then(|x| x.as_f64())
        .ok_or("城市坐标无效")?;
    let lon = first
        .get("longitude")
        .and_then(|x| x.as_f64())
        .ok_or("城市坐标无效")?;
    let label = first
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or(name)
        .to_string();
    Ok((lat, lon, label))
}

/// IP geolocation with a fallback chain. ipapi.co returns 403 from mainland
/// China, so try ip-api.com first (returns Chinese city names). Never fails:
/// falls back to Beijing so the weather widget always shows something.
fn locate_by_ip() -> (f64, f64, String) {
    if let Ok(body) = http_get_text("http://ip-api.com/json/?lang=zh-CN") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
            let ok = v.get("status").and_then(|s| s.as_str()) == Some("success");
            let lat = v.get("lat").and_then(|x| x.as_f64());
            let lon = v.get("lon").and_then(|x| x.as_f64());
            if ok {
                if let (Some(lat), Some(lon)) = (lat, lon) {
                    let city = v
                        .get("city")
                        .and_then(|x| x.as_str())
                        .or_else(|| v.get("regionName").and_then(|x| x.as_str()))
                        .unwrap_or("当前位置");
                    return (lat, lon, city.to_string());
                }
            }
        }
    }
    if let Ok(body) = http_get_text("https://ipapi.co/json/") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
            let lat = v.get("latitude").and_then(|x| x.as_f64());
            let lon = v.get("longitude").and_then(|x| x.as_f64());
            if let (Some(lat), Some(lon)) = (lat, lon) {
                let city = v.get("city").and_then(|x| x.as_str()).unwrap_or("当前位置");
                return (lat, lon, city.to_string());
            }
        }
    }
    (39.9, 116.4, "北京（默认）".to_string())
}

fn weather_description(code: i64) -> &'static str {
    match code {
        0 => "晴朗",
        1 => "大部晴朗",
        2 => "多云",
        3 => "阴天",
        45 | 48 => "雾",
        51 | 53 | 55 => "毛毛雨",
        56 | 57 => "冻毛毛雨",
        61 | 63 | 65 => "雨",
        66 | 67 => "冻雨",
        71 | 73 | 75 => "雪",
        77 => "雪粒",
        80..=82 => "阵雨",
        85 | 86 => "阵雪",
        95 => "雷暴",
        96 | 99 => "雷暴冰雹",
        _ => "未知",
    }
}

/// "HH:mm" local time. Spawning PowerShell for this (the old approach) costs
/// ~1s per call, which is unacceptable inside the 1.5s clipboard poll loop;
/// `GetLocalTime` is a zero-cost kernel call instead.
fn chrono_like_now() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let t = unsafe { GetLocalTime() };
        format!("{:02}:{:02}", t.wHour, t.wMinute)
    }
    #[cfg(not(windows))]
    String::new()
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

    // Build widget submenu items dynamically from the builtin catalog.
    let widget_items: Vec<_> = widgets::builtin_widgets()
        .iter()
        .map(|w| {
            MenuItem::with_id(
                app,
                format!("widget:{}", w.id),
                format!("{} {}", w.icon, w.label),
                true,
                None::<&str>,
            )
            .expect("failed to create widget menu item")
        })
        .collect();

    let mut submenu_builder = SubmenuBuilder::new(app, "小工具");
    for item in &widget_items {
        submenu_builder = submenu_builder.item(item);
    }
    let widget_submenu = submenu_builder.build()?;

    let quit = MenuItem::with_id(app, "tray:quit", "退出灵犀", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show_panel, &hide_panel, &separator, &widget_submenu, &separator, &quit])?;
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
            id if id.starts_with("widget:") => {
                // Tray menu callbacks run on the main thread; building a
                // WebView2 window from there deadlocks (see open_widget).
                let widget_id = id[7..].to_string();
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Some(manifest) = widgets::builtin_widgets()
                        .into_iter()
                        .find(|w| w.id == widget_id)
                    {
                        if let Err(e) = widgets::open_widget(&app, &manifest) {
                            eprintln!("[lingxi] open widget from tray: {e}");
                        }
                    }
                });
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
            quit_app,
            agent_chat,
            agent_history,
            agent_reset,
            list_tools,
            toggle_tool,
            // Widget commands
            list_widgets,
            open_widget,
            close_widget,
            list_open_widgets,
            widget_capture_screen,
            widget_ocr,
            widget_pick_color,
            widget_get_weather,
            widget_calculate,
            widget_translate,
            widget_capture_and_translate,
            widget_clipboard_history,
            widget_clipboard_write,
            widget_clipboard_clear,
            widget_clipboard_remove,
            widget_read_clipboard
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
            if app.state::<AppState>().backend.safe_lock().backend == "local" {
                assistant_inference::prepare_in_background();
            }
            spawn_hotkey_worker(app.handle().clone());
            spawn_widget_hotkey_worker(app.handle().clone());
            spawn_qq_foreground_sampler();
            spawn_clipboard_listener();
            install_tray(app.handle())?;
            // Smoke-test mode: LINGXI_OPEN_ALL_WIDGETS=1 opens every widget
            // window in sequence so they can be verified in bulk (check the
            // stderr log for "page_load: Finished" per widget).
            if std::env::var("LINGXI_OPEN_ALL_WIDGETS").is_ok() {
                let handle = app.handle().clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(1500));
                    for w in widgets::builtin_widgets() {
                        eprintln!("[lingxi] auto-open widget {} (verify mode)", w.id);
                        if let Err(e) = widgets::open_widget(&handle, &w) {
                            eprintln!("[lingxi] auto-open widget {} FAILED: {}", w.id, e);
                        }
                        thread::sleep(Duration::from_millis(500));
                    }
                    eprintln!("[lingxi] verify mode: all widgets opened");
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to launch LingXi overlay");
}
