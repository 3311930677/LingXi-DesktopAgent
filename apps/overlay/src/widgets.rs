//! Widget framework: manifest, registry, and multi-window management.
//!
//! Each widget is a small independent Tauri window with its own HTML page,
//! launched on demand via `open_widget`. The manifest declares the widget's
//! metadata (label, shortcut, size) and is shared between the tray menu,
//! the tools grid, and the hotkey system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindowBuilder};

use crate::state::{AppState, MutexExt};

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
pub fn open_widget_window(app: &AppHandle, manifest: &WidgetManifest) -> tauri::Result<()> {
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
pub fn close_widget_window(app: &AppHandle, id: &str) -> tauri::Result<()> {
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

// ---------------------------------------------------------------------------
// Tauri commands for the widget system (moved from main.rs)
// ---------------------------------------------------------------------------

#[tauri::command]
pub(crate) fn list_widgets() -> Vec<WidgetManifest> {
    builtin_widgets()
}

/// Open a widget window. MUST NOT run on the main thread: while the main
/// thread is inside an IPC dispatch or tray-menu callback it cannot pump
/// messages, and `WebviewWindowBuilder::build()` needs the main thread to
/// complete — calling it synchronously deadlocks the whole app (window
/// never appears, tray quit stops working). The background-thread path
/// (hotkey worker / verify mode) never deadlocks, so route every caller
/// through a spawned thread.
#[tauri::command]
pub(crate) async fn open_widget(app: AppHandle, id: String) -> Result<(), String> {
    let manifest = builtin_widgets()
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| format!("未知小工具: {id}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        open_widget_window(&app, &manifest).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("打开小工具任务失败: {e}"))?
}

#[tauri::command]
pub(crate) async fn close_widget(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        close_widget_window(&app, &id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("关闭小工具任务失败: {e}"))?
}

/// Return the ids of widget windows currently open. Used by the tools grid to
/// mark already-open widgets without re-querying all webview windows.
#[tauri::command]
pub(crate) fn list_open_widgets(app: AppHandle) -> Vec<String> {
    open_widget_ids(&app)
}

/// Capture the full screen and return as a PNG data URL.
///
/// Runs on a blocking thread (spawn_blocking) so the widget WebView does not
/// freeze while BitBlt + PNG encode runs. A 10s timeout guards against GDI hangs.
#[tauri::command]
pub(crate) async fn widget_capture_screen() -> Result<serde_json::Value, String> {
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
pub(crate) async fn widget_ocr(x: i32, y: i32, w: i32, h: i32) -> Result<serde_json::Value, String> {
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
pub(crate) async fn widget_pick_color(app: AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON};
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

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
pub(crate) async fn widget_get_weather(city: Option<String>) -> Result<serde_json::Value, String> {
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
pub(crate) async fn widget_calculate(expression: String) -> Result<serde_json::Value, String> {
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
pub(crate) async fn widget_translate(
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
pub(crate) async fn widget_capture_and_translate(
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
pub(crate) struct ClipboardEntry {
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
pub(crate) fn spawn_clipboard_listener() {
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
pub(crate) fn widget_clipboard_history() -> Result<Vec<ClipboardEntry>, String> {
    let history = clipboard_history().lock().map_err(|e| format!("锁失败: {e}"))?;
    Ok(history.clone())
}

#[tauri::command]
pub(crate) fn widget_clipboard_write(text: String) -> Result<(), String> {
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
pub(crate) fn widget_clipboard_clear() -> Result<(), String> {
    let mut history = clipboard_history().lock().map_err(|e| format!("锁失败: {e}"))?;
    history.clear();
    Ok(())
}

/// Remove a single entry (first match by text) so the delete button in the
/// clipboard widget actually persists.
#[tauri::command]
pub(crate) fn widget_clipboard_remove(text: String) -> Result<(), String> {
    let mut history = clipboard_history().lock().map_err(|e| format!("锁失败: {e}"))?;
    if let Some(pos) = history.iter().position(|e| e.text == text) {
        history.remove(pos);
    }
    Ok(())
}

/// Read current system clipboard text (for clipboard-history polling).
#[tauri::command]
pub(crate) fn widget_read_clipboard() -> Result<String, String> {
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
