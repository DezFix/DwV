//! Tauri commands and the state they share. Kind-specific work lives in
//! `video`, `torrent` and `media`; this module routes by download id.

use crate::settings::Settings;
use dwv_core::{Database, DownloadSpec, Queue, QueueConfig, QueueEvent, State as DbState};
use dwv_torrent::{TorrentManager, TorrentSessionConfig};
use serde::Serialize;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, command};

pub use crate::media::media_for;
pub use crate::model::*;
pub use crate::names::{
    deep_link_type, filename_from_disposition, from_url, parse_deep_link, resolve_filename,
    unique_download_name,
};
pub use crate::platform::{apply_autostart, register_protocol};
pub use crate::torrent::{
    add_torrent_inner, apply_torrent_event, fetch_torrent_path, inspect_torrent,
    looks_like_torrent, torrent_target_path,
};
pub use crate::video::add_video_inner;

use crate::names::{file_name_of, sanitize_file_name, suggest_name};
use crate::notify;
use crate::torrent::{
    add_torrent_with, forget_torrent_file, is_cached_torrent, parse_only_files, retry_torrent,
    torrent_from_download,
};
use crate::video::{VideoJob, pause_video, resume_video};

pub const TORRENT_ID_BASE: i64 = 1_000_000_000;
pub const VIDEO_ID_BASE: i64 = 2_000_000_000;

/// A history row a restored download keeps (its id, date and name).
#[derive(Debug, Clone)]
pub struct Reuse {
    pub row: i64,
    pub added_at: u64,
    pub filename: String,
}
pub struct AppState {
    pub queue: Queue,
    pub db: Option<Database>,
    pub downloads: Arc<Mutex<HashMap<u64, DownloadInfo>>>,
    pub settings: Arc<Mutex<Settings>>,
    pub settings_file: PathBuf,
    pub data_dir: PathBuf,
    pub handle: tokio::runtime::Handle,
    pub torrent_seq: Arc<AtomicU64>,
    pub torrents: TorrentManager,
    pub video_tasks: Arc<Mutex<HashMap<i64, tokio::task::JoinHandle<()>>>>,
    pub video_jobs: Arc<Mutex<HashMap<i64, VideoJob>>>,
    pub media_cache: Arc<Mutex<HashMap<String, MediaInfo>>>,
    pub pending: Arc<Mutex<HashMap<u64, PendingAdd>>>,
    pub pending_seq: Arc<AtomicU64>,
}

/// The core queue runs as tokio tasks, so the app owns a tokio runtime for its
/// whole lifetime. Keeping it global avoids dropping it from async contexts.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            db: self.db.clone(),
            downloads: self.downloads.clone(),
            settings: self.settings.clone(),
            settings_file: self.settings_file.clone(),
            data_dir: self.data_dir.clone(),
            handle: self.handle.clone(),
            torrent_seq: self.torrent_seq.clone(),
            torrents: self.torrents.clone(),
            video_tasks: self.video_tasks.clone(),
            video_jobs: self.video_jobs.clone(),
            media_cache: self.media_cache.clone(),
            pending: self.pending.clone(),
            pending_seq: self.pending_seq.clone(),
        }
    }
}

impl AppState {
    pub fn new(
        settings_file: PathBuf,
        data_dir: PathBuf,
    ) -> (Self, tokio::sync::mpsc::Receiver<QueueEvent>) {
        let settings = Settings::load(&settings_file);
        let db = Database::open(&data_dir.join("dwv.db")).ok();

        let runtime = runtime();
        let handle = runtime.handle().clone();

        let downloads_dir = if settings.download_dir.is_empty() {
            data_dir.join("Downloads")
        } else {
            PathBuf::from(&settings.download_dir)
        };
        let torrents = TorrentManager::new(downloads_dir, torrent_config(&settings));

        let (queue, events) = {
            let _guard = runtime.enter();
            Queue::start(
                QueueConfig {
                    max_parallel: settings.max_concurrent_downloads as usize,
                    speed_limit: None,
                },
                db.clone(),
            )
        };

        (
            Self {
                queue,
                db,
                downloads: Arc::new(Mutex::new(HashMap::new())),
                settings: Arc::new(Mutex::new(settings)),
                settings_file,
                data_dir,
                handle,
                torrent_seq: Arc::new(AtomicU64::new(0)),
                torrents,
                video_tasks: Arc::new(Mutex::new(HashMap::new())),
                video_jobs: Arc::new(Mutex::new(HashMap::new())),
                media_cache: Arc::new(Mutex::new(HashMap::new())),
                pending: Arc::new(Mutex::new(HashMap::new())),
                pending_seq: Arc::new(AtomicU64::new(0)),
            },
            events,
        )
    }

    pub fn target_dir(&self) -> PathBuf {
        let settings = self.settings.lock().unwrap();
        if settings.download_dir.is_empty() {
            self.data_dir.join("Downloads")
        } else {
            PathBuf::from(&settings.download_dir)
        }
    }
}

fn torrent_config(settings: &Settings) -> TorrentSessionConfig {
    TorrentSessionConfig {
        dht_enabled: settings.torrent_dht_enabled,
        peer_limit: (settings.torrent_max_peers > 0).then_some(settings.torrent_max_peers as usize),
        download_bps: download_limit_bps(settings.speed_limit_enabled, settings.speed_limit_mbps)
            .and_then(NonZeroU32::new),
        upload_bps: upload_limit_bps(settings.torrent_upload_limit_kbps).and_then(NonZeroU32::new),
        socks_proxy: settings.socks_proxy_url(),
    }
}

