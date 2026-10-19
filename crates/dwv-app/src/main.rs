slint::include_modules!();

use std::path::{Path, PathBuf};
use std::rc::Rc;

use dwv_core::{Database, DownloadSpec, Queue, QueueConfig, QueueEvent};
use slint::{ComponentHandle, Model, VecModel};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;
    let model = Rc::new(VecModel::<DownloadItem>::default());
    ui.set_downloads(model.clone().into());

    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let (queue, mut events) = Queue::start(QueueConfig::default(), open_db());

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
        move |url| match spec_from_url(url.as_str()) {
            Ok(spec) => {
                let queue = queue.clone();
                handle.spawn(async move {
                    let _ = queue.add(spec).await;
                });
            }
            Err(error) => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_error_text(error.into());
                }
            }
        }
    });

    ui.on_cancel_download({
        let queue = queue.clone();
        move |id| queue.cancel(id as u64)
    });

    ui.run()?;
    Ok(())
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
            };
            match index {
                Some(i) => model.set_row_data(i, item),
                None => model.push(item),
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
            if let Some(i) = index {
                let mut item = model.row_data(i).unwrap();
                item.progress = 1.0;
                item.detail = "Finished".into();
                model.set_row_data(i, item);
            }
        }
        QueueEvent::Failed { error, .. } => {
            if let Some(i) = index {
                let mut item = model.row_data(i).unwrap();
                item.detail = format!("Failed: {error}").into();
                model.set_row_data(i, item);
            }
        }
        QueueEvent::Cancelled { .. } => {
            if let Some(i) = index {
                let mut item = model.row_data(i).unwrap();
                item.detail = "Cancelled".into();
                model.set_row_data(i, item);
            }
        }
    }
}

fn spec_from_url(url: &str) -> Result<DownloadSpec, String> {
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
    let mut spec = DownloadSpec::new(url, unique_path(&download_dir().join(&name)));
    spec.connections = 8;
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

fn download_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join("Downloads"))
        .filter(|dir| dir.is_dir())
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
