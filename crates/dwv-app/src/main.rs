slint::include_modules!();

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use dwv_core::{Database, DownloadSpec, Queue, QueueConfig, QueueEvent, State, StoredDownload};
use slint::{ComponentHandle, Model, VecModel};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;
    let model = Rc::new(VecModel::<DownloadItem>::default());
    ui.set_downloads(model.clone().into());

    let db = open_db();
    ui.set_save_dir_text(load_save_dir(db.as_ref()).display().to_string().into());
    fill_history(&model, db.as_ref());

    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let (queue, mut events) = Queue::start(QueueConfig::default(), db.clone());

    let (ipc_tx, mut ipc_rx) = tokio::sync::mpsc::unbounded_channel();
    runtime.spawn(async move {
        if let Err(error) = dwv_ipc::serve(dwv_ipc::DEFAULT_PORT, ipc_tx).await {
            eprintln!("browser integration disabled: {error}");
        }
    });

    let ui_weak = ui.as_weak();
    let ipc_queue = queue.clone();
    let ipc_handle = runtime.handle().clone();
    runtime.spawn(async move {
        while let Some(request) = ipc_rx.recv().await {
            let dwv_ipc::Request::Add {
                url,
                filename,
                referrer,
            } = request
            else {
                continue;
            };
            let ui_weak = ui_weak.clone();
            let queue = ipc_queue.clone();
            let handle = ipc_handle.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else { return };
                let dir = PathBuf::from(ui.get_save_dir_text().as_str());
                match spec_from_browser(&url, filename.as_deref(), referrer.as_deref(), &dir) {
                    Ok(spec) => {
                        handle.spawn(async move {
                            let _ = queue.add(spec).await;
                        });
                    }
                    Err(error) => ui.set_error_text(error.into()),
                }
            });
        }
    });

    let ui_weak = ui.as_weak();
    runtime.spawn(async move {
        while let Some(event) = events.recv().await {
            let ui_weak = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    apply_event(&ui, event);
                }
            });
        }
    });

    ui.on_download_url({
        let ui_weak = ui.as_weak();
        let queue = queue.clone();
        let handle = runtime.handle().clone();
        move |url| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let dir = PathBuf::from(ui.get_save_dir_text().as_str());
            match spec_from_url(url.as_str(), &dir) {
                Ok(spec) => {
                    let queue = queue.clone();
                    handle.spawn(async move {
                        let _ = queue.add(spec).await;
                    });
                }
                Err(error) => ui.set_error_text(error.into()),
            }
        }
    });

    ui.on_download_video({
        let ui_weak = ui.as_weak();
        let handle = runtime.handle().clone();
        let video_seq = Rc::new(Cell::new(0i32));
        move |url| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let url = url.trim().to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                ui.set_error_text("only http:// and https:// URLs are supported".into());
                return;
            }
            let dir = PathBuf::from(ui.get_save_dir_text().as_str());
            // video rows get far-away negative ids so they never collide with the queue or history
            let id = -(1_000_000 + video_seq.get());
            video_seq.set(video_seq.get() + 1);
            push_item(
                &ui,
                DownloadItem {
                    id,
                    name: url_host(&url).into(),
                    progress: 0.0,
                    detail: "video · starting".into(),
                    active: false,
                },
            );
            let ui_weak = ui_weak.clone();
            handle.spawn(async move {
                let progress_weak = ui_weak.clone();
                let mut last_emit = Instant::now() - Duration::from_secs(1);
                let result = dwv_media::download_video(&url, &dir, |progress| {
                    if last_emit.elapsed() < Duration::from_millis(250) {
                        return;
                    }
                    last_emit = Instant::now();
                    let ui_weak = progress_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(ui) = ui_weak.upgrade() else { return };
                        with_item(&ui, id, |item| {
                            item.progress = progress
                                .total
                                .map(|total| {
                                    (progress.downloaded as f64 / total.max(1) as f64).min(1.0)
                                        as f32
                                })
                                .unwrap_or(0.0);
                            let total = progress
                                .total
                                .map(|total| format!(" / {}", human_bytes(total)))
                                .unwrap_or_default();
                            let speed = match progress.speed_bps {
                                Some(speed) => format!("{}/s", human_bytes(speed)),
                                None => "?/s".to_string(),
                            };
                            item.detail =
                                format!("{}{} · {speed}", human_bytes(progress.downloaded), total)
                                    .into();
                        });
                    });
                })
                .await;
                let outcome = match result {
                    Ok(path) => {
                        let name = path.and_then(|path| {
                            path.file_name().map(|n| n.to_string_lossy().into_owned())
                        });
                        Ok(name)
                    }
                    Err(error) => Err(error.to_string()),
                };
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui_weak.upgrade() else { return };
                    with_item(&ui, id, |item| match &outcome {
                        Ok(name) => {
                            item.progress = 1.0;
                            if let Some(name) = name {
                                item.name = name.as_str().into();
                            }
                            item.detail = "Finished".into();
                        }
                        Err(error) => {
                            item.detail = format!("Failed: {error}").into();
                        }
                    });
                });
            });
        }
    });

    ui.on_cancel_download({
        let queue = queue.clone();
        move |id| queue.cancel(id as u64)
    });

    ui.on_choose_folder({
        let ui_weak = ui.as_weak();
        let db = db.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let start_dir = PathBuf::from(ui.get_save_dir_text().as_str());
            let ui_weak = ui.as_weak();
            let db = db.clone();
            // rfd blocks its thread while the dialog is open; keep it off the UI loop
            std::thread::spawn(move || {
                let Some(dir) = rfd::FileDialog::new()
                    .set_directory(&start_dir)
                    .pick_folder()
                else {
                    return;
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(db) = db.as_ref() {
                        let _ = db.set_setting("download_dir", &dir.to_string_lossy());
                    }
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_save_dir_text(dir.display().to_string().into());
                    }
                });
            });
        }
    });

    ui.run()?;
    Ok(())
}

