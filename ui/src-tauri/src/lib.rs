use crate::commands::{AppState, app_data_dir, resume_pending, spawn_event_bridge};
use tauri::Manager;

pub mod commands;
mod i18n;
mod settings;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let is_autostart = args.iter().any(|arg| arg == "--autostart");
    let initial_link = args
        .iter()
        .find(|a| a.starts_with("dwv://") || a.starts_with("magnet:"))
        .cloned();

    crate::commands::register_protocol();

    if try_forward(initial_link.as_deref()) {
        std::process::exit(0);
    }

    let initial_url = initial_link.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(move |app| {
            let data_dir = app_data_dir(app.handle());
            std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
            let config_dir = app
                .path()
                .app_config_dir()
                .map_err(|e| format!("No config dir: {e}"))?;

            let (state, events) = AppState::new(config_dir.join("settings.json"), data_dir);
            let downloads = state.downloads.clone();
            let handle = state.handle.clone();
            let link_state = state.clone();
            let app_handle = app.handle().clone();
            app.manage(state);

            // The core queue does the real work; this task feeds the UI.
            spawn_event_bridge(app.handle().clone(), downloads, events, handle);

            let ipc_state = app.state::<AppState>().inner().clone();
            crate::commands::start_ipc_server(app.handle().clone(), ipc_state);

            resume_pending(&app.state::<AppState>());

            if is_autostart {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            if let Some(link) = initial_url {
                if let Some((target, kind)) = launch_target(&link) {
                    if !target.is_empty() {
                        let state = link_state.clone();
                        let handle = app_handle.clone();
                        let rt = state.handle.clone();
                        rt.spawn(async move {
                            let ask = state.settings.lock().unwrap().ask_save_path;
                            let torrent =
                                kind.as_deref() == Some("torrent") || target.starts_with("magnet:");
                            if ask || torrent {
                                crate::commands::queue_deep_link(
                                    &state,
                                    &handle,
                                    &target,
                                    kind.as_deref(),
                                );
                            } else {
                                let _ = crate::commands::add_link(
                                    &state,
                                    &handle,
                                    &target,
                                    kind.as_deref(),
                                )
                                .await;
                            }
                        });
                    }
                }
            }

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let close_to_tray = window
                    .app_handle()
                    .state::<AppState>()
                    .settings
                    .lock()
                    .unwrap()
                    .close_to_tray;
                if close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            tauri::WindowEvent::Resized(_) => {
                let minimize_to_tray = window
                    .app_handle()
                    .state::<AppState>()
                    .settings
                    .lock()
                    .unwrap()
                    .minimize_to_tray;
                if minimize_to_tray && window.is_minimized().unwrap_or(false) {
                    let _ = window.unminimize();
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_download,
            commands::preview_download,
            commands::add_download_as,
            commands::add_video,
            commands::add_torrent,
            commands::inspect_torrent,
            commands::pending_downloads,
            commands::resolve_pending,
            commands::cancel_pending,
            commands::default_download_dir,
            commands::get_downloads,
            commands::get_download_details,
            commands::pause_download,
            commands::resume_download,
            commands::remove_download,
            commands::pause_all,
            commands::resume_all,
            commands::clear_completed,
            commands::get_settings,
            commands::save_settings,
            commands::import_settings,
            commands::clear_cache,
            commands::reset_database,
            commands::check_updates,
            commands::open_file,
            commands::open_url,
            commands::show_in_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app, event| {
            if let tauri::RunEvent::Ready = event {
                let _ = tray::build(app);
                let state = app.state::<AppState>();
                let settings = state.settings.lock().unwrap().clone();
                i18n::set_locale(&settings.language);
                commands::apply_limits(&state, &settings);
                commands::apply_autostart(settings.autostart);
            }
        });
}

fn launch_target(link: &str) -> Option<(String, Option<String>)> {
    if link.starts_with("magnet:") {
        return Some((link.to_string(), Some("torrent".to_string())));
    }
    let target = crate::commands::parse_deep_link(link)?;
    let kind = crate::commands::deep_link_type(link);
    Some((target, kind))
}

fn try_forward(link: Option<&str>) -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let addr: std::net::SocketAddr = match "127.0.0.1:41763".parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let body = if let Some(link) = link {
        if link.starts_with("magnet:") {
            serde_json::json!({"type":"torrent","source":link})
        } else {
            match launch_target(link) {
                Some((target, kind)) if !target.is_empty() => {
                    let kind = kind.unwrap_or_default();
                    if kind == "video" {
                        serde_json::json!({"type":"video","url":target})
                    } else if kind == "torrent"
                        || (kind != "file" && crate::commands::is_torrent_source(&target))
                    {
                        serde_json::json!({"type":"torrent","source":target})
                    } else {
                        serde_json::json!({"type":"add","url":target})
                    }
                }
                _ => serde_json::json!({"type":"ping"}),
            }
        }
    } else {
        serde_json::json!({"type":"ping"})
    };
    let body = body.to_string();
    let req = format!(
        "POST / HTTP/1.1\r
Host: 127.0.0.1:41763\r
Origin: chrome-extension://dwv\r
Content-Type: application/json\r
Content-Length: {}\r
Connection: close\r
\r
{}",
        body.len(),
        body
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    buf.contains("\"ok\":true")
}
