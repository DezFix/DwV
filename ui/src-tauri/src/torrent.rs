//! Torrents: adding (with the file picker), restoring after a restart and
//! mirroring librqbit's events into the shared download list.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dwv_core::{DownloadSpec, State as DbState};
use dwv_torrent::{TorrentEvent, TorrentState};
use tauri::{AppHandle, Emitter, State, command};

use crate::commands::{AppState, PendingPayload, Reuse, TORRENT_ID_BASE, queue_pending};
use crate::model::{
    DownloadInfo, DownloadStatus, TorrentFileInfo, TorrentMetaInfo, category_of, now_secs,
    status_name,
};
use crate::names::{file_name_of, unique_download_name};
use crate::notify;

/// A `.torrent` body saved by the browser under any name/MIME: rename it and
/// hand it to the torrent picker instead of keeping it as a download.
pub fn torrent_from_download(state: &AppState, app: &AppHandle, id: u64, path: &Path) -> bool {
    if !looks_like_torrent(path) {
        return false;
    }
    let target = torrent_target_path(path);
    if target != path && std::fs::rename(path, &target).is_err() {
        return false;
    }

    let entry = state.downloads.lock().unwrap().remove(&id);
    if let Some(db) = &state.db {
        if let Some(row) = entry.as_ref().and_then(|entry| entry.db_id) {
            let _ = db.delete(row);
        } else {
            let _ = db.delete_by_target(path);
        }
    }
    let _ = app.emit("download-removed", serde_json::json!({ "id": id }));

    let source = target.to_string_lossy().to_string();
    let name = file_name_of(&target);
    let name = name
        .strip_suffix(".torrent")
        .or_else(|| name.strip_suffix(".TORRENT"))
        .unwrap_or(&name)
        .to_string();
    queue_pending(
        state,
        app,
        PendingPayload::Torrent(source.clone()),
        "torrent",
        &source,
        Some(name),
    );
    true
}

fn looks_like_torrent_bytes(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'd') && bytes.windows(6).any(|window| window == b"4:info")
}