fn push_item(ui: &AppWindow, item: DownloadItem) {
    let downloads = ui.get_downloads();
    let Some(model) = downloads.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    model.insert(0, item);
}

fn with_item(ui: &AppWindow, id: i32, apply: impl FnOnce(&mut DownloadItem)) {
    let downloads = ui.get_downloads();
    let Some(model) = downloads.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    let index = (0..model.row_count()).find(|&i| model.row_data(i).is_some_and(|d| d.id == id));
    if let Some(i) = index {
        let mut item = model.row_data(i).unwrap();
        apply(&mut item);
        model.set_row_data(i, item);
    }
}

fn url_host(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if host.is_empty() {
        "video".to_string()
    } else {
        host.to_string()
    }
}

fn fill_history(model: &VecModel<DownloadItem>, db: Option<&Database>) {
    let Some(db) = db else { return };
    // rows left over from a crash would otherwise stay "downloading" forever
    for row in db.load_pending().unwrap_or_default() {
        let _ = db.set_error(row.id, "interrupted");
        let _ = db.finish(row.id, State::Failed);
    }
    for row in db.recent(200).unwrap_or_default() {
        model.push(history_item(row));
    }
}

fn history_item(row: StoredDownload) -> DownloadItem {
    let name = row
        .target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| row.target.display().to_string());
    let progress = match row.state {
        State::Completed => 1.0,
        _ => row
            .total_bytes
            .map(|total| (row.downloaded_bytes as f64 / total.max(1) as f64).min(1.0) as f32)
            .unwrap_or(0.0),
    };
    let detail = match row.state {
        State::Completed => format!(
            "Finished · {}",
            human_bytes(row.total_bytes.unwrap_or(row.downloaded_bytes))
        ),
        State::Failed => format!(
            "Failed: {}",
            row.error.unwrap_or_else(|| "unknown".to_string())
        ),
        State::Cancelled => match row.total_bytes {
            Some(total) => format!(
                "Cancelled · {} / {}",
                human_bytes(row.downloaded_bytes),
                human_bytes(total)
            ),
            None => "Cancelled".to_string(),
        },
        State::Queued | State::Downloading => "Interrupted".to_string(),
    };
    DownloadItem {
        // negative ids keep history rows from colliding with live queue ids
        id: -(row.id as i32),
        name: name.into(),
        progress,
        detail: detail.into(),
        active: false,
    }
}

fn apply_event(ui: &AppWindow, event: QueueEvent) {
    let downloads = ui.get_downloads();
    let Some(model) = downloads.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    let id = match &event {
        QueueEvent::Started { id, .. }
        | QueueEvent::Progress { id, .. }
        | QueueEvent::Finished { id, .. }
        | QueueEvent::Failed { id, .. }
        | QueueEvent::Cancelled { id } => *id,
    } as i32;
    let index = (0..model.row_count()).find(|&i| model.row_data(i).is_some_and(|d| d.id == id));

    match event {
        QueueEvent::Started { path, total, .. } => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let detail = match total {
                Some(total) => format!("0 B / {}", human_bytes(total)),
                None => "unknown size".to_string(),
            };
            let item = DownloadItem {
                id,
                name: name.into(),
                progress: 0.0,
                detail: detail.into(),
                active: true,
            };
            match index {
                Some(i) => model.set_row_data(i, item),
                None => model.insert(0, item),
            }
        }
        QueueEvent::Progress {
            downloaded,
            total,
            speed_bps,
            ..
        } => {
            let Some(i) = index else { return };
            let mut item = model.row_data(i).unwrap();
            item.progress = total
                .map(|total| (downloaded as f64 / total.max(1) as f64).min(1.0) as f32)
                .unwrap_or(0.0);
            item.detail = match total {
                Some(total) => format!(
                    "{} / {} · {}/s",
                    human_bytes(downloaded),
                    human_bytes(total),
                    human_bytes(speed_bps)
                ),
                None => format!("{} · {}/s", human_bytes(downloaded), human_bytes(speed_bps)),
            }
            .into();
            model.set_row_data(i, item);
        }
        QueueEvent::Finished { .. } => {
            finish_item(model, index, |item| {
                item.progress = 1.0;
                item.detail = "Finished".into();
            });
        }
        QueueEvent::Failed { error, .. } => {
            finish_item(model, index, |item| {
                item.detail = format!("Failed: {error}").into();
            });
        }
        QueueEvent::Cancelled { .. } => {
            finish_item(model, index, |item| {
                item.detail = "Cancelled".into();
            });
        }
    }
}

