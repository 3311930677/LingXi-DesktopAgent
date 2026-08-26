//! 后台热键工作线程：改写/撤销热键循环 + 小工具全局快捷键消息泵。

use std::thread;
use std::time::Duration;

use assistant_windows::{run_assistant_hotkey_loop, wait_for_trigger_release, AssistantHotkey};
use tauri::AppHandle;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_C, VK_O, VK_OEM_PLUS,
    VK_T, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

use crate::rewrite::{on_transform, on_undo};
use crate::widgets;
/// Background hotkey worker: capture on transform, revert on undo.
pub(crate) fn spawn_hotkey_worker(app: AppHandle) {
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
/// fixed hotkey ID; the message loop dispatches back to `widgets::open_widget_window`.
/// Failures to register individual hotkeys are logged but do not abort the
/// loop, since another app may already own the key combination.
pub(crate) fn spawn_widget_hotkey_worker(app: AppHandle) {
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
                            if let Err(e) = widgets::open_widget_window(&app, &manifest) {
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
