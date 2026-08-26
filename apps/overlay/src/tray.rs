//! 托盘图标与菜单：面板显示/隐藏、小工具子菜单、退出。
//!
//! 菜单项回调在主线程执行，从那里同步创建 WebView2 窗口会死锁
//! （见 widgets::open_widget 的注释），因此小工具一律在独立线程打开。

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, SubmenuBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

pub(crate) fn install_tray(app: &AppHandle) -> tauri::Result<()> {
    eprintln!("[lingxi] install_tray: creating menu...");
    let show_panel = MenuItem::with_id(app, "tray:show", "显示面板", true, None::<&str>)?;
    let hide_panel = MenuItem::with_id(app, "tray:hide", "隐藏面板", true, None::<&str>)?;

    // Build widget submenu items dynamically from the builtin catalog.
    let widget_items: Vec<_> = crate::widgets::builtin_widgets()
        .iter()
        .map(|w| {
            MenuItem::with_id(
                app,
                format!("widget:{}", w.id),
                w.label.to_string(),
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
                // WebView2 window from there deadlocks (see open_widget_window).
                let widget_id = id[7..].to_string();
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Some(manifest) = crate::widgets::builtin_widgets()
                        .into_iter()
                        .find(|w| w.id == widget_id)
                    {
                        if let Err(e) = crate::widgets::open_widget_window(&app, &manifest) {
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
