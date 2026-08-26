//! 模型后端与窗口行为设置：加载/落盘 + 设置页命令。
//!
//! `settings.json` 保存后端选择、桌宠皮肤与窗口行为开关；
//! API Key 永不落盘（`serde(skip)`），勾选"安全记忆"时走 DPAPI（secret_store）。

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::secret_store;
use crate::state::{AppState, MutexExt};
use crate::window_state;

/// Runtime model settings. The API key is session-only by design: endpoint,
/// model and backend choice are persisted, but plaintext credentials are not.
/// 桌宠相关字段（pet_skin/pet_bubble_overrides/pet_visible）同存此文件：
/// 复用同一份落盘通道，避免多配置文件与读写竞争。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct BackendSettings {
    pub(crate) backend: String,
    pub(crate) endpoint: String,
    pub(crate) model: String,
    #[serde(skip)]
    pub(crate) api_key: String,
    /// Whether the key is persisted using Windows DPAPI for this user.
    pub(crate) remember_api_key: bool,
    /// 当前桌宠皮肤 id（见 ui/assets/skins/）。
    pub(crate) pet_skin: String,
    /// 用户自定义气泡文案（None = 使用皮肤默认文案）。
    pub(crate) pet_bubble_overrides: crate::pet_skin::PetBubbleOverrides,
    /// 桌宠窗口是否显示。
    pub(crate) pet_visible: bool,
    /// 面板失焦（用户切到别的应用）后自动收起。
    pub(crate) panel_auto_hide: bool,
    /// 记住面板拖动位置；关闭则每次出现在光标附近。
    pub(crate) panel_remember_position: bool,
}

impl Default for BackendSettings {
    fn default() -> Self {
        Self {
            backend: "local".into(),
            endpoint: "https://api.openai.com".into(),
            model: "gpt-4o-mini".into(),
            api_key: std::env::var("LINGXI_OPENAI_API_KEY").unwrap_or_default(),
            remember_api_key: false,
            pet_skin: crate::pet_skin::DEFAULT_SKIN_ID.to_string(),
            pet_bubble_overrides: Default::default(),
            pet_visible: true,
            panel_auto_hide: true,
            panel_remember_position: true,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct BackendSettingsView {
    pub(crate) backend: String,
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) api_key_configured: bool,
    pub(crate) remember_api_key: bool,
}

#[derive(Deserialize)]
pub(crate) struct BackendSettingsInput {
    pub(crate) backend: String,
    pub(crate) endpoint: String,
    pub(crate) model: String,
    // Accept both snake_case and the JS-idiomatic camelCase so a future
    // frontend key style change cannot break saving again.
    #[serde(alias = "apiKey")]
    pub(crate) api_key: String,
    #[serde(alias = "rememberApiKey")]
    pub(crate) remember_api_key: bool,
}

pub(crate) fn backend_settings_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lingxi").join("settings.json"))
}

pub(crate) fn load_backend_settings() -> BackendSettings {
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

pub(crate) fn persist_backend_settings(settings: &BackendSettings) -> Result<(), String> {
    let path = backend_settings_path().ok_or("cannot resolve config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    // `api_key` has serde(skip), so credentials never reach this file.
    let json = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn get_backend_settings(state: State<AppState>) -> BackendSettingsView {
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
pub(crate) fn save_backend_settings(
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
pub(crate) fn model_progress() -> assistant_inference::ProgressSnapshot {
    assistant_inference::progress_snapshot()
}

#[derive(Serialize)]
pub(crate) struct WindowOptionsView {
    pub(crate) panel_auto_hide: bool,
    pub(crate) panel_remember_position: bool,
}

#[tauri::command]
pub(crate) fn get_window_options(state: State<AppState>) -> WindowOptionsView {
    let settings = state.backend.safe_lock();
    WindowOptionsView {
        panel_auto_hide: settings.panel_auto_hide,
        panel_remember_position: settings.panel_remember_position,
    }
}

/// 窗口行为开关。关掉"记住位置"时同时清空已存的面板位置与拖动标志，
/// 面板立即回到"跟随光标出现"模式。
#[tauri::command]
pub(crate) fn set_window_options(
    state: State<AppState>,
    panel_auto_hide: bool,
    panel_remember_position: bool,
) -> Result<(), String> {
    {
        let mut settings = state.backend.safe_lock();
        settings.panel_auto_hide = panel_auto_hide;
        settings.panel_remember_position = panel_remember_position;
        persist_backend_settings(&settings)?;
    }
    if !panel_remember_position {
        state
            .user_positioned
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let mut windows = state.window_state.safe_lock();
        windows.panel = None;
        window_state::persist(&windows)?;
    }
    Ok(())
}