pub fn download_limit_bps(enabled: bool, mbps: f64) -> Option<u32> {
    if !enabled {
        return None;
    }
    let value = (mbps.max(0.5) * 1_000_000.0).round();
    if !value.is_finite() {
        return None;
    }
    Some(value.clamp(1.0, u32::MAX as f64) as u32)
}

pub fn upload_limit_bps(kbps: u32) -> Option<u32> {
    (kbps > 0).then(|| kbps.saturating_mul(1000))
}

/// Applies a queue event to the state the UI reads from.
pub fn apply_event(
    downloads: &mut HashMap<u64, DownloadInfo>,
    event: &QueueEvent,
) -> serde_json::Value {
    match event {
        QueueEvent::Queued { id, path } => {
            downloads
                .entry(*id)
                .or_insert_with(|| DownloadInfo::new(*id, path.clone()));
            serde_json::json!({ "id": id, "status": "pending" })
        }
        QueueEvent::Started {
            id,
            path,
            total,
            connections,
            ..
        } => {
            let entry = downloads
                .entry(*id)
                .or_insert_with(|| DownloadInfo::new(*id, path.clone()));
            entry.status = DownloadStatus::Downloading;
            entry.total_size = total.unwrap_or(0);
            entry.connections = *connections;
            entry.file_path = Some(path.to_string_lossy().to_string());
            entry.filename = file_name_of(path);
            entry.category = category_of(&entry.filename);
            serde_json::json!({ "id": id, "status": "downloading", "total": total })
        }
        QueueEvent::Progress {
            id,
            downloaded,
            total,
            speed_bps,
        } => {
            if let Some(entry) = downloads.get_mut(id) {
                entry.downloaded = *downloaded;
                entry.speed = *speed_bps;
                if let Some(total) = total {
                    entry.total_size = *total;
                }
                entry.eta = match total {
                    Some(total) if *speed_bps > 0 => {
                        (total.saturating_sub(*downloaded)) / *speed_bps
                    }
                    _ => 0,
                };
            }
            serde_json::json!({
                "id": id,
                "downloaded": downloaded,
                "speed": speed_bps,
                "eta": match total {
                    Some(total) if *speed_bps > 0 => (total.saturating_sub(*downloaded)) / *speed_bps,
                    _ => 0,
                }
            })
        }
        QueueEvent::Finished { id, path } => {
            let total = downloads.get(id).map(|entry| entry.total_size).unwrap_or(0);
            if let Some(entry) = downloads.get_mut(id) {
                entry.status = DownloadStatus::Completed;
                entry.downloaded = entry.total_size;
                entry.speed = 0;
                entry.eta = 0;
                entry.file_path = Some(path.to_string_lossy().to_string());
            }
            let mut payload =
                serde_json::json!({ "id": id, "status": "completed", "file_path": path });
            if total > 0 {
                payload["total"] = serde_json::json!(total);
            }
            payload
        }
        QueueEvent::Failed { id, error } => {
            if let Some(entry) = downloads.get_mut(id) {
                entry.status = DownloadStatus::Error;
                entry.error = Some(error.clone());
                entry.speed = 0;
            }
            serde_json::json!({ "id": id, "status": "error", "error": error })
        }
        QueueEvent::Cancelled { id } => {
            if let Some(entry) = downloads.get_mut(id) {
                entry.status = DownloadStatus::Cancelled;
                entry.speed = 0;
            }
            serde_json::json!({ "id": id, "status": "cancelled" })
        }
        QueueEvent::Paused { id } => {
            if let Some(entry) = downloads.get_mut(id) {
                entry.status = DownloadStatus::Paused;
                entry.speed = 0;
            }
            serde_json::json!({ "id": id, "status": "paused" })
        }
    }
}

/// Bridges the core queue into Tauri events so the UI can follow real progress.
pub fn spawn_event_bridge(
    app: AppHandle,
    state: AppState,
    mut events: tokio::sync::mpsc::Receiver<QueueEvent>,
    handle: tokio::runtime::Handle,
) {
    handle.spawn(async move {
        while let Some(event) = events.recv().await {
            if let QueueEvent::Finished { id, path } = &event
                && torrent_from_download(&state, &app, *id, path)
            {
                // The "file" was a .torrent; the picker owns it now.
                continue;
            }

            let (payload, filename) = {
                let mut guard = state.downloads.lock().unwrap();
                let payload = apply_event(&mut guard, &event);
                let id = match &event {
                    QueueEvent::Finished { id, .. } | QueueEvent::Failed { id, .. } => Some(*id),
                    _ => None,
                };
                let filename = id
                    .and_then(|id| guard.get(&id))
                    .map(|entry| entry.filename.clone());
                (payload, filename)
            };

            let name = match &event {
                QueueEvent::Progress { .. } => "download-progress",
                QueueEvent::Finished { .. } => "download-complete",
                QueueEvent::Failed { .. } => "download-error",
                _ => "download-status",
            };
            let _ = app.emit(name, payload);
            match (&event, filename) {
                (QueueEvent::Finished { .. }, Some(filename)) => {
                    notify::download_finished(&app, &filename, notify::Outcome::Done);
                }
                (QueueEvent::Failed { error, .. }, Some(filename)) => {
                    notify::download_finished(&app, &filename, notify::Outcome::Failed(error));
                }
                _ => {}
            }
        }
    });
}

pub(crate) fn emit_added(state: &AppState, app: &AppHandle, id: i64) {
    emit_added_with_requested(state, app, id, None);
}