pub fn looks_like_torrent(path: &Path) -> bool {
    const MAX: u64 = 16 << 20;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() > MAX {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    looks_like_torrent_bytes(&bytes)
}

/// Downloads a `.torrent` link with the browser's cookies, referer and user
/// agent: the torrent session would request it anonymously and get a login
/// page from private trackers instead.
pub async fn fetch_torrent_path(
    state: &AppState,
    url: &str,
    referrer: Option<&str>,
    cookies: Option<&str>,
    user_agent: Option<&str>,
) -> Option<PathBuf> {
    const MAX: usize = 16 << 20;
    let mut builder = reqwest::Client::builder()
        .http1_only()
        .timeout(Duration::from_secs(30));
    if let Some(proxy) = state.settings.lock().unwrap().proxy_url()
        && let Ok(proxy) = reqwest::Proxy::all(proxy)
    {
        builder = builder.proxy(proxy);
    }
    let client = builder.build().ok()?;
    let mut request = client.get(url);
    if let Some(value) = referrer {
        request = request.header(reqwest::header::REFERER, value);
    }
    if let Some(value) = cookies {
        request = request.header(reqwest::header::COOKIE, value);
    }
    if let Some(value) = user_agent {
        request = request.header(reqwest::header::USER_AGENT, value);
    }
    let response = request.send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX as u64)
    {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > MAX || !looks_like_torrent_bytes(&bytes) {
        return None;
    }
    let dir = state.data_dir.join("cache").join("torrents");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!(
        "{}-{}.torrent",
        now_secs(),
        dwv_torrent::torrent_name(url)
    ));
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

pub fn is_cached_torrent(state: &AppState, source: &str) -> bool {
    Path::new(source).starts_with(state.data_dir.join("cache").join("torrents"))
}

pub fn torrent_target_path(path: &Path) -> PathBuf {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
    {
        return path.to_path_buf();
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "download".to_string());
    let candidate = unique_download_name(dir, &format!("{stem}.torrent"));
    dir.join(candidate)
}

#[command]
pub async fn add_torrent(
    source: String,
    dir: Option<String>,
    only_files: Option<Vec<usize>>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<i64, String> {
    add_torrent_inner(&state, &app, &source, dir.as_deref(), only_files).await
}

#[command]
pub async fn inspect_torrent(
    source: String,
    dir: Option<String>,
    state: State<'_, AppState>,
) -> Result<TorrentMetaInfo, String> {
    let source = source.trim().to_string();
    if source.is_empty() {
        return Err("Пустая ссылка".to_string());
    }
    let dir = dir
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| state.target_dir());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // Magnets wait for metadata from peers; two minutes is the practical limit.
    let inspected = tokio::time::timeout(
        Duration::from_secs(120),
        state.torrents.inspect(&source, &dir),
    )
    .await
    .map_err(|_| "metadata-timeout".to_string())?
    .map_err(|error| error.to_string())?;

    Ok(TorrentMetaInfo {
        info_hash: inspected.info_hash,
        name: inspected.name,
        total_size: inspected.total_size,
        files: inspected
            .files
            .into_iter()
            .map(|file| TorrentFileInfo {
                index: file.index,
                path: file.path,
                size: file.size,
                downloaded: 0,
            })
            .collect(),
    })
}

pub async fn add_torrent_inner(
    state: &AppState,
    app: &AppHandle,
    source: &str,
    dir: Option<&str>,
    only_files: Option<Vec<usize>>,
) -> Result<i64, String> {
    add_torrent_with(state, app, source, dir, only_files, None).await
}

/// Adds a torrent, or restores one from history when `reuse` is set: then
/// the existing folder is expected and librqbit checks the pieces already on
/// disk instead of starting over.
pub async fn add_torrent_with(
    state: &AppState,
    app: &AppHandle,
    source: &str,
    dir: Option<&str>,
    only_files: Option<Vec<usize>>,
    reuse: Option<Reuse>,
) -> Result<i64, String> {
    let source = source.trim().to_string();
    if source.is_empty() {
        return Err("Пустая ссылка".to_string());
    }

    let settings = state.settings.lock().unwrap().clone();
    let dir = dir
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| state.target_dir());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    if only_files.as_ref().is_some_and(|files| files.is_empty()) {
        return Err("Не выбран ни один файл".to_string());
    }

    let id = TORRENT_ID_BASE + state.torrent_seq.fetch_add(1, Ordering::Relaxed) as i64;
    let name = match &reuse {
        Some(reuse) => reuse.filename.clone(),
        None => dwv_torrent::torrent_name(&source),
    };
    let target = dir.join(&name);
    if reuse.is_none() && target.exists() {
        return Err(format!("Файл или папка с именем «{}» уже существует", name));
    }
    // A restored torrent continues; autostart only decides for new ones.
    let paused = reuse.is_none() && !settings.torrent_autostart;
    // Keep our own copy of a .torrent file: the original (or the browser
    // cache copy) may be gone when the download is restored later.
    let source = keep_torrent_file(state, &source);
    let extra = only_files.as_ref().map(|files| {
        files
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",")
    });

    let (db_id, added_at) = match &reuse {
        Some(reuse) => {
            if let Some(db) = &state.db {
                let _ = db.requeue(reuse.row);
                let _ = db.set_url(reuse.row, &source);
            }
            (Some(reuse.row), reuse.added_at)
        }
        None => {
            let db_id = state.db.as_ref().and_then(|db| {
                let mut spec = DownloadSpec::new(source.clone(), target.clone());
                spec.connections = 1;
                let row = db.insert_kind(&spec, "torrent", extra.as_deref()).ok()?;
                if paused {
                    let _ = db.set_state(row, DbState::Paused);
                }
                Some(row)
            });
            (db_id, now_secs())
        }
    };

    let item = DownloadInfo {
        id,
        url: source.clone(),
        filename: name.clone(),
        total_size: 0,
        downloaded: 0,
        speed: 0,
        status: if paused {
            DownloadStatus::Paused
        } else {
            DownloadStatus::Pending
        },
        error: None,
        eta: 0,
        added_at,
        file_path: Some(target.to_string_lossy().to_string()),
        category: category_of(&name),
        connections: 0,
        kind: "torrent".to_string(),
        db_id,
    };
    state
        .downloads
        .lock()
        .unwrap()
        .insert(id as u64, item.clone());
    let _ = app.emit("download-added", item);

    let event_state = state.clone();
    let event_app = app.clone();
    let sink: Arc<dyn Fn(TorrentEvent) + Send + Sync> = Arc::new(move |event| {
        apply_torrent_event(&event_state, &event_app, event);
    });

    match state
        .torrents
        .add(
            id,
            &source,
            &dir,
            paused,
            settings.torrent_seed_after_complete,
            name,
            only_files,
            sink,
        )
        .await
    {
        Ok(()) => Ok(id),
        Err(error) => {
            let message = error.to_string();
            if let Some(entry) = state.downloads.lock().unwrap().get_mut(&(id as u64)) {
                entry.status = DownloadStatus::Error;
                entry.error = Some(message.clone());
                entry.speed = 0;
            }
            if let (Some(db), Some(row)) = (&state.db, db_id) {
                let _ = db.set_error(row, &message);
                let _ = db.finish(row, DbState::Failed);
            }
            let _ = app.emit(
                "download-error",
                serde_json::json!({ "id": id, "error": message }),
            );
            Err(message)
        }
    }
}

