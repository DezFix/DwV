slint::include_modules!();

use std::cell::Cell;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use dwv_core::{Database, DownloadSpec, Queue, QueueConfig, QueueEvent, State, StoredDownload};
use dwv_media::VideoQuality;
use slint::{ComponentHandle, Model, VecModel};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;
    let all = Rc::new(VecModel::<DownloadItem>::default());
    ui.set_all_downloads(all.clone().into());
    ui.set_downloads(Rc::new(VecModel::<DownloadItem>::default()).into());

    let db = open_db();
    ui.set_save_dir_text(load_save_dir(db.as_ref()).display().to_string().into());
    fill_history(&all, db.as_ref());
    refilter(&ui);

    let connections = db
        .as_ref()
        .and_then(|db| db.get_setting("connections").ok().flatten())
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(8)
        .clamp(1, 32);
    let speed_limit = db
        .as_ref()
        .and_then(|db| db.get_setting("speed_limit").ok().flatten())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&value| value > 0);
    ui.set_connections(connections);
    ui.set_speed_limit_kbps(speed_limit.map(|value| (value / 1024) as i32).unwrap_or(0));
    let quality_index = db
        .as_ref()
        .and_then(|db| db.get_setting("video_quality").ok().flatten())
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(0)
        .clamp(0, 3);
    ui.set_video_quality(quality_index);

    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let (queue, mut events) = Queue::start(
        QueueConfig {
            speed_limit,
            ..QueueConfig::default()
        },
        db.clone(),
    );

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
                let connections = ui.get_connections().clamp(1, 32) as usize;
                match spec_from_browser(
                    &url,
                    filename.as_deref(),
                    referrer.as_deref(),
                    &dir,
                    connections,
                ) {
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

    ui.on_add_link({
        let ui_weak = ui.as_weak();
        let queue = queue.clone();
        let handle = runtime.handle().clone();
        move |text| {
            let Some(ui) = ui_weak.upgrade() else { return };
            add_link(&ui, &queue, &handle, text.as_str());
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
            let dir_text = dir.display().to_string();
            let quality = video_quality(ui.get_video_quality());
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
                    paused: false,
                    kind: "video".into(),
                    state: "Active".into(),
                    speed_bps: 0,
                    path: "".into(),
                    folder: dir_text.into(),
                },
            );
            let ui_weak = ui_weak.clone();
            handle.spawn(async move {
                let progress_weak = ui_weak.clone();
                let mut last_emit = Instant::now() - Duration::from_secs(1);
                let result = dwv_media::download_video(&url, &dir, quality, |progress| {
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
                            item.speed_bps = progress.speed_bps.unwrap_or(0) as i32;
                            item.detail =
                                format!("{}{} · {speed}", human_bytes(progress.downloaded), total)
                                    .into();
                        });
                    });
                })
                .await;
                let outcome = match result {
                    Ok(path) => {
                        let file = path.map(|path| path.to_string_lossy().into_owned());
                        let name = file.as_deref().and_then(|file| {
                            Path::new(file)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                        });
                        Ok((name, file))
                    }
                    Err(error) => Err(error.to_string()),
                };
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui_weak.upgrade() else { return };
                    with_item(&ui, id, |item| match &outcome {
                        Ok((name, file)) => {
                            item.progress = 1.0;
                            if let Some(name) = name {
                                item.name = name.as_str().into();
                            }
                            if let Some(file) = file {
                                item.path = file.as_str().into();
                                item.folder = Path::new(file)
                                    .parent()
                                    .map(|dir| dir.display().to_string())
                                    .unwrap_or_default()
                                    .into();
                            }
                            item.detail = "Finished".into();
                            item.state = "Finished".into();
                        }
                        Err(error) => {
                            item.detail = format!("Failed: {error}").into();
                            item.state = "Failed".into();
                        }
                    });
                });
            });
        }
    });

    ui.on_download_torrent({
        let ui_weak = ui.as_weak();
        let handle = runtime.handle().clone();
        let torrent_seq = Rc::new(Cell::new(0i32));
        move |source| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let source = source.trim().to_string();
            let is_local =
                source.to_lowercase().ends_with(".torrent") && Path::new(&source).is_file();
            if !(source.starts_with("magnet:")
                || source.starts_with("http://")
                || source.starts_with("https://")
                || is_local)
            {
                ui.set_error_text(
                    "torrent source must be a magnet link, a .torrent URL or a local .torrent file"
                        .into(),
                );
                return;
            }
            let dir = PathBuf::from(ui.get_save_dir_text().as_str());
            let dir_text = dir.display().to_string();
            let id = -(2_000_000 + torrent_seq.get());
            torrent_seq.set(torrent_seq.get() + 1);
            push_item(
                &ui,
                DownloadItem {
                    id,
                    name: torrent_label(&source).into(),
                    progress: 0.0,
                    detail: "torrent · starting".into(),
                    active: false,
                    paused: false,
                    kind: "torrent".into(),
                    state: "Active".into(),
                    speed_bps: 0,
                    path: "".into(),
                    folder: dir_text.into(),
                },
            );
            let ui_weak = ui_weak.clone();
            handle.spawn(async move {
                let progress_weak = ui_weak.clone();
                let mut last_emit = Instant::now() - Duration::from_secs(1);
                let result = dwv_torrent::download_torrent(&source, &dir, |progress| {
                    if last_emit.elapsed() < Duration::from_millis(250) {
                        return;
                    }
                    last_emit = Instant::now();
                    let ui_weak = progress_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(ui) = ui_weak.upgrade() else { return };
                        with_item(&ui, id, |item| {
                            item.speed_bps = progress.down_bps as i32;
                            if progress.total == 0 {
                                item.detail = format!(
                                    "getting metadata · {}/s",
                                    human_bytes(progress.down_bps)
                                )
                                .into();
                                return;
                            }
                            item.progress = (progress.downloaded as f64 / progress.total as f64)
                                .min(1.0) as f32;
                            item.detail = format!(
                                "{} / {} · {}/s",
                                human_bytes(progress.downloaded),
                                human_bytes(progress.total),
                                human_bytes(progress.down_bps)
                            )
                            .into();
                        });
                    });
                })
                .await;
                let outcome = match result {
                    Ok(path) => Ok(path.file_name().map(|n| n.to_string_lossy().into_owned())),
                    Err(error) => Err(error.to_string()),
                };
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui_weak.upgrade() else { return };
                    with_item(&ui, id, |item| {
                        item.speed_bps = 0;
                        match &outcome {
                            Ok(name) => {
                                item.progress = 1.0;
                                if let Some(name) = name {
                                    item.name = name.as_str().into();
                                }
                                item.detail = "Finished".into();
                                item.state = "Finished".into();
                            }
                            Err(error) => {
                                item.detail = format!("Failed: {error}").into();
                                item.state = "Failed".into();
                            }
                        }
                    });
                });
            });
        }
    });

    ui.on_pause_download({
        let queue = queue.clone();
        move |id, paused| {
            if id <= 0 {
                return;
            }
            if paused {
                queue.resume(id as u64);
            } else {
                queue.pause(id as u64);
            }
        }
    });

    ui.on_open_item({
        let ui_weak = ui.as_weak();
        move |id| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let Some(item) = find_item(&ui, id) else {
                return;
            };
            if !item.path.is_empty() {
                open_file(Path::new(item.path.as_str()));
            } else if !item.folder.is_empty() {
                open_file(Path::new(item.folder.as_str()));
            }
        }
    });

    ui.on_show_folder({
        let ui_weak = ui.as_weak();
        move |id| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let Some(item) = find_item(&ui, id) else {
                return;
            };
            if !item.path.is_empty() {
                reveal_path(Path::new(item.path.as_str()));
            } else if !item.folder.is_empty() {
                open_file(Path::new(item.folder.as_str()));
            }
        }
    });

    ui.on_remove_item({
        let ui_weak = ui.as_weak();
        let queue = queue.clone();
        let db = db.clone();
        move |id| {
            let Some(ui) = ui_weak.upgrade() else { return };
            if id > 0 {
                queue.cancel(id as u64);
            } else if id > -1_000_000
                && let Some(db) = db.as_ref()
            {
                let _ = db.delete(-id as i64);
            }
            // shortcut: removing a video/torrent row hides it, the transfer keeps running
            remove_item(&ui, id);
        }
    });

    ui.on_refilter({
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                refilter(&ui);
            }
        }
    });

    ui.on_dropped({
        let ui_weak = ui.as_weak();
        let queue = queue.clone();
        let handle = runtime.handle().clone();
        move |data: slint::DataTransfer| {
            let Some(ui) = ui_weak.upgrade() else { return };
            // shortcut: only the first dropped file is handled
            if data.has_file_paths() {
                if let Ok(mut paths) = data.file_paths()
                    && let Some(path) = paths.next()
                {
                    add_link(&ui, &queue, &handle, &path.to_string_lossy());
                }
                return;
            }
            if !data.has_plain_text() {
                return;
            }
            let Ok(text) = data.plain_text() else { return };
            add_link(&ui, &queue, &handle, text.trim());
        }
    });

    ui.on_save_settings({
        let ui_weak = ui.as_weak();
        let db = db.clone();
        let queue = queue.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let connections = ui.get_connections().clamp(1, 32);
            let kbps = ui.get_speed_limit_kbps().max(0);
            let video_quality = ui.get_video_quality().clamp(0, 3);
            ui.set_connections(connections);
            ui.set_speed_limit_kbps(kbps);
            ui.set_video_quality(video_quality);
            if let Some(db) = db.as_ref() {
                let _ = db.set_setting("connections", &connections.to_string());
                let _ = db.set_setting("speed_limit", &(kbps as u64 * 1024).to_string());
                let _ = db.set_setting("video_quality", &video_quality.to_string());
            }
            queue.set_speed_limit((kbps > 0).then_some(kbps as u64 * 1024));
        }
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

    ui.window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    let tray = AppTray::new()?;
    tray.on_show_window({
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                let _ = ui.show();
            }
        }
    });
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    ui.run()?;
    Ok(())
}