pub(crate) fn emit_added_with_requested(
    state: &AppState,
    app: &AppHandle,
    id: i64,
    requested_name: Option<&str>,
) {
    let item = state.downloads.lock().unwrap().get(&(id as u64)).cloned();
    if let Some(item) = item {
        if requested_name.is_some_and(|name| name != item.filename) {
            let _ = app.emit(
                "download-renamed",
                serde_json::json!({ "id": id, "filename": item.filename }),
            );
        }
        let _ = app.emit("download-added", item);
    }
}

#[command]
pub fn default_download_dir(state: State<'_, AppState>) -> String {
    state.target_dir().to_string_lossy().to_string()
}

#[command]
pub async fn add_download(
    url: String,
    dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<u64, String> {
    let context = DownloadContext {
        url,
        dir: dir
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty()),
        ..Default::default()
    };
    let id = add_download_with_context(&state, context).await?;
    emit_added(&state, &app, id as i64);
    Ok(id)
}

#[derive(Default, Clone)]
pub struct DownloadContext {
    pub url: String,
    pub final_url: Option<String>,
    pub filename: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    pub cookies: Option<String>,
    pub headers: HashMap<String, String>,
    pub dir: Option<PathBuf>,
}

pub async fn add_download_inner(state: &AppState, url: &str) -> Result<u64, String> {
    add_download_with_context(
        state,
        DownloadContext {
            url: url.to_string(),
            ..Default::default()
        },
    )
    .await
}

pub async fn add_download_with_context(
    state: &AppState,
    context: DownloadContext,
) -> Result<u64, String> {
    let original_url = context.url.trim().to_string();
    if original_url.is_empty() {
        return Err("Пустая ссылка".to_string());
    }
    let fetch_url = context
        .final_url
        .as_deref()
        .map(str::trim)
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .unwrap_or(&original_url)
        .to_string();

    let mut headers = Vec::new();
    for (name, value) in &context.headers {
        set_header(&mut headers, name, value);
    }
    if let Some(referrer) = context.referrer.as_deref() {
        set_header(&mut headers, "Referer", referrer);
    }
    if let Some(user_agent) = context.user_agent.as_deref() {
        set_header(&mut headers, "User-Agent", user_agent);
    }
    if let Some(cookies) = context.cookies.as_deref() {
        set_header(&mut headers, "Cookie", cookies);
    }
    let (connections, proxy, custom_agent, auto_start) = {
        let settings = state.settings.lock().unwrap();
        (
            settings.connections_per_download.clamp(1, 32) as usize,
            settings.proxy_url(),
            settings.user_agent().map(str::to_string),
            settings.auto_start_downloads,
        )
    };
    // The browser's own User-Agent wins; the custom one replaces "DwV/x.y".
    if context.user_agent.is_none()
        && let Some(agent) = custom_agent.as_deref()
    {
        set_header(&mut headers, "User-Agent", agent);
    }
    if headers
        .iter()
        .any(|(name, value)| has_crlf(name) || has_crlf(value))
    {
        return Err("Заголовки содержат перевод строки".to_string());
    }

    let filename = match context
        .filename
        .as_deref()
        .map(sanitize_file_name)
        .filter(|name| !name.trim().is_empty() && name != "download")
    {
        Some(name) => name,
        None => crate::names::resolve_filename_with(&fetch_url, &headers, proxy.as_deref()).await,
    };
    let category = category_of(&filename);
    let dir = context.dir.clone().unwrap_or_else(|| state.target_dir());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let filename = unique_download_name(&dir, &filename);
    let target = dir.join(&filename);
    let mut spec = DownloadSpec::new(fetch_url, target.clone());
    spec.connections = connections;
    spec.headers = headers;
    spec.proxy = proxy;

    let id = if auto_start {
        state.queue.add(spec).await
    } else {
        state.queue.add_paused(spec).await
    }
    .map_err(|e| e.to_string())?;
    let db_id = state
        .db
        .as_ref()
        .and_then(|db| db.latest_id_by_target(&target).ok().flatten());

    insert_live_file(
        state,
        DownloadInfo {
            id: id as i64,
            url: original_url,
            filename,
            total_size: 0,
            downloaded: 0,
            speed: 0,
            status: if auto_start {
                DownloadStatus::Pending
            } else {
                DownloadStatus::Paused
            },
            error: None,
            eta: 0,
            added_at: now_secs(),
            // Known upfront, so the UI can match this row against history.
            file_path: Some(target.to_string_lossy().to_string()),
            category,
            connections: 0,
            kind: "file".to_string(),
            db_id,
        },
    );

    Ok(id)
}

/// Stores the full row for a queued file. The event bridge may have created
/// a bare entry already (the queue reports `Queued`/`Started` at once); keep
/// the progress it recorded instead of resetting it to "pending".
fn insert_live_file(state: &AppState, item: DownloadInfo) {
    let mut guard = state.downloads.lock().unwrap();
    match guard.get_mut(&(item.id as u64)) {
        Some(existing) => {
            let status = existing.status.clone();
            let (total, downloaded, connections) = (
                existing.total_size,
                existing.downloaded,
                existing.connections,
            );
            *existing = item;
            if status != DownloadStatus::Pending {
                existing.status = status;
            }
            existing.total_size = existing.total_size.max(total);
            existing.downloaded = existing.downloaded.max(downloaded);
            existing.connections = existing.connections.max(connections);
        }
        None => {
            guard.insert(item.id as u64, item);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadPreview {
    pub filename: String,
    pub suggested: String,
    pub exists: bool,
    pub path: String,
}

#[command]
pub async fn preview_download(
    url: String,
    dir: Option<String>,
    state: State<'_, AppState>,
) -> Result<DownloadPreview, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("Пустая ссылка".to_string());
    }
    let filename = resolve_filename(&url).await;
    let dir = dir
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| state.target_dir());
    let exists = dir.join(&filename).exists();
    let suggested = unique_download_name(&dir, &filename);
    Ok(DownloadPreview {
        filename,
        suggested: suggested.clone(),
        exists,
        path: dir.join(suggested).to_string_lossy().to_string(),
    })
}