/// Folder that holds DwV's copies of `.torrent` files.
pub fn torrent_store(state: &AppState) -> PathBuf {
    state.data_dir.join("torrents")
}

/// Copies a local `.torrent` into app data and returns the copy's path;
/// magnets and URLs pass through unchanged.
fn keep_torrent_file(state: &AppState, source: &str) -> String {
    let path = Path::new(source);
    let store = torrent_store(state);
    if !path.is_file() || path.starts_with(&store) {
        return source.to_string();
    }
    if std::fs::create_dir_all(&store).is_err() {
        return source.to_string();
    }
    let name = format!(
        "{}-{}.torrent",
        now_secs(),
        dwv_torrent::torrent_name(source)
    );
    let copy = store.join(unique_download_name(&store, &name));
    match std::fs::copy(path, &copy) {
        Ok(_) => copy.to_string_lossy().to_string(),
        Err(_) => source.to_string(),
    }
}

/// Deletes DwV's copy of a `.torrent` when its download leaves the list.
pub fn forget_torrent_file(state: &AppState, source: &str) {
    let path = Path::new(source);
    if path.starts_with(torrent_store(state)) && path.is_file() {
        let _ = std::fs::remove_file(path);
    }
}

/// Re-adds a torrent the session reported as failed, keeping its row.
pub async fn retry_torrent(state: &AppState, app: &AppHandle, id: i64) -> Result<i64, String> {
    let entry = state
        .downloads
        .lock()
        .unwrap()
        .get(&(id as u64))
        .cloned()
        .ok_or_else(|| "Загрузка не найдена".to_string())?;
    let _ = state.torrents.remove(id, false).await;
    let target = entry
        .file_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "Не найдена папка торрента".to_string())?;
    let dir = target
        .parent()
        .map(|dir| dir.to_string_lossy().to_string())
        .ok_or_else(|| "Не найдена папка торрента".to_string())?;
    let only_files = entry
        .db_id
        .and_then(|row| state.db.as_ref()?.get(row).ok().flatten())
        .and_then(|stored| parse_only_files(stored.extra.as_deref()));
    state.downloads.lock().unwrap().remove(&(id as u64));
    let _ = app.emit("download-removed", serde_json::json!({ "id": id }));
    let reuse = entry.db_id.map(|row| Reuse {
        row,
        added_at: entry.added_at,
        filename: file_name_of(&target),
    });
    add_torrent_with(state, app, &entry.url, Some(&dir), only_files, reuse).await
}

/// Reads the selected file indices stored with a torrent row.
pub fn parse_only_files(extra: Option<&str>) -> Option<Vec<usize>> {
    let files: Vec<usize> = extra?
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    (!files.is_empty()).then_some(files)
}

