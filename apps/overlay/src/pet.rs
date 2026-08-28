//! 桌宠命令：状态切换、皮肤热切换、气泡文案与可见性设置。

use tauri::{AppHandle, Emitter, Manager, State};

use crate::pet_skin;
use crate::settings::persist_backend_settings;
use crate::state::{AppState, MutexExt};

#[tauri::command]
pub(crate) fn pet_status(state: State<AppState>) -> String {
    state.pet_status.safe_lock().clone()
}

#[tauri::command]
pub(crate) fn set_pet_status(state: State<AppState>, status: String) -> Result<(), String> {
    if !matches!(status.as_str(), "idle" | "thinking" | "speaking" | "alert") {
        return Err("invalid pet status".into());
    }
    *state.pet_status.safe_lock() = status;
    Ok(())
}

/// 设置页：列出全部可用皮肤。
#[tauri::command]
pub(crate) fn list_pet_skins() -> Vec<pet_skin::PetSkinInfo> {
    pet_skin::list_skins()
}

/// 桌宠窗口启动时拉取当前配置（皮肤 + 气泡覆盖 + 可见性）。
#[tauri::command]
pub(crate) fn current_pet_config(state: State<AppState>) -> Result<pet_skin::PetSkinView, String> {
    let settings = state.backend.safe_lock();
    pet_skin::view_for(
        &settings.pet_skin,
        &settings.pet_bubble_overrides,
        settings.pet_visible,
    )
}

/// 切换皮肤：校验 → 落盘 → 广播 `pet-config-changed`（桌宠窗口即时热换）。
#[tauri::command]
pub(crate) fn set_pet_skin(
    app: AppHandle,
    state: State<AppState>,
    skin_id: String,
) -> Result<pet_skin::PetSkinView, String> {
    if !pet_skin::valid_skin_id(&skin_id) {
        return Err("皮肤 id 非法".into());
    }
    let view = {
        let mut settings = state.backend.safe_lock();
        // 先校验皮肤可用，失败不落盘；切换不改变可见性设置。
        pet_skin::load_manifest(&skin_id)?;
        let visible = settings.pet_visible;
        let overrides = settings.pet_bubble_overrides.clone();
        settings.pet_skin = skin_id.clone();
        persist_backend_settings(&settings)?;
        drop(settings);
        pet_skin::view_for(&skin_id, &overrides, visible)?
    };
    app.emit("pet-config-changed", &view)
        .map_err(|e| e.to_string())?;
    Ok(view)
}

/// 桌宠杂项设置：气泡文案覆盖 + 可见性开关。
#[tauri::command]
pub(crate) fn set_pet_options(
    app: AppHandle,
    state: State<AppState>,
    overrides: pet_skin::PetBubbleOverrides,
    visible: bool,
) -> Result<pet_skin::PetSkinView, String> {
    let view = {
        let mut settings = state.backend.safe_lock();
        // 覆盖文案裁剪：空串视为"用皮肤默认"。
        let trimmed = |text: &Option<String>| {
            text.as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        settings.pet_bubble_overrides = pet_skin::PetBubbleOverrides {
            idle: trimmed(&overrides.idle),
            thinking: trimmed(&overrides.thinking),
            speaking: trimmed(&overrides.speaking),
            alert: trimmed(&overrides.alert),
        };
        settings.pet_visible = visible;
        persist_backend_settings(&settings)?;
        let skin_id = settings.pet_skin.clone();
        let overrides = settings.pet_bubble_overrides.clone();
        drop(settings);
        pet_skin::view_for(&skin_id, &overrides, visible)?
    };
    if visible {
        if let Some(pet) = app.get_webview_window("pet") {
            let _ = pet.show();
        }
    } else if let Some(pet) = app.get_webview_window("pet") {
        let _ = pet.hide();
    }
    app.emit("pet-config-changed", &view)
        .map_err(|e| e.to_string())?;
    Ok(view)
}