#[command]
pub async fn add_download_as(
    url: String,
    filename: String,
    dir: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<u64, String> {
    let id = add_download_with_context(
        &state,
        DownloadContext {
            url,
            filename: Some(filename),
            dir: dir
                .map(PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty()),
            ..Default::default()
        },
    )
    .await?;
    emit_added(&state, &app, id as i64);
    Ok(id)
}

fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if let Some(entry) = headers
        .iter_mut()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
    {
        *entry = (name.to_string(), value.to_string());
    } else {
        headers.push((name.to_string(), value.to_string()));
    }
}

fn has_crlf(value: &str) -> bool {
    value.contains('\r') || value.contains('\n')
}

#[command]
pub async fn get_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadInfo>, String> {
    let live: Vec<DownloadInfo> = state.downloads.lock().unwrap().values().cloned().collect();
    let mut items = live.clone();

    if let Some(db) = &state.db {
        for stored in db.recent(1000).unwrap_or_default() {
            let path = stored.target.to_string_lossy().to_string();
            // The queue and the database number downloads independently, so a
            // finished row can shadow a live one. Match on the file instead.
            if live.iter().any(|item| {
                item.db_id == Some(stored.id) || item.file_path.as_deref() == Some(path.as_str())
            }) {
                continue;
            }
            items.push(DownloadInfo::from_stored(&stored));
        }
    }

    items.sort_by_key(|item| std::cmp::Reverse(item.added_at));
    Ok(items)
}

#[command]
pub async fn get_download_details(
    id: i64,
    state: State<'_, AppState>,
) -> Result<DownloadDetails, String> {
    build_details(&state, id).await
}

async fn build_details(state: &AppState, id: i64) -> Result<DownloadDetails, String> {
    let item = if id < 0 {
        let stored = state
            .db
            .as_ref()
            .and_then(|db| db.get(-id).ok().flatten())
            .ok_or_else(|| "Загрузка не найдена".to_string())?;
        DownloadInfo::from_stored(&stored)
    } else if let Some(item) = state.downloads.lock().unwrap().get(&(id as u64)).cloned() {
        item
    } else {
        let stored = state
            .db
            .as_ref()
            .and_then(|db| db.get(id).ok().flatten())
            .ok_or_else(|| "Загрузка не найдена".to_string())?;
        DownloadInfo::from_stored(&stored)
    };

    let kind = item.kind.clone();
    let mut downloaded = item.downloaded;
    let mut uploaded = 0;
    let mut connections = item.connections;
    let mut peers = Vec::new();
    let mut torrent = None;

    if kind == "torrent"
        && let Some(details) = state.torrents.details(id)
    {
        downloaded = details.downloaded;
        uploaded = details.uploaded;
        connections = details.peers_live as usize;
        peers = details
            .peers
            .into_iter()
            .map(|peer| PeerInfo {
                address: peer.address,
                state: peer.state,
                client: peer.client,
                downloaded: peer.downloaded,
                uploaded: peer.uploaded,
            })
            .collect();
        torrent = Some(TorrentInfo {
            info_hash: Some(details.info_hash),
            downloaded: details.downloaded,
            uploaded: details.uploaded,
            ratio: if details.downloaded > 0 {
                details.uploaded as f64 / details.downloaded as f64
            } else {
                0.0
            },
            peers_live: details.peers_live,
            peers_seen: details.peers_seen,
            files: state
                .torrents
                .files(id)
                .into_iter()
                .map(|file| TorrentFileInfo {
                    index: file.index,
                    path: file.path,
                    size: file.size,
                    downloaded: file.downloaded,
                })
                .collect(),
        });
    }

    let media = if item.status == DownloadStatus::Completed {
        media_for(state, &item).await
    } else {
        None
    };

    Ok(DownloadDetails {
        id,
        kind,
        url: item.url,
        filename: item.filename,
        file_path: item.file_path,
        total_size: item.total_size,
        downloaded,
        uploaded,
        speed: item.speed,
        status: item.status,
        error: item.error,
        category: item.category,
        added_at: item.added_at,
        connections,
        media,
        torrent,
        peers,
    })
}

pub fn is_torrent_id(id: i64) -> bool {
    (TORRENT_ID_BASE..VIDEO_ID_BASE).contains(&id)
}

pub fn is_video_id(id: i64) -> bool {
    id >= VIDEO_ID_BASE
}

#[command]
pub async fn pause_download(
    id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    pause_one(&state, &app, id).await
}

async fn pause_one(state: &AppState, app: &AppHandle, id: i64) -> Result<(), String> {
    if id < 0 {
        return Ok(());
    }
    if is_video_id(id) {
        pause_video(state, app, id);
    } else if is_torrent_id(id) {
        state
            .torrents
            .pause(id)
            .await
            .map_err(|error| error.to_string())?;
        set_torrent_status(state, app, id, DownloadStatus::Paused);
    } else {
        state.queue.pause(id as u64);
    }
    Ok(())
}

/// Continues a paused download, retries a failed one, or restores a history
/// row (left unfinished by a restart, paused, failed or cancelled).
#[command]
pub async fn resume_download(
    id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    resume_one(&state, &app, id).await
}