pub fn apply_torrent_event(state: &AppState, app: &AppHandle, event: TorrentEvent) {
    match event {
        TorrentEvent::Progress {
            id,
            downloaded,
            total,
            speed,
            uploaded: _,
            connections,
            eta,
        } => {
            let payload = {
                let mut guard = state.downloads.lock().unwrap();
                guard.get_mut(&(id as u64)).map(|entry| {
                    entry.downloaded = downloaded;
                    entry.speed = speed;
                    entry.connections = connections;
                    entry.eta = eta;
                    if total > 0 {
                        entry.total_size = total;
                    }
                    serde_json::json!({
                        "id": id,
                        "downloaded": downloaded,
                        "speed": speed,
                        "eta": eta,
                    })
                })
            };
            if let Some(payload) = payload {
                let _ = app.emit("download-progress", payload);
            }
        }
        TorrentEvent::Status {
            id,
            state: torrent_state,
            total,
            error,
        } => {
            let status = torrent_status(torrent_state);
            let payload = {
                let mut guard = state.downloads.lock().unwrap();
                guard.get_mut(&(id as u64)).map(|entry| {
                    entry.status = status.clone();
                    if let Some(error) = &error {
                        entry.error = Some(error.clone());
                    }
                    if total > 0 {
                        entry.total_size = total;
                    }
                    if status == DownloadStatus::Paused {
                        entry.speed = 0;
                    }
                    let mut payload =
                        serde_json::json!({ "id": id, "status": status_name(&status) });
                    if entry.total_size > 0 {
                        payload["total"] = serde_json::json!(entry.total_size);
                    }
                    if let Some(error) = &error {
                        payload["error"] = serde_json::json!(error);
                    }
                    (payload, entry.db_id, status)
                })
            };
            if let Some((payload, db_id, status)) = payload {
                if let (Some(db), Some(row)) = (&state.db, db_id) {
                    match status {
                        DownloadStatus::Downloading => {
                            let _ = db.set_state(row, DbState::Downloading);
                        }
                        DownloadStatus::Paused => {
                            let _ = db.set_state(row, DbState::Paused);
                        }
                        _ => {}
                    }
                }
                let _ = app.emit("download-status", payload);
            }
        }
        TorrentEvent::Complete { id, path, total } => {
            let payload = {
                let mut guard = state.downloads.lock().unwrap();
                guard.get_mut(&(id as u64)).map(|entry| {
                    let name = file_name_of(&path);
                    entry.status = DownloadStatus::Completed;
                    entry.file_path = Some(path.to_string_lossy().to_string());
                    entry.filename = name.clone();
                    entry.category = category_of(&name);
                    if total > 0 {
                        entry.total_size = total;
                    }
                    entry.downloaded = entry.total_size.max(entry.downloaded);
                    entry.speed = 0;
                    entry.eta = 0;
                    (
                        serde_json::json!({
                            "id": id,
                            "file_path": path.to_string_lossy(),
                            "total": entry.total_size,
                            "filename": entry.filename,
                            "category": entry.category,
                        }),
                        entry.db_id,
                        entry.downloaded,
                    )
                })
            };
            if let Some((payload, db_id, downloaded)) = payload {
                if let (Some(db), Some(row)) = (&state.db, db_id) {
                    let _ = db.set_target(row, &path);
                    let _ = db.set_progress(row, downloaded, Some(downloaded));
                    let _ = db.finish(row, DbState::Completed);
                }
                let _ = app.emit("download-complete", payload);
                notify::download_finished(app, &file_name_of(&path), notify::Outcome::Done);
            }
        }
        TorrentEvent::Error { id, error } => {
            let found = {
                let mut guard = state.downloads.lock().unwrap();
                guard.get_mut(&(id as u64)).map(|entry| {
                    entry.status = DownloadStatus::Error;
                    entry.error = Some(error.clone());
                    entry.speed = 0;
                    (entry.db_id, entry.filename.clone())
                })
            };
            let Some((db_id, filename)) = found else {
                return;
            };
            if let (Some(db), Some(row)) = (&state.db, db_id) {
                let _ = db.set_error(row, &error);
                let _ = db.finish(row, DbState::Failed);
            }
            let _ = app.emit(
                "download-error",
                serde_json::json!({ "id": id, "error": error }),
            );
            notify::download_finished(app, &filename, notify::Outcome::Failed(&error));
        }
    }
}

fn torrent_status(state: TorrentState) -> DownloadStatus {
    match state {
        TorrentState::Pending => DownloadStatus::Pending,
        TorrentState::Downloading => DownloadStatus::Downloading,
        TorrentState::Paused => DownloadStatus::Paused,
        TorrentState::Completed => DownloadStatus::Completed,
        TorrentState::Error => DownloadStatus::Error,
    }
}
