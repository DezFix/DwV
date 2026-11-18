//! System notifications for finished and failed downloads
//! (setting "show_notifications").

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::commands::AppState;
use crate::i18n;

pub enum Outcome<'a> {
    Done,
    Failed(&'a str),
}

/// Shows a notification unless the user turned them off or is looking at
/// the window already (the list shows the change there).
pub fn download_finished(app: &AppHandle, filename: &str, outcome: Outcome<'_>) {
    let enabled = app
        .try_state::<AppState>()
        .map(|state| state.settings.lock().unwrap().show_notifications)
        .unwrap_or(false);
    if !enabled || window_in_focus(app) {
        return;
    }
    let (title, body) = match outcome {
        Outcome::Done => (i18n::t("notify.done"), filename.to_string()),
        Outcome::Failed(error) => (
            i18n::t("notify.failed"),
            format!("{filename}\n{}", shorten(error, 160)),
        ),
    };
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        log::warn!("notification failed: {error}");
    }
}

fn window_in_focus(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|window| {
        window.is_visible().unwrap_or(false)
            && !window.is_minimized().unwrap_or(false)
            && window.is_focused().unwrap_or(false)
    })
}

fn shorten(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    if line.chars().count() <= max {
        line.to_string()
    } else {
        let cut: String = line.chars().take(max).collect();
        format!("{cut}…")
    }
}