async fn resume_one(state: &AppState, app: &AppHandle, id: i64) -> Result<(), String> {
    if id < 0 {
        return restore_history(state, app, -id).await.map(|_| ());
    }
    let status = state
        .downloads
        .lock()
        .unwrap()
        .get(&(id as u64))
        .map(|entry| entry.status.clone());
    if is_video_id(id) {
        if !resume_video(state, app, id) {
            return Err("Загрузка не найдена".to_string());
        }
    } else if is_torrent_id(id) {
        if status == Some(DownloadStatus::Error) {
            retry_torrent(state, app, id).await?;
            return Ok(());
        }
        state
            .torrents
            .resume(id)
            .await
            .map_err(|error| error.to_string())?;
        set_torrent_status(state, app, id, DownloadStatus::Downloading);
    } else {
        if matches!(
            status,
            Some(DownloadStatus::Error) | Some(DownloadStatus::Cancelled)
        ) && let Some(entry) = state.downloads.lock().unwrap().get_mut(&(id as u64))
        {
            entry.error = None;
        }
        state.queue.resume(id as u64);
    }
    Ok(())
}

fn set_torrent_status(state: &AppState, app: &AppHandle, id: i64, status: DownloadStatus) {
    let payload = {
        let mut guard = state.downloads.lock().unwrap();
        guard.get_mut(&(id as u64)).map(|entry| {
            entry.status = status.clone();
            if status == DownloadStatus::Paused {
                entry.speed = 0;
            }
            let mut payload = serde_json::json!({ "id": id, "status": status_name(&status) });
            if entry.total_size > 0 {
                payload["total"] = serde_json::json!(entry.total_size);
            }
            (payload, entry.db_id)
        })
    };
    if let Some((payload, db_id)) = payload {
        if let (Some(db), Some(row)) = (&state.db, db_id) {
            let db_state = if status == DownloadStatus::Paused {
                DbState::Paused
            } else {
                DbState::Downloading
            };
            let _ = db.set_state(row, db_state);
        }
        let _ = app.emit("download-status", payload);
    }
}

/// Starts a stored download again under a new live id. The history row is
/// kept (videos, torrents) or replaced by the queue's own row (files).
pub async fn restore_history(state: &AppState, app: &AppHandle, row: i64) -> Result<i64, String> {
    let db = state.db.as_ref().ok_or("История недоступна")?;
    let stored = db
        .get(row)
        .map_err(|error| error.to_string())?
        .ok_or("Загрузка не найдена")?;
    if stored.state == DbState::Completed {
        return Err("Загрузка уже завершена".to_string());
    }
    let reuse = Reuse {
        row,
        added_at: stored.created_at.max(0) as u64,
        filename: file_name_of(&stored.target),
    };
    let dir = stored
        .target
        .parent()
        .map(|dir| dir.to_string_lossy().to_string());
    let id = match stored_kind(&stored) {
        "torrent" => {
            let source = stored.url.trim();
            let local = !source.starts_with("magnet:")
                && !source.starts_with("http://")
                && !source.starts_with("https://");
            if source == "magnet:" || (source.starts_with("magnet:?") && !source.contains("xt=")) {
                // 0.2.x stored magnets without their query
                return Err("Ссылка на торрент не сохранилась: добавьте его заново".to_string());
            }
            if local && !Path::new(source).is_file() {
                return Err("Файл .torrent больше не найден: добавьте торрент заново".to_string());
            }
            let only_files = parse_only_files(stored.extra.as_deref());
            // the live entry replaces the history row even when the session
            // rejects the torrent: it then shows the error itself
            let _ = app.emit("download-removed", serde_json::json!({ "id": -row }));
            add_torrent_with(state, app, source, dir.as_deref(), only_files, Some(reuse)).await?
        }
        "video" => {
            let quality = stored.extra.clone();
            let id = add_video_inner(
                state,
                app,
                &stored.url,
                dir.as_deref(),
                quality.as_deref(),
                Some(reuse),
            )
            .await?;
            let _ = app.emit("download-removed", serde_json::json!({ "id": -row }));
            id
        }
        _ => restore_file(state, app, &stored).await? as i64,
    };
    Ok(id)
}

