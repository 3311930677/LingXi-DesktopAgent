//! Widget framework: manifest, registry, and multi-window management.
//!
//! Each widget is a small independent Tauri window with its own HTML page,
//! launched on demand via `open_widget`. The manifest declares the widget's
//! metadata (label, shortcut, size) and is shared between the tray menu,
//! the tools grid, and the hotkey system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{AppHandle, Manager, WebviewWindowBuilder};

/// A widget's static metadata, declared at compile time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetManifest {
    /// Unique identifier used as the Tauri window label.
    pub id: &'static str,
    /// Display name shown in tray menu and tools grid.
    pub label: &'static str,
    /// Emoji or short icon string.
    pub icon: &'static str,
    /// Global shortcut (empty string = no shortcut).
    pub shortcut: &'static str,
    /// Default window width in pixels.
    pub width: f64,
    /// Default window height in pixels.
    pub height: f64,
    /// Whether the window is resizable.
    pub resizable: bool,
    /// Whether the widget floats above all other applications. Only
    /// transient pickers (取色器等即用即走的工具) should set this — forcing
    /// every widget on top is intrusive: normal tools like the calculator
    /// or weather should behave like regular app windows.
    pub always_on_top: bool,
    /// HTML page path relative to the ui/ directory.
    pub page: &'static str,
    /// Short description for the tools grid.
    pub description: &'static str,
}

/// The built-in widget catalog. Additional widgets can be registered at
/// runtime in the future via plugins.
pub fn builtin_widgets() -> Vec<WidgetManifest> {
    vec![
        WidgetManifest {
            id: "widget-ocr",
            label: "屏幕识别",
            icon: "🔍",
            shortcut: "Ctrl+Alt+O",
            width: 460.0,
            height: 440.0,
            resizable: true,
            always_on_top: false,
            page: "widgets/ocr.html",
            description: "框选屏幕区域，OCR 提取文字",
        },
        WidgetManifest {
            id: "widget-translate",
            label: "全屏翻译",
            icon: "🌐",
            shortcut: "Ctrl+Alt+T",
            width: 460.0,
            height: 440.0,
            resizable: true,
            always_on_top: false,
            page: "widgets/translate.html",
            description: "框选区域识别并翻译",
        },
        WidgetManifest {
            id: "widget-colorpicker",
            label: "取色器",
            icon: "🎨",
            shortcut: "Ctrl+Alt+C",
            width: 340.0,
            height: 320.0,
            resizable: false,
            always_on_top: true,
            page: "widgets/colorpicker.html",
            description: "屏幕取色，HEX/RGB/HSL",
        },
        WidgetManifest {
            id: "widget-weather",
            label: "天气",
            icon: "🌤️",
            shortcut: "",
            width: 400.0,
            height: 470.0,
            resizable: true,
            always_on_top: false,
            page: "widgets/weather.html",
            description: "当前天气与 3 日预报",
        },
        WidgetManifest {
            id: "widget-calculator",
            label: "计算器",
            icon: "🧮",
            shortcut: "Ctrl+Alt+=",
            width: 360.0,
            height: 430.0,
            resizable: true,
            always_on_top: false,
            page: "widgets/calculator.html",
            description: "输入即算，支持单位换算",
        },
        WidgetManifest {
            id: "widget-clipboard",
            label: "剪贴板历史",
            icon: "📋",
            shortcut: "Ctrl+Alt+V",
            width: 420.0,
            height: 420.0,
            resizable: true,
            always_on_top: false,
            page: "widgets/clipboard.html",
            description: "最近剪贴板记录",
        },
    ]
}

/// Open a widget as a new Tauri window, or focus it if already open.
pub fn open_widget(app: &AppHandle, manifest: &WidgetManifest) -> tauri::Result<()> {
    // If the window already exists, just focus it.
    if let Some(window) = app.get_webview_window(manifest.id) {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }

    let url = manifest.page.to_string();
    eprintln!("[lingxi] opening widget {} -> {}", manifest.id, url);

    let window = WebviewWindowBuilder::new(app, manifest.id, tauri::WebviewUrl::App(url.into()))
        .title(format!("{} · 灵犀", manifest.label))
        .inner_size(manifest.width, manifest.height)
        .resizable(manifest.resizable)
        .decorations(true)
        .always_on_top(manifest.always_on_top)
        .skip_taskbar(false)
        .center()
        .on_page_load(move |window, payload| {
            eprintln!(
                "[lingxi] widget {} page_load: {:?} url={}",
                window.label(),
                payload.event(),
                payload.url()
            );
        })
        .build();

    match window {
        Ok(_w) => {
            eprintln!("[lingxi] widget {} built OK", manifest.id);
            // NOTE: do NOT call open_devtools() here — on Windows, opening
            // DevTools on a freshly created WebView2 window minimizes the
            // host window (observed: window parked at -16000,-16000 with
            // showCmd=SW_SHOWMINIMIZED), which looked like a frozen white
            // window. Debug the pages in a normal browser instead.
            Ok(())
        }
        Err(e) => {
            eprintln!("[lingxi] widget {} build FAILED: {}", manifest.id, e);
            Err(e)
        }
    }
}

/// Close a widget window by id. Uses `destroy()` instead of `close()`:
/// `close()` goes through a CloseRequested round-trip that can stall when
/// the webview's JS is busy (observed as "关不掉"), while `destroy()` closes
/// the window immediately.
pub fn close_widget(app: &AppHandle, id: &str) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(id) {
        window.destroy()?;
    }
    Ok(())
}

/// Check which widgets are currently open.
pub fn open_widget_ids(app: &AppHandle) -> Vec<String> {
    let ids: HashMap<String, ()> = builtin_widgets()
        .iter()
        .map(|w| (w.id.to_string(), ()))
        .collect();
    app.webview_windows()
        .keys()
        .filter(|k| ids.contains_key(k.as_str()))
        .cloned()
        .collect()
}
