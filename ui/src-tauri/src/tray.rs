use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::AppState;
use crate::i18n;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", i18n::t("tray.show"), true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", i18n::t("tray.pause_all"), true, None::<&str>)?;
    let resume = MenuItem::with_id(
        app,
        "resume",
        i18n::t("tray.resume_all"),
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(
        app,
        "settings",
        i18n::t("menu.settings"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", i18n::t("menu.quit"), true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&show, &pause, &resume, &settings, &separator, &quit])?;

    let builder = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("DwV")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_window(app),
            "settings" => {
                show_window(app);
                let _ = app.emit("open-settings", ());
            }
            "pause" => {
                let state = app.state::<AppState>().inner().clone();
                let app = app.clone();
                let handle = state.handle.clone();
                handle.spawn(async move {
                    let _ = crate::commands::pause_all_inner(&state, &app).await;
                });
            }
            "resume" => {
                let state = app.state::<AppState>().inner().clone();
                let app = app.clone();
                let handle = state.handle.clone();
                handle.spawn(async move {
                    let _ = crate::commands::resume_all_inner(&state, &app).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });

    builder.build(app)?;
    Ok(())
}

pub(crate) fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
