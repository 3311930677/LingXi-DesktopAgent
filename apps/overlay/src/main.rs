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

mod agent;
mod hotkeys;
mod panel;
mod placement;
mod pet;
mod pet_skin;
mod qq;
mod rewrite;
// OwO 迁移前的旧键盘钩子实现，仅作参考。整个文件编译期排除，稳定后删除。
#[cfg(any())]
mod removed_ime_hook;
mod secret_store;
mod settings;
mod state;
mod tray;
mod widgets;
mod window_state;

use state::{AppState, MutexExt};

use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use tauri::{Manager, PhysicalSize, WindowEvent};

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            rewrite::current_selection,
            rewrite::preview_transform,
            rewrite::apply_transform,
            rewrite::undo_last,
            panel::hide_overlay,
            panel::start_window_drag,
            panel::start_window_resize,
            settings::get_backend_settings,
            settings::save_backend_settings,
            settings::model_progress,
            pet::pet_status,
            pet::set_pet_status,
            pet::list_pet_skins,
            pet::current_pet_config,
            pet::set_pet_skin,
            pet::set_pet_options,
            panel::toggle_panel,
            panel::set_panel_focusable,
            qq::qq_poll_latest,
            qq::capture_qq_selection,
            qq::generate_qq_draft,
            qq::write_qq_draft,
            panel::quit_app,
            agent::agent_chat,
            agent::agent_history,
            agent::agent_reset,
            agent::list_tools,
            agent::toggle_tool,
            // Widget commands
            widgets::list_widgets,
            widgets::open_widget,
            widgets::close_widget,
            widgets::list_open_widgets,
            widgets::widget_capture_screen,
            widgets::widget_ocr,
            widgets::widget_ocr_fullscreen,
            widgets::widget_pick_color_lens,
            widgets::widget_lens_get_image,
            widgets::widget_lens_pick,
            widgets::widget_lens_cancel,
            widgets::widget_get_weather,
            widgets::widget_calculate,
            widgets::widget_translate,
            widgets::widget_translate_lines,
            widgets::widget_clipboard_history,
            widgets::widget_clipboard_write,
            widgets::widget_clipboard_clear,
            widgets::widget_clipboard_remove,
            widgets::widget_read_clipboard,
            settings::get_window_options,
            settings::set_window_options
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Moved(pos) = event {
                let label = window.label();
                if label == "main" || label == "pet" {
                    placement::handle_window_moved(&window.app_handle().clone(), label, *pos);
                }
            }
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            let (remember_position, pet_visible, saved) = {
                let settings = state.backend.safe_lock();
                let windows = state.window_state.safe_lock();
                (
                    settings.panel_remember_position,
                    settings.pet_visible,
                    windows.clone(),
                )
            };
            if let Some(window) = app.get_webview_window("main") {
                panel::make_non_activating(&window).map_err(std::io::Error::other)?;
                // 恢复上次拖动后的面板位置（显示器已拔掉则回落跟随光标）。
                if remember_position {
                    if let Some(pos) = saved.panel {
                        let size = window.outer_size().unwrap_or(PhysicalSize::new(520, 520));
                        if placement::position_on_screen(
                            pos.x,
                            pos.y,
                            size.width as i32,
                            size.height as i32,
                        ) {
                            placement::set_position_tracked(app.handle(), &window, pos.x, pos.y);
                            state.user_positioned.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            if let Some(window) = app.get_webview_window("pet") {
                panel::make_non_activating(&window).map_err(std::io::Error::other)?;
                let size = window.outer_size().unwrap_or(PhysicalSize::new(220, 260));
                let restored = saved.pet.filter(|pos| {
                    placement::position_on_screen(pos.x, pos.y, size.width as i32, size.height as i32)
                });
                match restored {
                    Some(pos) => placement::set_position_tracked(app.handle(), &window, pos.x, pos.y),
                    None => placement::position_pet(app.handle(), &window),
                }
                // 配置里 visible:false 避免先在默认位置闪现再跳到恢复位置；
                // 摆好之后按设置决定是否显示。
                if pet_visible {
                    let _ = window.show();
                }
            }
            // Only local users need the GGUF. A persisted cloud configuration
            // must not trigger a needless ~400MB download at startup.
            if app.state::<AppState>().backend.safe_lock().backend == "local" {
                assistant_inference::prepare_in_background();
            }
            hotkeys::spawn_hotkey_worker(app.handle().clone());
            hotkeys::spawn_widget_hotkey_worker(app.handle().clone());
            panel::spawn_panel_autohide_worker(app.handle().clone());
            qq::spawn_qq_foreground_sampler();
            widgets::spawn_clipboard_listener();
            tray::install_tray(app.handle())?;
            // Smoke-test mode: LINGXI_OPEN_ALL_WIDGETS=1 opens every widget
            // window in sequence so they can be verified in bulk (check the
            // stderr log for "page_load: Finished" per widget).
            if std::env::var("LINGXI_OPEN_ALL_WIDGETS").is_ok() {
                let handle = app.handle().clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(1500));
                    for w in widgets::builtin_widgets() {
                        eprintln!("[lingxi] auto-open widget {} (verify mode)", w.id);
                        if let Err(e) = widgets::open_widget_window(&handle, &w) {
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
