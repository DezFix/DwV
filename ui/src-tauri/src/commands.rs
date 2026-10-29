use crate::settings::Settings;
use dwv_core::{Database, DownloadSpec, Queue, QueueConfig, QueueEvent, State as DbState};
use dwv_torrent::{TorrentEvent, TorrentManager, TorrentSessionConfig, TorrentState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, command};

pub const TORRENT_ID_BASE: i64 = 1_000_000_000;
pub const VIDEO_ID_BASE: i64 = 2_000_000_000;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn hide_console(command: &mut tokio::process::Command) {
    use std::os::windows::process::CommandExt;
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut tokio::process::Command) {}

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
                        ((total.saturating_sub(*downloaded)) / *speed_bps).max(0)
                    }
                    _ => 0,
                };
            }
            serde_json::json!({
                "id": id,
                "downloaded": downloaded,
                "speed": speed_bps,
                "eta": match total {
                    Some(total) if *speed_bps > 0 => ((total.saturating_sub(*downloaded)) / *speed_bps).max(0),
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
    downloads: Arc<Mutex<HashMap<u64, DownloadInfo>>>,
    mut events: tokio::sync::mpsc::Receiver<QueueEvent>,
    handle: tokio::runtime::Handle,
) {
    handle.spawn(async move {
        while let Some(event) = events.recv().await {
            let payload = {
                let mut guard = downloads.lock().unwrap();
                apply_event(&mut guard, &event)
            };

            let name = match &event {
                QueueEvent::Progress { .. } => "download-progress",
                QueueEvent::Finished { .. } => "download-complete",
                QueueEvent::Failed { .. } => "download-error",
                _ => "download-status",
            };
            let _ = app.emit(name, payload);
        }
    });
}

fn emit_added(state: &AppState, app: &AppHandle, id: i64) {
    emit_added_with_requested(state, app, id, None);
}

fn emit_added_with_requested(
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
        None => resolve_filename_with_headers(&fetch_url, &headers).await,
    };
    let category = category_of(&filename);
    let connections = {
        let settings = state.settings.lock().unwrap();
        settings.connections_per_download.clamp(1, 32) as usize
    };
    let dir = context.dir.clone().unwrap_or_else(|| state.target_dir());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let filename = unique_download_name(&dir, &filename);
    let target = dir.join(&filename);
    let mut spec = DownloadSpec::new(fetch_url, target.clone());
    spec.connections = connections;
    spec.headers = headers;

    let id = state.queue.add(spec).await.map_err(|e| e.to_string())?;
    let db_id = state
        .db
        .as_ref()
        .and_then(|db| db.latest_id_by_target(&target).ok().flatten());

    state.downloads.lock().unwrap().insert(
        id,
        DownloadInfo {
            id: id as i64,
            url: original_url,
            filename,
            total_size: 0,
            downloaded: 0,
            speed: 0,
            status: DownloadStatus::Pending,
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

#[derive(Debug, Clone, Serialize)]
pub struct DownloadPreview {
    pub filename: String,
    pub suggested: String,
    pub exists: bool,
    pub path: String,
}

pub fn unique_download_name(dir: &Path, filename: &str) -> String {
    let filename = sanitize_file_name(filename);
    if !dir.join(&filename).exists() {
        return filename;
    }
    let path = Path::new(&filename);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "download".to_string());
    let extension = path
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    for index in 1..10_000 {
        let candidate = format!("{stem} ({index}){extension}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{stem}-{}{extension}", now_secs())
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
pub async fn add_video(
    url: String,
    dir: Option<String>,
    quality: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<i64, String> {
    add_video_inner(&state, &app, &url, dir.as_deref(), quality.as_deref()).await
}

pub async fn add_video_inner(
    state: &AppState,
    app: &AppHandle,
    url: &str,
    dir: Option<&str>,
    quality: Option<&str>,
) -> Result<i64, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("Пустая ссылка".to_string());
    }
    let quality = quality
        .and_then(dwv_media::VideoQuality::from_key)
        .unwrap_or(dwv_media::VideoQuality::Best);

    let id = VIDEO_ID_BASE + state.torrent_seq.fetch_add(1, Ordering::Relaxed) as i64;
    let dir = dir
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| state.target_dir());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let filename = format!("video-{id}.{}", quality.output_extension());
    let placeholder = dir.join(&filename);
    let db_id = state.db.as_ref().and_then(|db| {
        let mut spec = DownloadSpec::new(url.clone(), placeholder.clone());
        spec.connections = 1;
        db.insert(&spec).ok()
    });

    let item = DownloadInfo {
        id,
        url: url.clone(),
        filename,
        total_size: 0,
        downloaded: 0,
        speed: 0,
        status: DownloadStatus::Pending,
        error: None,
        eta: 0,
        added_at: now_secs(),
        file_path: None,
        category: if quality.is_audio() {
            Category::Audio
        } else {
            Category::Video
        },
        connections: 0,
        kind: "video".to_string(),
        db_id,
    };
    state
        .downloads
        .lock()
        .unwrap()
        .insert(id as u64, item.clone());
    let _ = app.emit("download-added", item);

    let settings = state.settings.lock().unwrap().clone();
    let limit_bps = if settings.speed_limit_enabled {
        download_limit_bps(true, settings.speed_limit_mbps).map(|value| value as u64)
    } else {
        None
    };
    spawn_video_task(state, app, id, url, dir, placeholder, quality, limit_bps);
    Ok(id)
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

fn spawn_video_task(
    state: &AppState,
    app: &AppHandle,
    id: i64,
    url: String,
    dir: PathBuf,
    placeholder: PathBuf,
    quality: dwv_media::VideoQuality,
    limit_bps: Option<u64>,
) {
    let st = state.clone();
    let emitter = app.clone();
    let tasks = state.video_tasks.clone();
    let task = state.handle.spawn(async move {
        let progress_state = st.clone();
        let progress_app = emitter.clone();
        let key = id as u64;

        let result =
            dwv_media::download_video_with_limit(&url, &dir, quality, limit_bps, move |progress| {
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
            })
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
                if let Some(path) = path {
                    let path = path.to_string_lossy().to_string();
                    let filename = file_name_of(Path::new(&path));
                    let category = category_of(&filename);
                    let _ = emitter.emit(
                        "download-complete",
                        serde_json::json!({
                            "id": id,
                            "file_path": path,
                            "total": size,
                            "filename": filename,
                            "category": category,
                        }),
                    );
                    let mut row = db_id;
                    if let Some(db) = &st.db {
                        if let Some(old) = db_id {
                            let _ = db.delete(old);
                        }
                        let mut spec = DownloadSpec::new(url.clone(), PathBuf::from(&path));
                        spec.connections = 1;
                        if let Ok(new_id) = db.insert(&spec) {
                            let _ = db.set_progress(new_id, size, Some(size));
                            let _ = db.finish(new_id, DbState::Completed);
                            row = Some(new_id);
                        }
                    }
                    if let Some(entry) = st.downloads.lock().unwrap().get_mut(&key) {
                        entry.db_id = row;
                    }
                }
            }
            VideoFinish::Failed { error, db_id } => {
                if let Some(db) = &st.db {
                    if let Some(row) = db_id {
                        let _ = db.set_error(row, &error);
                        let _ = db.finish(row, DbState::Failed);
                    }
                }
                let _ = emitter.emit(
                    "download-error",
                    serde_json::json!({ "id": id, "error": error }),
                );
            }
        }
        tasks.lock().unwrap().remove(&id);
    });
    state.video_tasks.lock().unwrap().insert(id, task);
}

#[command]
pub async fn get_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadInfo>, String> {
    let live: Vec<DownloadInfo> = state.downloads.lock().unwrap().values().cloned().collect();
    let mut items = live.clone();

    if let Some(db) = &state.db {
        for stored in db.recent(200).unwrap_or_default() {
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

    items.sort_by(|a, b| b.added_at.cmp(&a.added_at));
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

    if kind == "torrent" {
        if let Some(details) = state.torrents.details(id) {
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

async fn media_for(state: &AppState, item: &DownloadInfo) -> Option<MediaInfo> {
    let path = PathBuf::from(item.file_path.as_ref()?);
    if !path.is_file() {
        return None;
    }
    let filename = file_name_of(&path);
    if !matches!(category_of(&filename), Category::Video | Category::Audio) {
        return None;
    }
    let size = std::fs::metadata(&path).ok()?.len();
    let key = format!("{}|{size}", path.to_string_lossy());
    if let Some(cached) = state.media_cache.lock().unwrap().get(&key).cloned() {
        return Some(cached);
    }
    let media = probe_media(&path).await?;
    state.media_cache.lock().unwrap().insert(key, media.clone());
    Some(media)
}

async fn probe_media(path: &Path) -> Option<MediaInfo> {
    let ffprobe = dwv_media::bin_dir().ok()?.join("ffprobe.exe");
    if !ffprobe.is_file() {
        return None;
    }
    let mut command = tokio::process::Command::new(&ffprobe);
    command
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path);
    hide_console(&mut command);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let format = &value["format"];
    let streams = value["streams"].as_array();
    let video = streams.and_then(|streams| {
        streams
            .iter()
            .find(|stream| stream["codec_type"].as_str() == Some("video"))
    });
    let audio = streams.and_then(|streams| {
        streams
            .iter()
            .find(|stream| stream["codec_type"].as_str() == Some("audio"))
    });

    let duration_secs = format["duration"]
        .as_str()
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            video
                .and_then(|stream| stream["duration"].as_str())
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0.0);
    let width = video
        .and_then(|stream| stream["width"].as_u64())
        .unwrap_or(0) as u32;
    let height = video
        .and_then(|stream| stream["height"].as_u64())
        .unwrap_or(0) as u32;
    let video_codec = video
        .and_then(|stream| stream["codec_name"].as_str())
        .map(str::to_string);
    let audio_codec = audio
        .and_then(|stream| stream["codec_name"].as_str())
        .map(str::to_string);
    let bitrate = format["bit_rate"]
        .as_str()
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            video
                .and_then(|stream| stream["bit_rate"].as_str())
                .and_then(|value| value.parse().ok())
        });
    let container = format["format_name"].as_str().map(str::to_string);
    let frame_rate = video
        .and_then(|stream| stream["avg_frame_rate"].as_str())
        .filter(|value| *value != "0/0")
        .map(str::to_string);

    Some(MediaInfo {
        duration_secs,
        width,
        height,
        video_codec,
        audio_codec,
        bitrate,
        container,
        frame_rate,
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
    if id < 0 || is_video_id(id) {
        return Ok(());
    }
    if is_torrent_id(id) {
        state
            .torrents
            .pause(id)
            .await
            .map_err(|error| error.to_string())?;
        let payload = {
            let mut guard = state.downloads.lock().unwrap();
            guard.get_mut(&(id as u64)).map(|entry| {
                entry.status = DownloadStatus::Paused;
                entry.speed = 0;
                let mut payload = serde_json::json!({ "id": id, "status": "paused" });
                if entry.total_size > 0 {
                    payload["total"] = serde_json::json!(entry.total_size);
                }
                (payload, entry.db_id)
            })
        };
        if let Some((payload, db_id)) = payload {
            if let (Some(db), Some(row)) = (&state.db, db_id) {
                let _ = db.set_state(row, DbState::Paused);
            }
            let _ = app.emit("download-status", payload);
        }
    } else {
        state.queue.pause(id as u64);
    }
    Ok(())
}

#[command]
pub async fn resume_download(
    id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    if id < 0 || is_video_id(id) {
        return Ok(());
    }
    if is_torrent_id(id) {
        state
            .torrents
            .resume(id)
            .await
            .map_err(|error| error.to_string())?;
        let payload = {
            let mut guard = state.downloads.lock().unwrap();
            guard.get_mut(&(id as u64)).map(|entry| {
                entry.status = DownloadStatus::Downloading;
                let mut payload = serde_json::json!({ "id": id, "status": "downloading" });
                if entry.total_size > 0 {
                    payload["total"] = serde_json::json!(entry.total_size);
                }
                (payload, entry.db_id)
            })
        };
        if let Some((payload, db_id)) = payload {
            if let (Some(db), Some(row)) = (&state.db, db_id) {
                let _ = db.set_state(row, DbState::Downloading);
            }
            let _ = app.emit("download-status", payload);
        }
    } else {
        state.queue.resume(id as u64);
    }
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

/// Deletes a file or a directory (torrents store the folder as the target),
/// together with the engine's `.part`/`.ytdl` leftovers.
fn delete_download_path(path: &Path) -> Result<(), String> {
    let mut error = None;
    for candidate in [
        path.to_path_buf(),
        with_suffix(path, ".part"),
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
            if delete_file && let Ok(Some(stored)) = db.get(-id) {
                error = delete_download_path_retry(&stored.target).await.err();
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
            if let Some(path) = entry.as_ref().and_then(|item| item.file_path.as_deref()) {
                if Path::new(path).exists()
                    && let Err(fallback) = delete_download_path_retry(Path::new(path)).await
                {
                    error.get_or_insert(fallback);
                }
            }
        } else {
            let _ = state.torrents.remove(id, false).await;
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

    if let Some(entry) = &entry {
        if let Some(db) = &state.db {
            if let Some(row) = entry.db_id {
                let _ = db.delete(row);
            } else if let Some(path) = &entry.file_path {
                // Older live rows were never bound to their DB id.
                let _ = db.delete_by_target(Path::new(path));
            }
        }
    }
    error.map_or(Ok(()), Err)
}

#[command]
pub async fn pause_all(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    pause_all_inner(&state, &app).await
}

pub async fn pause_all_inner(state: &AppState, app: &AppHandle) -> Result<(), String> {
    let entries: Vec<(i64, DownloadStatus)> = state
        .downloads
        .lock()
        .unwrap()
        .values()
        .map(|download| (download.id, download.status.clone()))
        .collect();
    for (id, status) in entries {
        if status != DownloadStatus::Pending && status != DownloadStatus::Downloading {
            continue;
        }
        if is_torrent_id(id) {
            if state.torrents.pause(id).await.is_ok() {
                let payload = {
                    let mut guard = state.downloads.lock().unwrap();
                    guard.get_mut(&(id as u64)).map(|entry| {
                        entry.status = DownloadStatus::Paused;
                        entry.speed = 0;
                        let mut payload = serde_json::json!({ "id": id, "status": "paused" });
                        if entry.total_size > 0 {
                            payload["total"] = serde_json::json!(entry.total_size);
                        }
                        (payload, entry.db_id)
                    })
                };
                if let Some((payload, db_id)) = payload {
                    if let (Some(db), Some(row)) = (&state.db, db_id) {
                        let _ = db.set_state(row, DbState::Paused);
                    }
                    let _ = app.emit("download-status", payload);
                }
            }
        } else if !is_video_id(id) {
            state.queue.pause(id as u64);
        }
    }
    Ok(())
}

#[command]
pub async fn resume_all(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    resume_all_inner(&state, &app).await
}

pub async fn resume_all_inner(state: &AppState, app: &AppHandle) -> Result<(), String> {
    let entries: Vec<(i64, DownloadStatus)> = state
        .downloads
        .lock()
        .unwrap()
        .values()
        .map(|download| (download.id, download.status.clone()))
        .collect();
    for (id, status) in entries {
        if is_video_id(id) {
            continue;
        }
        if is_torrent_id(id) {
            if status != DownloadStatus::Paused {
                continue;
            }
            if state.torrents.resume(id).await.is_ok() {
                let payload = {
                    let mut guard = state.downloads.lock().unwrap();
                    guard.get_mut(&(id as u64)).map(|entry| {
                        entry.status = DownloadStatus::Downloading;
                        let mut payload = serde_json::json!({ "id": id, "status": "downloading" });
                        if entry.total_size > 0 {
                            payload["total"] = serde_json::json!(entry.total_size);
                        }
                        (payload, entry.db_id)
                    })
                };
                if let Some((payload, db_id)) = payload {
                    if let (Some(db), Some(row)) = (&state.db, db_id) {
                        let _ = db.set_state(row, DbState::Downloading);
                    }
                    let _ = app.emit("download-status", payload);
                }
            }
        } else {
            state.queue.resume(id as u64);
        }
    }
    Ok(())
}

#[command]
pub async fn clear_completed(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    let ids: Vec<(i64, Option<i64>)> = state
        .downloads
        .lock()
        .unwrap()
        .values()
        .filter(|download| download.status == DownloadStatus::Completed)
        .map(|download| (download.id, download.db_id))
        .collect();

    for (id, db_id) in ids {
        if is_torrent_id(id) {
            let _ = state.torrents.remove(id, false).await;
        } else if is_video_id(id) {
            if let Some(task) = state.video_tasks.lock().unwrap().remove(&id) {
                task.abort();
            }
        }
        if let Some(db) = &state.db {
            let _ = db.delete(db_id.unwrap_or(id));
        }
        state.downloads.lock().unwrap().remove(&(id as u64));
        let _ = app.emit("download-removed", serde_json::json!({ "id": id }));
    }

    if let Some(db) = &state.db {
        for stored in db.recent(1000).unwrap_or_default() {
            if stored.state != DbState::Completed {
                continue;
            }
            let _ = db.delete(stored.id);
            let _ = app.emit(
                "download-removed",
                serde_json::json!({ "id": -(stored.id as i64) }),
            );
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
    settings.save(&state.settings_file)?;
    apply_limits(&state, &settings);
    apply_autostart(settings.autostart);
    let mut current = state.settings.lock().unwrap();
    *current = settings;
    Ok(())
}

/// Adds or removes the current exe from the Windows Run key.
pub fn apply_autostart(enabled: bool) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let exe = exe.to_string_lossy().to_string();
        if enabled {
            let command = format!(r#""{}" --autostart"#, exe);
            let _ = std::process::Command::new("reg")
                .args([
                    "add",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    "DwV",
                    "/t",
                    "REG_SZ",
                    "/d",
                    &command,
                    "/f",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        } else {
            let _ = std::process::Command::new("reg")
                .args([
                    "delete",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    "DwV",
                    "/f",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
    }
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
    settings.save(&state.settings_file)?;
    apply_limits(&state, &settings);
    apply_autostart(settings.autostart);
    let mut current = state.settings.lock().unwrap();
    *current = settings;
    Ok(())
}

#[command]
pub async fn clear_cache(state: State<'_, AppState>) -> Result<(), String> {
    let cache = state.data_dir.join("cache");
    if cache.exists() {
        std::fs::remove_dir_all(&cache).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[command]
pub async fn reset_database(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    state.downloads.lock().unwrap().clear();
    if let Some(db) = &state.db {
        for stored in db.recent(1000).unwrap_or_default() {
            let _ = db.delete(stored.id);
        }
    }
    let _ = app.emit("database-reset", ());
    Ok(())
}

#[command]
pub async fn check_updates() -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::Client::new();
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
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        let parent = PathBuf::from(&path)
            .parent()
            .unwrap_or(PathBuf::from("/"))
            .to_string_lossy()
            .to_string();
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A download held back until the user picks a folder in the add dialog.
pub struct PendingAdd {
    pub meta: PendingDownload,
    payload: PendingPayload,
}

enum PendingPayload {
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

fn should_ask_path(state: &AppState) -> bool {
    state.settings.lock().unwrap().ask_save_path
}

fn suggest_name(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .filter(|name| !name.is_empty())
        .unwrap_or("download")
        .to_string()
}

fn queue_pending(
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
    state.pending.lock().unwrap().remove(&id);
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
            add_video_inner(&state, &app, &url, Some(&dir), quality.as_deref()).await?;
        }
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

/// Starts the local HTTP endpoint the browser extension posts links to.
pub fn start_ipc_server(state: AppHandle, ipc_state: AppState) {
    let handle = ipc_state.handle.clone();
    handle.spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<dwv_ipc::Request>();

        tokio::spawn(async move {
            let _ = dwv_ipc::serve(41763, tx).await;
        });

        while let Some(request) = rx.recv().await {
            // Any request from the extension means something is happening; the
            // window may be hidden in the tray, so bring it up as feedback.
            crate::tray::show_window(&state);
            match request {
                dwv_ipc::Request::Add {
                    url,
                    filename,
                    referrer,
                } => {
                    if should_ask_path(&ipc_state) {
                        let context = DownloadContext {
                            url: url.clone(),
                            filename: filename.clone(),
                            referrer,
                            ..Default::default()
                        };
                        queue_pending(
                            &ipc_state,
                            &state,
                            PendingPayload::File(context),
                            "file",
                            &url,
                            filename,
                        );
                        continue;
                    }
                    let context = DownloadContext {
                        url,
                        filename,
                        referrer,
                        ..Default::default()
                    };
                    if let Ok(id) = add_download_with_context(&ipc_state, context).await {
                        emit_added(&ipc_state, &state, id as i64);
                    }
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
                    if should_ask_path(&ipc_state) {
                        let context = DownloadContext {
                            url: url.clone(),
                            final_url,
                            filename: filename.clone(),
                            referrer,
                            user_agent,
                            cookies,
                            headers,
                            dir: None,
                        };
                        queue_pending(
                            &ipc_state,
                            &state,
                            PendingPayload::File(context),
                            "file",
                            &url,
                            filename,
                        );
                        continue;
                    }
                    let requested = filename.clone();
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
                    if let Ok(id) = add_download_with_context(&ipc_state, context).await {
                        emit_added_with_requested(
                            &ipc_state,
                            &state,
                            id as i64,
                            requested.as_deref(),
                        );
                    }
                }
                dwv_ipc::Request::Torrent { source, .. } => {
                    // The file picker is mandatory for torrents, so queue them even
                    // when the folder dialog is disabled.
                    let name = dwv_torrent::torrent_name(&source);
                    queue_pending(
                        &ipc_state,
                        &state,
                        PendingPayload::Torrent(source.clone()),
                        "torrent",
                        &source,
                        Some(name),
                    );
                    continue;
                }
                dwv_ipc::Request::Video { url, .. } => {
                    if should_ask_path(&ipc_state) {
                        queue_pending(
                            &ipc_state,
                            &state,
                            PendingPayload::Video(url.clone()),
                            "video",
                            &url,
                            None,
                        );
                        continue;
                    }
                    let _ = add_video_inner(&ipc_state, &state, &url, None, None).await;
                }
                dwv_ipc::Request::Ping => {}
            }
        }
    });
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentFileInfo {
    pub index: usize,
    pub path: String,
    pub size: u64,
    pub downloaded: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TorrentMetaInfo {
    pub info_hash: String,
    pub name: String,
    pub total_size: u64,
    pub files: Vec<TorrentFileInfo>,
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

    // Magnets wait for metadata from peers; don't keep the dialog spinning forever.
    let inspected = tokio::time::timeout(
        Duration::from_secs(40),
        state.torrents.inspect(&source, &dir),
    )
    .await
    .map_err(|_| "Не удалось получить список файлов: трекер не ответил".to_string())?
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
    let name = dwv_torrent::torrent_name(&source);
    let target = dir.join(&name);
    if target.exists() {
        return Err(format!("Файл или папка с именем «{}» уже существует", name));
    }
    let paused = !settings.torrent_autostart;

    let db_id = state.db.as_ref().and_then(|db| {
        let mut spec = DownloadSpec::new(source.clone(), target.clone());
        spec.connections = 1;
        let row = db.insert(&spec).ok();
        if paused {
            if let Some(row) = row {
                let _ = db.set_state(row, DbState::Paused);
            }
        }
        row
    });

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
        added_at: now_secs(),
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
            }
        }
        TorrentEvent::Error { id, error } => {
            let db_id = {
                let mut guard = state.downloads.lock().unwrap();
                guard.get_mut(&(id as u64)).map(|entry| {
                    entry.status = DownloadStatus::Error;
                    entry.error = Some(error.clone());
                    entry.speed = 0;
                    entry.db_id
                })
            };
            if let (Some(db), Some(Some(row))) = (&state.db, db_id) {
                let _ = db.set_error(row, &error);
                let _ = db.finish(row, DbState::Failed);
            }
            let _ = app.emit(
                "download-error",
                serde_json::json!({ "id": id, "error": error }),
            );
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

fn status_name(status: &DownloadStatus) -> &'static str {
    match status {
        DownloadStatus::Pending => "pending",
        DownloadStatus::Downloading => "downloading",
        DownloadStatus::Paused => "paused",
        DownloadStatus::Completed => "completed",
        DownloadStatus::Error => "error",
        DownloadStatus::Cancelled => "cancelled",
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string())
}

pub fn category_of(filename: &str) -> Category {
    let ext = Path::new(filename)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" => Category::Audio,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" => Category::Archive,
        "exe" | "msi" | "iso" | "dmg" | "msix" | "apk" | "deb" | "rpm" => Category::Program,
        "pdf" | "doc" | "docx" | "txt" | "md" | "epub" | "xls" | "xlsx" | "ppt" | "pptx"
        | "csv" => Category::Document,
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "mpg" | "mpeg" | "flv" | "m4v" | "ts" => {
            Category::Video
        }
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "tif" | "tiff" | "heic"
        | "avif" => Category::Image,
        _ => Category::Other,
    }
}

/// Asks the server for the real file name, falling back to the URL path.
pub async fn resolve_filename(url: &str) -> String {
    resolve_filename_with_headers(url, &[]).await
}

async fn resolve_filename_with_headers(url: &str, headers: &[(String, String)]) -> String {
    if let Ok(client) = reqwest::Client::builder()
        .http1_only()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        let mut map = reqwest::header::HeaderMap::new();
        for (name, value) in headers {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::try_from(name.as_str()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                map.insert(name, value);
            }
        }
        if let Ok(resp) = client.head(url).headers(map).send().await {
            if let Some(name) = resp
                .headers()
                .get(reqwest::header::CONTENT_DISPOSITION)
                .and_then(|v| v.to_str().ok())
                .and_then(filename_from_disposition)
            {
                return name;
            }
        }
    }

    from_url(url)
}

pub fn filename_from_disposition(value: &str) -> Option<String> {
    let star = value.split("filename*=").nth(1)?;
    let raw = star.split(';').next()?.trim().trim_matches('\'');
    let encoded = raw.rsplit("''").next()?;
    let decoded = percent_decode(encoded);
    let name = Path::new(&decoded)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())?;
    let name = sanitize_file_name(&name);
    if name.is_empty() { None } else { Some(name) }
}

/// Keeps a remote name usable as one path segment, so a crafted URL cannot
/// escape the download folder.
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned.chars().take(180).collect()
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

pub fn from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("").trim();
    if name.is_empty() {
        "download".to_string()
    } else {
        sanitize_file_name(&percent_decode(name))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadInfo {
    /// Live queue entries use the queue id, history rows use a negative id so
    /// the two numbering spaces never collide in the UI.
    pub id: i64,
    pub url: String,
    pub filename: String,
    pub total_size: u64,
    pub downloaded: u64,
    pub speed: u64,
    pub status: DownloadStatus,
    pub error: Option<String>,
    pub eta: u64,
    pub added_at: u64,
    pub file_path: Option<String>,
    pub category: Category,
    pub connections: usize,
    pub kind: String,
    #[serde(skip)]
    pub db_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Pending,
    Downloading,
    Paused,
    Completed,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Video,
    Audio,
    Document,
    Archive,
    Program,
    Image,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_rate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub address: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    pub downloaded: u64,
    pub uploaded: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    pub downloaded: u64,
    pub uploaded: u64,
    pub ratio: f64,
    pub peers_live: u32,
    pub peers_seen: u32,
    pub files: Vec<TorrentFileInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadDetails {
    pub id: i64,
    pub kind: String,
    pub url: String,
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    pub total_size: u64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub speed: u64,
    pub status: DownloadStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub category: Category,
    pub added_at: u64,
    pub connections: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub torrent: Option<TorrentInfo>,
    pub peers: Vec<PeerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub version: String,
    pub notes: String,
    pub url: String,
}

impl DownloadInfo {
    pub fn new(id: u64, path: PathBuf) -> Self {
        let filename = file_name_of(&path);
        Self {
            id: id as i64,
            url: String::new(),
            category: category_of(&filename),
            filename,
            total_size: 0,
            downloaded: 0,
            speed: 0,
            status: DownloadStatus::Pending,
            error: None,
            eta: 0,
            added_at: now_secs(),
            file_path: None,
            connections: 0,
            kind: "file".to_string(),
            db_id: None,
        }
    }

    pub fn from_stored(stored: &dwv_core::StoredDownload) -> Self {
        let filename = file_name_of(&stored.target);
        let status = match stored.state {
            dwv_core::State::Completed => DownloadStatus::Completed,
            dwv_core::State::Failed => DownloadStatus::Error,
            dwv_core::State::Paused => DownloadStatus::Paused,
            dwv_core::State::Cancelled => DownloadStatus::Cancelled,
            dwv_core::State::Queued | dwv_core::State::Downloading => DownloadStatus::Pending,
            // Torrent/video jobs are not resumed after a restart, but showing
            // them as failed would be misleading.
            dwv_core::State::Interrupted => DownloadStatus::Paused,
        };
        Self {
            id: -(stored.id as i64),
            url: stored.url.clone(),
            category: category_of(&filename),
            filename,
            total_size: stored.total_bytes.unwrap_or(0),
            downloaded: stored.downloaded_bytes,
            speed: 0,
            status,
            error: stored.error.clone(),
            eta: 0,
            added_at: stored.created_at as u64,
            file_path: Some(stored.target.to_string_lossy().to_string()),
            connections: stored.connections,
            kind: infer_kind(&stored.url, &file_name_of(&stored.target)).to_string(),
            db_id: Some(stored.id),
        }
    }
}

pub fn infer_kind(url: &str, filename: &str) -> &'static str {
    if is_torrent_source(url) {
        return "torrent";
    }
    if matches!(category_of(filename), Category::Video | Category::Audio)
        && !matches!(
            category_of(&from_url(url)),
            Category::Video | Category::Audio
        )
    {
        return "video";
    }
    "file"
}

pub fn is_torrent_source(source: &str) -> bool {
    let source = source.trim();
    if source.starts_with("magnet:") {
        return true;
    }
    if let Some(rest) = source
        .strip_prefix("http://")
        .or_else(|| source.strip_prefix("https://"))
    {
        let path = rest.split(['?', '#']).next().unwrap_or("");
        return path.to_ascii_lowercase().ends_with(".torrent");
    }
    Path::new(source)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
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
        "video" => add_video_inner(state, app, target, None, None).await,
        _ => {
            let id = add_download_inner(state, target).await?;
            emit_added(state, app, id as i64);
            Ok(id as i64)
        }
    }
}

/// Restores unfinished downloads after a restart.
pub fn resume_pending(state: &AppState) {
    let Some(db) = &state.db else { return };
    let Ok(stored) = db.load_pending() else {
        return;
    };

    let mut resumable = Vec::new();
    for item in stored {
        if infer_kind(&item.url, &file_name_of(&item.target)) != "file" {
            // Torrent and video jobs are owned by their managers, not the HTTP queue.
            let _ = db.finish(item.id, DbState::Interrupted);
            continue;
        }
        // The stale row would show up next to the re-queued download.
        let _ = db.delete(item.id);
        resumable.push(item);
    }

    let queue = state.queue.clone();
    let downloads = state.downloads.clone();
    let db = state.db.clone();
    let connections = state
        .settings
        .lock()
        .unwrap()
        .connections_per_download
        .clamp(1, 32) as usize;
    state.handle.spawn(async move {
        for item in resumable {
            let mut spec = item.to_spec();
            spec.connections = connections;
            let path = spec.target.clone();
            let filename = file_name_of(&path);
            let category = category_of(&filename);

            if let Ok(id) = queue.add(spec).await {
                let db_id = db
                    .as_ref()
                    .and_then(|db| db.latest_id_by_target(&path).ok().flatten());
                downloads.lock().unwrap().insert(
                    id,
                    DownloadInfo {
                        id: id as i64,
                        url: item.url.clone(),
                        filename,
                        total_size: item.total_bytes.unwrap_or(0),
                        downloaded: item.downloaded_bytes,
                        speed: 0,
                        status: DownloadStatus::Pending,
                        error: item.error.clone(),
                        eta: 0,
                        added_at: item.created_at as u64,
                        file_path: Some(path.to_string_lossy().to_string()),
                        category,
                        connections: item.connections,
                        kind: "file".to_string(),
                        db_id,
                    },
                );
            }
        }
    });
}

pub fn app_data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("DwV"))
}

/// Parses a `dwv://add?url=<encoded>` deep link into the target URL.
pub fn parse_deep_link(link: &str) -> Option<String> {
    let link = link.trim();
    let rest = link.strip_prefix("dwv://")?;
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p.trim_end_matches('/'), q),
        None => (rest.trim_end_matches('/'), ""),
    };
    if !path.is_empty() && path != "add" {
        return None;
    }
    for param in query.split('&') {
        if param.is_empty() {
            continue;
        }
        let (key, value) = param.split_once('=')?;
        if key == "url" {
            return Some(percent_decode(value));
        }
    }
    // bare dwv:// opens the app without adding anything
    if query.is_empty() {
        return Some(String::new());
    }
    None
}

pub fn deep_link_type(link: &str) -> Option<String> {
    let link = link.trim();
    let rest = link.strip_prefix("dwv://")?;
    let query = rest.split_once('?').map(|x| x.1).unwrap_or("");
    for param in query.split('&') {
        if param.is_empty() {
            continue;
        }
        if let Some((key, value)) = param.split_once('=') {
            if key == "type" {
                return Some(percent_decode(value));
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn reg_add(key: &str, name: Option<&str>, value: &str) {
    use std::os::windows::process::CommandExt;

    let mut command = std::process::Command::new("reg");
    command.args(["add", key]);
    match name {
        Some(name) => {
            command.args(["/v", name]);
        }
        None => {
            command.arg("/ve");
        }
    }
    command
        .args(["/d", value, "/f"])
        .creation_flags(CREATE_NO_WINDOW);
    let _ = command.status();
}

/// Registers `dwv://` and `magnet:` for the current executable (Windows).
///
/// The magnet handler needs a ProgID listed in `RegisteredApplications`:
/// Windows' "Open with" dialog lists handlers from app capabilities, not from
/// the merged `magnet` class alone.
pub fn register_protocol() {
    #[cfg(target_os = "windows")]
    {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let exe = exe.to_string_lossy().to_string();
        let icon = format!("\"{exe}\"");
        let command = format!("\"{exe}\" \"%1\"");

        // dwv:// is ours alone, a direct class is enough.
        reg_add(r"HKCU\Software\Classes\dwv", None, "URL:dwv Protocol");
        reg_add(r"HKCU\Software\Classes\dwv", Some("URL Protocol"), "");
        reg_add(r"HKCU\Software\Classes\dwv\DefaultIcon", None, &icon);
        reg_add(
            r"HKCU\Software\Classes\dwv\shell\open\command",
            None,
            &command,
        );

        // magnet: a proper ProgID plus an OpenWith entry.
        reg_add(
            r"HKCU\Software\Classes\DwV.magnet",
            None,
            "URL:magnet Protocol",
        );
        reg_add(r"HKCU\Software\Classes\DwV.magnet", Some("URL Protocol"), "");
        reg_add(
            r"HKCU\Software\Classes\DwV.magnet\DefaultIcon",
            None,
            &icon,
        );
        reg_add(
            r"HKCU\Software\Classes\DwV.magnet\shell\open\command",
            None,
            &command,
        );
        reg_add(
            r"HKCU\Software\Classes\magnet\OpenWithProgids",
            Some("DwV.magnet"),
            "",
        );

        // Keep the direct class so a fresh install can open magnets before the
        // user picks a default application.
        reg_add(r"HKCU\Software\Classes\magnet", None, "URL:magnet Protocol");
        reg_add(r"HKCU\Software\Classes\magnet", Some("URL Protocol"), "");
        reg_add(r"HKCU\Software\Classes\magnet\DefaultIcon", None, &icon);
        reg_add(
            r"HKCU\Software\Classes\magnet\shell\open\command",
            None,
            &command,
        );

        // Default Apps entry (Windows lists these in the "Open with" dialog).
        reg_add(
            r"HKCU\Software\DwV\Capabilities",
            Some("ApplicationName"),
            "DwV",
        );
        reg_add(
            r"HKCU\Software\DwV\Capabilities",
            Some("ApplicationDescription"),
            "DwV Download Manager",
        );
        reg_add(
            r"HKCU\Software\DwV\Capabilities",
            Some("ApplicationIcon"),
            &icon,
        );
        reg_add(
            r"HKCU\Software\DwV\Capabilities\UrlAssociations",
            Some("magnet"),
            "DwV.magnet",
        );
        reg_add(
            r"HKCU\Software\DwV\Capabilities\UrlAssociations",
            Some("dwv"),
            "dwv",
        );
        reg_add(
            r"HKCU\Software\RegisteredApplications",
            Some("DwV"),
            r"Software\DwV\Capabilities",
        );
    }
}
