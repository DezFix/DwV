//! Video downloads through yt-dlp.
//!
//! A video is a job (`url`, folder, quality) plus at most one running task.
//! Pausing kills yt-dlp and keeps its `.part` files; resuming starts it again
//! with the same arguments and yt-dlp continues where it stopped. The job is
//! also stored in history, so it can be restored after a restart.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use dwv_core::{DownloadSpec, State as DbState};
use dwv_media::VideoQuality;
use tauri::{AppHandle, Emitter};

use crate::commands::{AppState, Reuse, VIDEO_ID_BASE, download_limit_bps};
use crate::model::{Category, DownloadInfo, DownloadStatus, category_of, now_secs};
use crate::names::file_name_of;
use crate::notify;

/// What is needed to (re)start a video download.
#[derive(Debug, Clone)]
pub struct VideoJob {
    pub url: String,
    pub dir: PathBuf,
    pub quality: VideoQuality,
}

/// Resolves the quality key from the request, then the user's default.
pub fn pick_quality(state: &AppState, requested: Option<&str>) -> VideoQuality {
    requested
        .and_then(VideoQuality::from_key)
        .or_else(|| {
            let settings = state.settings.lock().unwrap();
            VideoQuality::from_key(&settings.default_video_quality)
        })
        .unwrap_or(VideoQuality::Best)
}

#[tauri::command]
pub async fn add_video(
    url: String,
    dir: Option<String>,
    quality: Option<String>,
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<i64, String> {
    add_video_inner(&state, &app, &url, dir.as_deref(), quality.as_deref(), None).await
}

pub async fn add_video_inner(
    state: &AppState,
    app: &AppHandle,
    url: &str,
    dir: Option<&str>,
    quality: Option<&str>,
    reuse: Option<Reuse>,
) -> Result<i64, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("Пустая ссылка".to_string());
    }
    let quality = pick_quality(state, quality);

    let id = VIDEO_ID_BASE + state.torrent_seq.fetch_add(1, Ordering::Relaxed) as i64;
    let dir = dir
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| state.target_dir());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // A restored job keeps its history row and its place in the list.
    let start_paused = reuse.is_none() && !state.settings.lock().unwrap().auto_start_downloads;
    let (db_id, added_at, filename) = match &reuse {
        Some(reuse) => {
            if let Some(db) = &state.db {
                let _ = db.requeue(reuse.row);
                let _ = db.set_extra(reuse.row, Some(quality.key()));
            }
            (Some(reuse.row), reuse.added_at, reuse.filename.clone())
        }
        None => {
            let filename = format!("video-{id}.{}", quality.output_extension());
            let placeholder = dir.join(&filename);
            let db_id = state.db.as_ref().and_then(|db| {
                let mut spec = DownloadSpec::new(url.clone(), placeholder);
                spec.connections = 1;
                let row = db.insert_kind(&spec, "video", Some(quality.key())).ok()?;
                if start_paused {
                    let _ = db.set_state(row, DbState::Paused);
                }
                Some(row)
            });
            (db_id, now_secs(), filename)
        }
    };

    let item = DownloadInfo {
        id,
        url: url.clone(),
        category: if quality.is_audio() {
            Category::Audio
        } else {
            Category::Video
        },
        filename,
        total_size: 0,
        downloaded: 0,
        speed: 0,
        status: if start_paused {
            DownloadStatus::Paused
        } else {
            DownloadStatus::Pending
        },
        error: None,
        eta: 0,
        added_at,
        file_path: None,
        connections: 0,
        kind: "video".to_string(),
        db_id,
    };
    state
        .downloads
        .lock()
        .unwrap()
        .insert(id as u64, item.clone());
    state
        .video_jobs
        .lock()
        .unwrap()
        .insert(id, VideoJob { url, dir, quality });
    let _ = app.emit("download-added", item);

    if !start_paused {
        spawn_video_task(state, app, id);
    }
    Ok(id)
}

/// Stops yt-dlp and keeps the partial files.
pub fn pause_video(state: &AppState, app: &AppHandle, id: i64) {
    if let Some(task) = state.video_tasks.lock().unwrap().remove(&id) {
        task.abort();
    }
    let db_id = {
        let mut guard = state.downloads.lock().unwrap();
        let Some(entry) = guard.get_mut(&(id as u64)) else {
            return;
        };
        if !matches!(
            entry.status,
            DownloadStatus::Pending | DownloadStatus::Downloading
        ) {
            return;
        }
        entry.status = DownloadStatus::Paused;
        entry.speed = 0;
        entry.eta = 0;
        entry.db_id
    };
    if let (Some(db), Some(row)) = (&state.db, db_id) {
        let _ = db.set_state(row, DbState::Paused);
    }
    let _ = app.emit(
        "download-status",
        serde_json::json!({ "id": id, "status": "paused" }),
    );
}

/// Starts a paused or failed video again. `false` when the job is unknown.
pub fn resume_video(state: &AppState, app: &AppHandle, id: i64) -> bool {
    if !state.video_jobs.lock().unwrap().contains_key(&id) {
        return false;
    }
    if state.video_tasks.lock().unwrap().contains_key(&id) {
        return true;
    }
    let db_id = {
        let mut guard = state.downloads.lock().unwrap();
        let Some(entry) = guard.get_mut(&(id as u64)) else {
            return false;
        };
        entry.status = DownloadStatus::Pending;
        entry.error = None;
        entry.db_id
    };
    if let (Some(db), Some(row)) = (&state.db, db_id) {
        let _ = db.requeue(row);
    }
    let _ = app.emit(
        "download-status",
        serde_json::json!({ "id": id, "status": "pending" }),
    );
    spawn_video_task(state, app, id);
    true
}

