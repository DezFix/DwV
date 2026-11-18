//! The local endpoint the browser extension (and a second app instance)
//! posts links to. Transport and validation live in `dwv-ipc`; this module
//! turns requests into downloads.

use std::path::Path;

use tauri::{AppHandle, Emitter};

use crate::commands::{
    AppState, DownloadContext, PendingPayload, add_download_with_context, add_video_inner,
    emit_added_with_requested, fetch_torrent_path, queue_pending, should_ask_path,
};
use crate::names::file_name_of;

/// Starts the server. Each request is handled on its own task: resolving a
/// file name or fetching a `.torrent` can take seconds and must not hold up
/// the next link from the browser.
pub fn start_ipc_server(app: AppHandle, state: AppState) {
    let handle = state.handle.clone();
    handle.spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<dwv_ipc::Request>();

        tokio::spawn(async move {
            if let Err(error) = dwv_ipc::serve(dwv_ipc::DEFAULT_PORT, tx).await {
                log::error!("IPC server failed to start: {error}");
            }
        });

        while let Some(request) = rx.recv().await {
            // Any request from the extension means something is happening; the
            // window may be hidden in the tray, so bring it up as feedback.
            crate::tray::show_window(&app);
            let app = app.clone();
            let state = state.clone();
            tokio::spawn(async move {
                handle_request(&state, &app, request).await;
            });
        }
    });
}

async fn handle_request(state: &AppState, app: &AppHandle, request: dwv_ipc::Request) {
    match request {
        dwv_ipc::Request::Add {
            url,
            filename,
            referrer,
        } => {
            let context = DownloadContext {
                url,
                filename,
                referrer,
                ..Default::default()
            };
            add_file(state, app, context).await;
        }
        dwv_ipc::Request::Download {
            url,
            final_url,
            filename,
            referrer,
            cookies,
            user_agent,
            headers,
        } => {
            let context = DownloadContext {
                url,
                final_url,
                filename,
                referrer,
                user_agent,
                cookies,
                headers,
                dir: None,
            };
            add_file(state, app, context).await;
        }
        dwv_ipc::Request::Torrent {
            source,
            referrer,
            cookies,
            user_agent,
        } => {
            // The file picker is mandatory for torrents, so queue them even
            // when the folder dialog is disabled.
            let source = if source.starts_with("http://") || source.starts_with("https://") {
                match fetch_torrent_path(
                    state,
                    &source,
                    referrer.as_deref(),
                    cookies.as_deref(),
                    user_agent.as_deref(),
                )
                .await
                {
                    Some(path) => path.to_string_lossy().to_string(),
                    None => source,
                }
            } else {
                source
            };
            let name = if Path::new(&source).is_file() {
                file_name_of(Path::new(&source))
                    .trim_end_matches(".torrent")
                    .to_string()
            } else {
                dwv_torrent::torrent_name(&source)
            };
            queue_pending(
                state,
                app,
                PendingPayload::Torrent(source.clone()),
                "torrent",
                &source,
                Some(name),
            );
        }
        dwv_ipc::Request::Video { url, quality, .. } => {
            if should_ask_path(state) {
                queue_pending(
                    state,
                    app,
                    PendingPayload::Video(url.clone()),
                    "video",
                    &url,
                    None,
                );
                return;
            }
            if let Err(error) =
                add_video_inner(state, app, &url, None, quality.as_deref(), None).await
            {
                report(app, &url, &error);
            }
        }
        dwv_ipc::Request::Ping => {}
    }
}

async fn add_file(state: &AppState, app: &AppHandle, context: DownloadContext) {
    if should_ask_path(state) {
        let url = context.url.clone();
        let filename = context.filename.clone();
        queue_pending(
            state,
            app,
            PendingPayload::File(context),
            "file",
            &url,
            filename,
        );
        return;
    }
    let requested = context.filename.clone();
    let url = context.url.clone();
    match add_download_with_context(state, context).await {
        Ok(id) => emit_added_with_requested(state, app, id as i64, requested.as_deref()),
        Err(error) => report(app, &url, &error),
    }
}

/// The extension already got "ok" (the request was valid), so a failure to
/// start the download is shown in the app instead of being dropped.
fn report(app: &AppHandle, url: &str, error: &str) {
    log::warn!("download from the browser failed: {error}");
    let _ = app.emit(
        "add-failed",
        serde_json::json!({ "url": url, "error": error }),
    );
}