/// Re-queues a stored file download at its old path, so the engine can pick
/// up the `.part` it left. Cookies are never stored, so a host that needs
/// them may refuse; the error then says so.
async fn restore_file(
    state: &AppState,
    app: &AppHandle,
    stored: &dwv_core::StoredDownload,
) -> Result<u64, String> {
    if !(stored.url.starts_with("http://") || stored.url.starts_with("https://")) {
        return Err("Ссылка не сохранилась: добавьте загрузку заново".to_string());
    }
    let mut spec = stored.to_spec();
    {
        let settings = state.settings.lock().unwrap();
        spec.proxy = settings.proxy_url();
        if let Some(agent) = settings.user_agent() {
            spec.headers
                .push(("User-Agent".to_string(), agent.to_string()));
        }
    }
    if let Some(parent) = spec.target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let target = spec.target.clone();
    // the queue writes its own row; the old one would show up twice
    if let Some(db) = &state.db {
        let _ = db.delete(stored.id);
    }
    let _ = app.emit("download-removed", serde_json::json!({ "id": -stored.id }));
    let id = state.queue.add(spec).await.map_err(|e| e.to_string())?;
    let db_id = state
        .db
        .as_ref()
        .and_then(|db| db.latest_id_by_target(&target).ok().flatten());
    let filename = file_name_of(&target);
    insert_live_file(
        state,
        DownloadInfo {
            id: id as i64,
            url: stored.url.clone(),
            category: category_of(&filename),
            filename,
            total_size: stored.total_bytes.unwrap_or(0),
            downloaded: stored.downloaded_bytes,
            speed: 0,
            status: DownloadStatus::Pending,
            error: None,
            eta: 0,
            added_at: stored.created_at.max(0) as u64,
            file_path: Some(target.to_string_lossy().to_string()),
            connections: stored.connections,
            kind: "file".to_string(),
            db_id,
        },
    );
    emit_added(state, app, id as i64);
    Ok(id)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

/// Deletes a file or a directory (torrents store the folder as the target),
/// together with the engine's `.part`/`.dwvmeta`/`.ytdl` leftovers.
fn delete_download_path(path: &Path) -> Result<(), String> {
    let mut error = None;
    for candidate in [
        path.to_path_buf(),
        with_suffix(path, ".part"),
        with_suffix(path, ".dwvmeta"),
        with_suffix(path, ".ytdl"),
    ] {
        if candidate.as_os_str().is_empty() || !candidate.exists() {
            continue;
        }
        let result = if candidate.is_dir() {
            std::fs::remove_dir_all(&candidate)
        } else {
            std::fs::remove_file(&candidate)
        };
        if let Err(failure) = result
            && error.is_none()
        {
            error = Some(format!("{}: {failure}", candidate.display()));
        }
    }
    error.map_or(Ok(()), Err)
}

/// Windows keeps the handle briefly after the writer is killed, so retry.
async fn delete_download_path_retry(path: &Path) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..10 {
        match delete_download_path(path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = error;
                if attempt < 9 {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }
    Err(last)
}

#[command]
pub async fn remove_download(
    id: i64,
    delete_file: bool,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    let result = remove_download_inner(&state, id, delete_file).await;
    let _ = app.emit("download-removed", serde_json::json!({ "id": id }));
    result
}

pub async fn remove_download_inner(
    state: &AppState,
    id: i64,
    delete_file: bool,
) -> Result<(), String> {
    if id < 0 {
        // History row: delete the stored target too when the UI asks for it.
        if let Some(db) = &state.db {
            let mut error = None;
            if let Ok(Some(stored)) = db.get(-id) {
                if delete_file {
                    error = delete_download_path_retry(&stored.target).await.err();
                }
                if stored_kind(&stored) == "torrent" {
                    forget_torrent_file(state, &stored.url);
                }
            }
            let _ = db.delete(-id);
            return error.map_or(Ok(()), Err);
        }
        return Ok(());
    }

    let entry = state.downloads.lock().unwrap().remove(&(id as u64));

    if is_torrent_id(id) {
        let mut error = None;
        if delete_file {
            if let Err(removal) = state.torrents.remove(id, true).await {
                error = Some(removal.to_string());
            }
            // A torrent unknown to the session (after a restart) keeps its files.
            if let Some(path) = entry.as_ref().and_then(|item| item.file_path.as_deref())
                && Path::new(path).exists()
                && let Err(fallback) = delete_download_path_retry(Path::new(path)).await
            {
                error.get_or_insert(fallback);
            }
        } else {
            let _ = state.torrents.remove(id, false).await;
        }
        if let Some(entry) = &entry {
            forget_torrent_file(state, &entry.url);
        }
        if let (Some(db), Some(row)) = (&state.db, entry.as_ref().and_then(|item| item.db_id)) {
            let _ = db.delete(row);
        }
        return error.map_or(Ok(()), Err);
    }

    if is_video_id(id) {
        if let Some(task) = state.video_tasks.lock().unwrap().remove(&id) {
            task.abort();
        }
        state.video_jobs.lock().unwrap().remove(&id);
    } else {
        state.queue.cancel(id as u64);
    }

    let mut file_path = entry.as_ref().and_then(|item| item.file_path.clone());
    if file_path.is_none() {
        // Videos only learn their path when they finish; fall back to history.
        if let (Some(db), Some(row)) = (&state.db, entry.as_ref().and_then(|item| item.db_id)) {
            file_path = db
                .get(row)
                .ok()
                .flatten()
                .map(|stored| stored.target.to_string_lossy().to_string());
        }
    }

    let mut error = None;
    if delete_file && let Some(path) = file_path.as_deref() {
        error = delete_download_path_retry(Path::new(path)).await.err();
    }

    if let Some(entry) = &entry
        && let Some(db) = &state.db
    {
        if let Some(row) = entry.db_id {
            let _ = db.delete(row);
        } else if let Some(path) = &entry.file_path {
            // Older live rows were never bound to their DB id.
            let _ = db.delete_by_target(Path::new(path));
        }
    }
    error.map_or(Ok(()), Err)
}

#[command]
pub async fn pause_all(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    pause_all_inner(&state, &app).await
}

pub async fn pause_all_inner(state: &AppState, app: &AppHandle) -> Result<(), String> {
    let ids: Vec<i64> = state
        .downloads
        .lock()
        .unwrap()
        .values()
        .filter(|item| {
            matches!(
                item.status,
                DownloadStatus::Pending | DownloadStatus::Downloading
            )
        })
        .map(|item| item.id)
        .collect();
    for id in ids {
        let _ = pause_one(state, app, id).await;
    }
    Ok(())
}

#[command]
pub async fn resume_all(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    resume_all_inner(&state, &app).await
}

/// Resumes paused live downloads (failed ones need an explicit retry).
pub async fn resume_all_inner(state: &AppState, app: &AppHandle) -> Result<(), String> {
    let ids: Vec<i64> = state
        .downloads
        .lock()
        .unwrap()
        .values()
        .filter(|item| item.status == DownloadStatus::Paused)
        .map(|item| item.id)
        .collect();
    for id in ids {
        let _ = resume_one(state, app, id).await;
    }
    Ok(())
}

#[command]
pub async fn clear_completed(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    let done: Vec<(i64, Option<i64>, Option<String>)> = state
        .downloads
        .lock()
        .unwrap()
        .values()
        .filter(|download| download.status == DownloadStatus::Completed)
        .map(|download| (download.id, download.db_id, download.file_path.clone()))
        .collect();

    for (id, db_id, file_path) in done {
        if is_torrent_id(id) {
            let _ = state.torrents.remove(id, false).await;
        } else if is_video_id(id) {
            if let Some(task) = state.video_tasks.lock().unwrap().remove(&id) {
                task.abort();
            }
            state.video_jobs.lock().unwrap().remove(&id);
        }
        if let Some(db) = &state.db {
            // Live ids and history ids are separate sequences: only delete
            // the row this entry is bound to.
            match (db_id, file_path) {
                (Some(row), _) => {
                    let _ = db.delete(row);
                }
                (None, Some(path)) => {
                    let _ = db.delete_by_target(Path::new(&path));
                }
                (None, None) => {}
            }
        }
        state.downloads.lock().unwrap().remove(&(id as u64));
        let _ = app.emit("download-removed", serde_json::json!({ "id": id }));
    }

    if let Some(db) = &state.db {
        for stored in db.recent(1_000_000).unwrap_or_default() {
            if stored.state != DbState::Completed {
                continue;
            }
            if stored_kind(&stored) == "torrent" {
                forget_torrent_file(&state, &stored.url);
            }
            let _ = db.delete(stored.id);
            let _ = app.emit("download-removed", serde_json::json!({ "id": -stored.id }));
        }
    }
    Ok(())
}

#[command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.lock().unwrap().clone())
}

#[command]
pub async fn save_settings(settings: Settings, state: State<'_, AppState>) -> Result<(), String> {
    store_settings(&state, settings)
}

fn store_settings(state: &AppState, settings: Settings) -> Result<(), String> {
    settings.save(&state.settings_file)?;
    apply_limits(state, &settings);
    crate::i18n::set_locale(&settings.language);
    let previous = std::mem::replace(&mut *state.settings.lock().unwrap(), settings.clone());
    if previous.autostart != settings.autostart {
        crate::platform::spawn_registry_setup(Some(settings.autostart), false);
    }
    Ok(())
}

/// The UI resolves "auto" to a language; notifications follow it.
#[command]
pub fn set_ui_locale(locale: String) {
    crate::i18n::set_locale(&locale);
}

/// Pushes the concurrency and speed settings into the running queue and torrents.
pub fn apply_limits(state: &AppState, settings: &Settings) {
    state
        .queue
        .set_max_parallel(settings.max_concurrent_downloads as usize);
    let download = download_limit_bps(settings.speed_limit_enabled, settings.speed_limit_mbps);
    state
        .queue
        .set_speed_limit(download.map(|value| value as u64));
    state.torrents.configure(torrent_config(settings));
}

#[command]
pub async fn import_settings(settings: Settings, state: State<'_, AppState>) -> Result<(), String> {
    store_settings(&state, settings)
}

#[command]
pub async fn clear_cache(state: State<'_, AppState>) -> Result<(), String> {
    let cache = state.data_dir.join("cache");
    if cache.exists() {
        std::fs::remove_dir_all(&cache).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Stops everything and forgets the whole history (files stay on disk).
#[command]
pub async fn reset_database(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    let ids: Vec<i64> = state
        .downloads
        .lock()
        .unwrap()
        .keys()
        .map(|id| *id as i64)
        .collect();
    for id in ids {
        if is_torrent_id(id) {
            let _ = state.torrents.remove(id, false).await;
        } else if is_video_id(id) {
            if let Some(task) = state.video_tasks.lock().unwrap().remove(&id) {
                task.abort();
            }
        } else {
            state.queue.cancel(id as u64);
        }
    }
    state.video_jobs.lock().unwrap().clear();
    state.downloads.lock().unwrap().clear();
    if let Some(db) = &state.db {
        db.delete_all().map_err(|error| error.to_string())?;
    }
    let _ = app.emit("database-reset", ());
    Ok(())
}

#[command]
pub async fn check_updates() -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get("https://api.github.com/repos/DezFix/DwV/releases/latest")
        .header("User-Agent", "DwV-Updater")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let release: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let version = release["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v');
    let current = env!("CARGO_PKG_VERSION");

    let is_newer = match (
        semver::Version::parse(version),
        semver::Version::parse(current),
    ) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => !version.is_empty() && version != current,
    };
    if !is_newer {
        return Ok(None);
    }

    let url = release["html_url"]
        .as_str()
        .unwrap_or("https://github.com/DezFix/DwV/releases/latest");

    Ok(Some(UpdateInfo {
        available: true,
        version: version.to_string(),
        notes: release["body"].as_str().unwrap_or("").to_string(),
        url: url.to_string(),
    }))
}

#[command]
pub async fn open_url(url: String) -> Result<(), String> {
    let parsed = url::Url::parse(url.trim()).map_err(|_| "Некорректная ссылка".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Поддерживаются только ссылки http и https".to_string());
    }
    opener::open(parsed.as_str()).map_err(|e| e.to_string())
}

#[command]
pub async fn open_file(path: String) -> Result<(), String> {
    if !Path::new(&path).exists() {
        return Err("Файл не найден".to_string());
    }
    opener::open(path).map_err(|e| e.to_string())
}

#[command]
pub async fn show_in_folder(path: String) -> Result<(), String> {
    crate::platform::show_in_folder(&path)
}

/// A download held back until the user picks a folder in the add dialog.
pub struct PendingAdd {
    pub meta: PendingDownload,
    payload: PendingPayload,
}

pub enum PendingPayload {
    File(DownloadContext),
    Torrent(String),
    Video(String),
}

#[derive(Clone, Serialize)]
pub struct PendingDownload {
    pub id: u64,
    pub kind: String,
    pub url: String,
    pub filename: String,
    pub dir: String,
}

pub(crate) fn should_ask_path(state: &AppState) -> bool {
    state.settings.lock().unwrap().ask_save_path
}

pub(crate) fn queue_pending(
    state: &AppState,
    app: &AppHandle,
    payload: PendingPayload,
    kind: &str,
    url: &str,
    filename: Option<String>,
) {
    let id = state.pending_seq.fetch_add(1, Ordering::Relaxed) + 1;
    let meta = PendingDownload {
        id,
        kind: kind.to_string(),
        url: url.to_string(),
        filename: filename.unwrap_or_else(|| suggest_name(url)),
        dir: state.target_dir().to_string_lossy().to_string(),
    };
    state.pending.lock().unwrap().insert(
        id,
        PendingAdd {
            meta: meta.clone(),
            payload,
        },
    );
    let _ = app.emit("pending-download", meta);
}

#[command]
pub fn pending_downloads(state: State<'_, AppState>) -> Vec<PendingDownload> {
    let pending = state.pending.lock().unwrap();
    let mut items: Vec<_> = pending.values().map(|add| add.meta.clone()).collect();
    items.sort_by_key(|meta| meta.id);
    items
}

#[command]
pub fn cancel_pending(id: u64, state: State<'_, AppState>) {
    let removed = state.pending.lock().unwrap().remove(&id);
    if let Some(PendingAdd {
        payload: PendingPayload::Torrent(source),
        ..
    }) = removed
        && is_cached_torrent(&state, &source)
    {
        let _ = std::fs::remove_file(source);
    }
}

#[command]
pub async fn resolve_pending(
    id: u64,
    dir: String,
    quality: Option<String>,
    only_files: Option<Vec<usize>>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    let Some(add) = state.pending.lock().unwrap().remove(&id) else {
        return Err("Загрузка уже добавлена".to_string());
    };
    let dir = dir.trim().to_string();
    if dir.is_empty() {
        return Err("Выберите папку".to_string());
    }
    let cached = match &add.payload {
        PendingPayload::Torrent(source) if is_cached_torrent(&state, source) => {
            Some(source.clone())
        }
        _ => None,
    };
    match add.payload {
        PendingPayload::File(mut context) => {
            context.dir = Some(PathBuf::from(&dir));
            let id = add_download_with_context(&state, context).await?;
            emit_added(&state, &app, id as i64);
        }
        PendingPayload::Torrent(source) => {
            add_torrent_inner(&state, &app, &source, Some(&dir), only_files).await?;
        }
        PendingPayload::Video(url) => {
            add_video_inner(&state, &app, &url, Some(&dir), quality.as_deref(), None).await?;
        }
    }
    if let Some(path) = cached {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// Queues a link that arrived from Windows (magnet, .torrent, dwv://) for the
/// folder dialog, so a cold start behaves like a browser hand-off.
pub fn queue_deep_link(state: &AppState, app: &AppHandle, target: &str, kind: Option<&str>) {
    let kind = match kind {
        Some("torrent") => "torrent",
        Some("video") => "video",
        Some("file") => "file",
        _ => {
            if is_torrent_source(target) {
                "torrent"
            } else {
                "file"
            }
        }
    };
    let (payload, filename) = match kind {
        "torrent" => (
            PendingPayload::Torrent(target.to_string()),
            Some(dwv_torrent::torrent_name(target)),
        ),
        "video" => (PendingPayload::Video(target.to_string()), None),
        _ => (
            PendingPayload::File(DownloadContext {
                url: target.to_string(),
                ..Default::default()
            }),
            None,
        ),
    };
    queue_pending(state, app, payload, kind, target, filename);
}

pub async fn add_link(
    state: &AppState,
    app: &AppHandle,
    target: &str,
    kind: Option<&str>,
) -> Result<i64, String> {
    let kind = match kind {
        Some("torrent") => "torrent",
        Some("video") => "video",
        Some("file") => "file",
        _ => {
            if is_torrent_source(target) {
                "torrent"
            } else {
                "file"
            }
        }
    };
    match kind {
        "torrent" => add_torrent_inner(state, app, target, None, None).await,
        "video" => add_video_inner(state, app, target, None, None, None).await,
        _ => {
            let id = add_download_inner(state, target).await?;
            emit_added(state, app, id as i64);
            Ok(id as i64)
        }
    }
}

/// Restarts what was running when the app closed: files continue from their
/// `.part`, torrents re-check their pieces, videos let yt-dlp continue.
/// A download that cannot be restored stays in history as interrupted, with
/// the reason, and can be retried by hand.
pub fn resume_pending(state: &AppState, app: &AppHandle) {
    let Some(db) = state.db.clone() else { return };
    let Ok(stored) = db.load_pending() else {
        return;
    };
    if stored.is_empty() {
        return;
    }
    let state = state.clone();
    let app = app.clone();
    state.handle.clone().spawn(async move {
        for item in stored {
            let result = match stored_kind(&item) {
                "file" => restore_file(&state, &app, &item).await.map(|id| id as i64),
                _ => restore_history(&state, &app, item.id).await,
            };
            if let Err(error) = result {
                let _ = db.set_error(item.id, &error);
                let _ = db.finish(item.id, DbState::Interrupted);
            }
        }
    });
}

pub fn app_data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("DwV"))
}