fn finish_item(
    model: &VecModel<DownloadItem>,
    index: Option<usize>,
    update: impl FnOnce(&mut DownloadItem),
) {
    let Some(i) = index else { return };
    let mut item = model.row_data(i).unwrap();
    item.active = false;
    update(&mut item);
    model.set_row_data(i, item);
}

fn spec_from_url(url: &str, dir: &Path) -> Result<DownloadSpec, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http:// and https:// URLs are supported".to_string());
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let raw_name = path.rsplit('/').next().unwrap_or("").trim();
    // shortcut: percent-encoding left as-is, decode when it becomes a problem
    let name = sanitize(if raw_name.is_empty() {
        "download"
    } else {
        raw_name
    });
    let mut spec = DownloadSpec::new(url, unique_path(&dir.join(&name)));
    spec.connections = 8;
    Ok(spec)
}

fn spec_from_browser(
    url: &str,
    filename: Option<&str>,
    referrer: Option<&str>,
    dir: &Path,
) -> Result<DownloadSpec, String> {
    let mut spec = spec_from_url(url, dir)?;
    if let Some(name) = filename {
        spec.target = unique_path(&dir.join(sanitize(name)));
    }
    if let Some(referrer) = referrer.filter(|r| !r.is_empty()) {
        spec.headers
            .push(("Referer".to_string(), referrer.to_string()));
    }
    Ok(spec)
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .take(150)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_end_matches([' ', '.']);
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned.to_string()
    }
}

fn load_save_dir(db: Option<&Database>) -> PathBuf {
    db.and_then(|db| db.get_setting("download_dir").ok().flatten())
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(default_download_dir)
}

fn default_download_dir() -> PathBuf {
    // engine creates the directory on first download
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join("Downloads"))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 1..1000 {
        let name = match &ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

fn open_db() -> Option<Database> {
    let dir = std::env::var("APPDATA").ok()?;
    Database::open(&PathBuf::from(dir).join("DwV").join("dwv.db")).ok()
}

// shortcut: duplicated in dwv-cli, move to dwv-core when a third caller appears
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_reserved_characters() {
        assert_eq!(sanitize("a:b?c*d"), "a_b_c_d");
        assert_eq!(sanitize("trailing. "), "trailing");
        assert_eq!(sanitize(""), "download");
    }

    #[test]
    fn spec_takes_name_from_url() {
        let dir = tempfile::tempdir().unwrap();
        let spec = spec_from_url(
            "https://example.com/dir/file.zip?token=abc#frag",
            dir.path(),
        )
        .unwrap();
        assert_eq!(spec.url, "https://example.com/dir/file.zip?token=abc#frag");
        assert_eq!(spec.target, dir.path().join("file.zip"));
        assert_eq!(spec.connections, 8);

        assert!(spec_from_url("ftp://example.com/file.zip", dir.path()).is_err());
    }

    #[test]
    fn spec_avoids_overwriting_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.zip"), b"x").unwrap();
        let spec = spec_from_url("https://example.com/file.zip", dir.path()).unwrap();
        assert_eq!(spec.target, dir.path().join("file (1).zip"));
    }

    #[test]
    fn browser_spec_uses_filename_and_referrer() {
        let dir = tempfile::tempdir().unwrap();
        let spec = spec_from_browser(
            "https://example.com/download?id=1",
            Some("Video (1080p).mp4"),
            Some("https://example.com/watch"),
            dir.path(),
        )
        .unwrap();
        assert_eq!(spec.target, dir.path().join("Video (1080p).mp4"));
        assert_eq!(
            spec.headers,
            vec![(
                "Referer".to_string(),
                "https://example.com/watch".to_string()
            )]
        );
    }
}