enum VideoFinish {
    Done {
        path: Option<PathBuf>,
        size: u64,
        db_id: Option<i64>,
    },
    Failed {
        error: String,
        db_id: Option<i64>,
    },
}

fn spawn_video_task(state: &AppState, app: &AppHandle, id: i64) {
    let Some(job) = state.video_jobs.lock().unwrap().get(&id).cloned() else {
        return;
    };
    let options = {
        let settings = state.settings.lock().unwrap();
        dwv_media::VideoOptions {
            limit_bps: download_limit_bps(settings.speed_limit_enabled, settings.speed_limit_mbps)
                .map(u64::from),
            proxy: settings.proxy_url(),
        }
    };
    let st = state.clone();
    let emitter = app.clone();
    let tasks = state.video_tasks.clone();
    // Hold the map while spawning: a task that ends at once must find its own
    // entry when it removes itself.
    let mut registry = state.video_tasks.lock().unwrap();
    let task = state.handle.spawn(async move {
        let progress_state = st.clone();
        let progress_app = emitter.clone();
        let key = id as u64;
        let placeholder = job
            .dir
            .join(format!("video-{id}.{}", job.quality.output_extension()));

        let result = dwv_media::download_video_with_options(
            &job.url,
            &job.dir,
            job.quality,
            &options,
            move |progress| {
                let payload = {
                    let mut guard = progress_state.downloads.lock().unwrap();
                    let Some(entry) = guard.get_mut(&key) else {
                        return;
                    };
                    entry.downloaded = progress.downloaded;
                    entry.speed = progress.speed_bps.unwrap_or(0);
                    if let Some(total) = progress.total {
                        entry.total_size = total;
                    }
                    let status_changed = entry.status != DownloadStatus::Downloading;
                    entry.status = DownloadStatus::Downloading;
                    entry.eta = if entry.speed > 0 {
                        entry
                            .total_size
                            .saturating_sub(entry.downloaded)
                            .saturating_div(entry.speed)
                    } else {
                        0
                    };
                    (
                        serde_json::json!({
                            "id": id,
                            "downloaded": progress.downloaded,
                            "speed": entry.speed,
                            "eta": entry.eta,
                        }),
                        status_changed,
                        entry.total_size,
                    )
                };
                let (progress_payload, status_changed, total) = payload;
                let _ = progress_app.emit("download-progress", progress_payload);
                if status_changed {
                    let _ = progress_app.emit(
                        "download-status",
                        serde_json::json!({ "id": id, "status": "downloading", "total": total }),
                    );
                }
            },
        )
        .await;

        let finish = {
            let mut guard = st.downloads.lock().unwrap();
            let Some(entry) = guard.get_mut(&key) else {
                tasks.lock().unwrap().remove(&id);
                return;
            };
            match result {
                Ok(path) => {
                    let path = path.or_else(|| placeholder.is_file().then(|| placeholder.clone()));
                    let size = path
                        .as_ref()
                        .and_then(|path| std::fs::metadata(path).ok())
                        .map(|meta| meta.len())
                        .unwrap_or(entry.downloaded);
                    entry.status = DownloadStatus::Completed;
                    entry.speed = 0;
                    entry.eta = 0;
                    if let Some(path) = path.as_ref() {
                        let name = file_name_of(path);
                        entry.filename = name.clone();
                        entry.category = category_of(&name);
                        entry.file_path = Some(path.to_string_lossy().to_string());
                    }
                    entry.total_size = size.max(entry.total_size);
                    entry.downloaded = entry.total_size;
                    VideoFinish::Done {
                        path,
                        size: entry.total_size,
                        db_id: entry.db_id,
                    }
                }
                Err(error) => {
                    let error = error.to_string();
                    entry.status = DownloadStatus::Error;
                    entry.error = Some(error.clone());
                    entry.speed = 0;
                    VideoFinish::Failed {
                        error,
                        db_id: entry.db_id,
                    }
                }
            }
        };

        match finish {
            VideoFinish::Done { path, size, db_id } => {
                st.video_jobs.lock().unwrap().remove(&id);
                if let Some(path) = path {
                    let filename = file_name_of(&path);
                    let category = category_of(&filename);
                    if let (Some(db), Some(row)) = (&st.db, db_id) {
                        let _ = db.set_target(row, &path);
                        let _ = db.set_progress(row, size, Some(size));
                        let _ = db.finish(row, DbState::Completed);
                    }
                    let _ = emitter.emit(
                        "download-complete",
                        serde_json::json!({
                            "id": id,
                            "file_path": path.to_string_lossy(),
                            "total": size,
                            "filename": filename,
                            "category": category,
                        }),
                    );
                    notify::download_finished(&emitter, &filename, notify::Outcome::Done);
                }
            }
            VideoFinish::Failed { error, db_id } => {
                if let (Some(db), Some(row)) = (&st.db, db_id) {
                    let _ = db.set_error(row, &error);
                    let _ = db.finish(row, DbState::Failed);
                }
                let _ = emitter.emit(
                    "download-error",
                    serde_json::json!({ "id": id, "error": error }),
                );
                let name = video_display_name(&st, key);
                notify::download_finished(&emitter, &name, notify::Outcome::Failed(&error));
            }
        }
        tasks.lock().unwrap().remove(&id);
    });
    registry.insert(id, task);
}

fn video_display_name(state: &AppState, key: u64) -> String {
    state
        .downloads
        .lock()
        .unwrap()
        .get(&key)
        .map(|entry| entry.url.clone())
        .unwrap_or_default()
}

/// Folder a stored video row downloads into.
pub fn stored_dir(target: &Path) -> Option<PathBuf> {
    target.parent().map(Path::to_path_buf)
}