fn find_item(ui: &AppWindow, id: i32) -> Option<DownloadItem> {
    let all = ui.get_all_downloads();
    let model = all.as_any().downcast_ref::<VecModel<DownloadItem>>()?;
    (0..model.row_count()).find_map(|i| model.row_data(i).filter(|item| item.id == id))
}

fn remove_item(ui: &AppWindow, id: i32) {
    let all = ui.get_all_downloads();
    let Some(model) = all.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    if let Some(index) =
        (0..model.row_count()).find(|&i| model.row_data(i).is_some_and(|item| item.id == id))
    {
        model.remove(index);
        refilter(ui);
    }
}

fn open_file(path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

fn reveal_path(path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

fn push_item(ui: &AppWindow, item: DownloadItem) {
    let all = ui.get_all_downloads();
    let Some(model) = all.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    model.insert(0, item);
    refilter(ui);
}

fn with_item(ui: &AppWindow, id: i32, apply: impl FnOnce(&mut DownloadItem)) {
    let all = ui.get_all_downloads();
    let Some(model) = all.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    let index = (0..model.row_count()).find(|&i| model.row_data(i).is_some_and(|d| d.id == id));
    if let Some(i) = index {
        let mut item = model.row_data(i).unwrap();
        apply(&mut item);
        model.set_row_data(i, item);
        refilter(ui);
    }
}

fn refilter(ui: &AppWindow) {
    let all = ui.get_all_downloads();
    let Some(all) = all.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    let shown = ui.get_downloads();
    let Some(shown) = shown.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    let filter = ui.get_filter();
    let search = ui.get_search_text().to_lowercase();
    let mut rows = Vec::new();
    let mut active = 0u32;
    let mut speed = 0u64;
    for i in 0..all.row_count() {
        let Some(item) = all.row_data(i) else {
            continue;
        };
        if item.state == "Active" {
            active += 1;
            speed += item.speed_bps.max(0) as u64;
        }
        if !matches_filter(&item, filter) {
            continue;
        }
        if !search.is_empty()
            && !item.name.to_lowercase().contains(&search)
            && !item.detail.to_lowercase().contains(&search)
        {
            continue;
        }
        rows.push(item);
    }
    let sort_key = ui.get_sort_key();
    if sort_key != 0 {
        rows.sort_by(|a, b| match sort_key {
            1 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            2 => a.state.as_str().cmp(b.state.as_str()),
            3 => a
                .progress
                .partial_cmp(&b.progress)
                .unwrap_or(Ordering::Equal),
            _ => Ordering::Equal,
        });
        if ui.get_sort_desc() {
            rows.reverse();
        }
    }
    shown.set_vec(rows);
    let status = match active {
        0 => "idle".to_string(),
        n => format!("{n} active · {}/s", human_bytes(speed)),
    };
    ui.set_status_text(status.into());
}

fn matches_filter(item: &DownloadItem, filter: i32) -> bool {
    match filter {
        1 => item.state == "Active" || item.state == "Queued",
        2 => item.state == "Paused",
        3 => matches!(
            item.state.as_str(),
            "Finished" | "Failed" | "Cancelled" | "Interrupted"
        ),
        4 => item.kind == "video",
        5 => item.kind == "torrent",
        _ => true,
    }
}

fn video_quality(value: i32) -> VideoQuality {
    match value {
        1 => VideoQuality::P1080,
        2 => VideoQuality::P720,
        3 => VideoQuality::P480,
        _ => VideoQuality::Best,
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

fn torrent_label(source: &str) -> String {
    if source.starts_with("magnet:") {
        return "magnet link".to_string();
    }
    let no_query = source.split(['?', '#']).next().unwrap_or(source);
    let last = no_query.rsplit('/').next().unwrap_or("").trim();
    if last.is_empty() {
        "torrent".to_string()
    } else {
        last.to_string()
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
    let (detail, state) = match row.state {
        State::Completed => (
            format!(
                "Finished · {}",
                human_bytes(row.total_bytes.unwrap_or(row.downloaded_bytes))
            ),
            "Finished",
        ),
        State::Failed => (
            format!(
                "Failed: {}",
                row.error.unwrap_or_else(|| "unknown".to_string())
            ),
            "Failed",
        ),
        State::Cancelled => (
            match row.total_bytes {
                Some(total) => format!(
                    "Cancelled · {} / {}",
                    human_bytes(row.downloaded_bytes),
                    human_bytes(total)
                ),
                None => "Cancelled".to_string(),
            },
            "Cancelled",
        ),
        State::Queued | State::Downloading => ("Interrupted".to_string(), "Interrupted"),
        State::Paused => (
            match row.total_bytes {
                Some(total) => format!(
                    "Paused · {} / {}",
                    human_bytes(row.downloaded_bytes),
                    human_bytes(total)
                ),
                None => "Paused".to_string(),
            },
            "Paused",
        ),
    };
    DownloadItem {
        // negative ids keep history rows from colliding with live queue ids
        id: -(row.id as i32),
        name: name.into(),
        progress,
        detail: detail.into(),
        active: false,
        paused: false,
        kind: "file".into(),
        state: state.into(),
        speed_bps: 0,
        folder: row
            .target
            .parent()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default()
            .into(),
        path: row.target.to_string_lossy().into_owned().into(),
    }
}

fn apply_event(ui: &AppWindow, event: QueueEvent) {
    let all = ui.get_all_downloads();
    let Some(model) = all.as_any().downcast_ref::<VecModel<DownloadItem>>() else {
        return;
    };
    let id = match &event {
        QueueEvent::Started { id, .. }
        | QueueEvent::Progress { id, .. }
        | QueueEvent::Finished { id, .. }
        | QueueEvent::Failed { id, .. }
        | QueueEvent::Cancelled { id }
        | QueueEvent::Paused { id } => *id,
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
                paused: false,
                kind: "file".into(),
                state: "Active".into(),
                speed_bps: 0,
                path: path.to_string_lossy().into_owned().into(),
                folder: path
                    .parent()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_default()
                    .into(),
            };
            match index {
                Some(i) => model.set_row_data(i, item),
                None => model.insert(0, item),
            }
            refilter(ui);
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
            item.speed_bps = speed_bps as i32;
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
            refilter(ui);
        }
        QueueEvent::Finished { .. } => {
            finish_item(model, index, "Finished", |item| {
                item.progress = 1.0;
                item.detail = "Finished".into();
            });
            refilter(ui);
        }
        QueueEvent::Failed { error, .. } => {
            finish_item(model, index, "Failed", |item| {
                item.detail = format!("Failed: {error}").into();
            });
            refilter(ui);
        }
        QueueEvent::Cancelled { .. } => {
            finish_item(model, index, "Cancelled", |item| {
                item.detail = "Cancelled".into();
            });
            refilter(ui);
        }
        QueueEvent::Paused { .. } => {
            let Some(i) = index else { return };
            let mut item = model.row_data(i).unwrap();
            item.paused = true;
            item.state = "Paused".into();
            item.detail = "Paused".into();
            item.speed_bps = 0;
            model.set_row_data(i, item);
            refilter(ui);
        }
    }
}

fn finish_item(
    model: &VecModel<DownloadItem>,
    index: Option<usize>,
    state: &str,
    update: impl FnOnce(&mut DownloadItem),
) {
    let Some(i) = index else { return };
    let mut item = model.row_data(i).unwrap();
    item.active = false;
    item.paused = false;
    item.state = state.into();
    item.speed_bps = 0;
    update(&mut item);
    model.set_row_data(i, item);
}

#[derive(Debug, PartialEq, Eq)]
enum LinkKind {
    File,
    Video,
    Torrent,
}

fn classify_link(text: &str) -> Result<LinkKind, &'static str> {
    if Path::new(text).is_file() {
        return if text.to_lowercase().ends_with(".torrent") {
            Ok(LinkKind::Torrent)
        } else {
            Err("only .torrent files can be opened from disk")
        };
    }
    let lower = text.to_lowercase();
    if lower.starts_with("magnet:") {
        return Ok(LinkKind::Torrent);
    }
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("expected a URL, a magnet link or a .torrent file");
    }
    if lower
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .ends_with(".torrent")
    {
        return Ok(LinkKind::Torrent);
    }
    if is_video_url(&lower) {
        return Ok(LinkKind::Video);
    }
    Ok(LinkKind::File)
}

fn add_link(ui: &AppWindow, queue: &Queue, handle: &tokio::runtime::Handle, raw: &str) {
    let text = raw.trim();
    if text.is_empty() {
        return;
    }
    let kind = match classify_link(text) {
        Ok(kind) => kind,
        Err(error) => {
            ui.set_error_text(error.into());
            return;
        }
    };
    match kind {
        LinkKind::Torrent => ui.invoke_download_torrent(text.into()),
        LinkKind::Video => ui.invoke_download_video(text.into()),
        LinkKind::File => {
            let dir = PathBuf::from(ui.get_save_dir_text().as_str());
            let connections = ui.get_connections().clamp(1, 32) as usize;
            match spec_from_url(text, &dir, connections) {
                Ok(spec) => {
                    let queue = queue.clone();
                    handle.spawn(async move {
                        let _ = queue.add(spec).await;
                    });
                }
                Err(error) => ui.set_error_text(error.into()),
            }
        }
    }
}

fn is_video_url(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim_start_matches("www.");
    const VIDEO_HOSTS: [&str; 9] = [
        "youtube.com",
        "youtu.be",
        "vimeo.com",
        "dailymotion.com",
        "twitch.tv",
        "tiktok.com",
        "instagram.com",
        "x.com",
        "twitter.com",
    ];
    VIDEO_HOSTS
        .iter()
        .any(|known| host == *known || host.ends_with(&format!(".{known}")))
}

fn spec_from_url(url: &str, dir: &Path, connections: usize) -> Result<DownloadSpec, String> {
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
    spec.connections = connections;
    Ok(spec)
}

fn spec_from_browser(
    url: &str,
    filename: Option<&str>,
    referrer: Option<&str>,
    dir: &Path,
    connections: usize,
) -> Result<DownloadSpec, String> {
    let mut spec = spec_from_url(url, dir, connections)?;
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
    fn classifies_links() {
        assert_eq!(
            classify_link("magnet:?xt=urn:btih:abc"),
            Ok(LinkKind::Torrent)
        );
        assert_eq!(
            classify_link("https://example.com/debian.torrent?mirror=1"),
            Ok(LinkKind::Torrent)
        );
        assert_eq!(
            classify_link("https://www.youtube.com/watch?v=x"),
            Ok(LinkKind::Video)
        );
        assert_eq!(classify_link("https://youtu.be/x"), Ok(LinkKind::Video));
        assert_eq!(
            classify_link("https://example.com/file.zip"),
            Ok(LinkKind::File)
        );
        assert!(classify_link("ftp://example.com/file").is_err());
        assert!(classify_link("just some text").is_err());
    }

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
            8,
        )
        .unwrap();
        assert_eq!(spec.url, "https://example.com/dir/file.zip?token=abc#frag");
        assert_eq!(spec.target, dir.path().join("file.zip"));
        assert_eq!(spec.connections, 8);

        assert!(spec_from_url("ftp://example.com/file.zip", dir.path(), 8).is_err());
    }

    #[test]
    fn spec_avoids_overwriting_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.zip"), b"x").unwrap();
        let spec = spec_from_url("https://example.com/file.zip", dir.path(), 8).unwrap();
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
            8,
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
