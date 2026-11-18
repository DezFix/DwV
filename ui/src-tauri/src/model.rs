//! Data shared with the frontend. Field names are part of the UI contract
//! (`docs/feature-contract.md`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::names::{file_name_of, from_url};

pub fn status_name(status: &DownloadStatus) -> &'static str {
    match status {
        DownloadStatus::Pending => "pending",
        DownloadStatus::Downloading => "downloading",
        DownloadStatus::Paused => "paused",
        DownloadStatus::Completed => "completed",
        DownloadStatus::Error => "error",
        DownloadStatus::Cancelled => "cancelled",
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
            // Left unfinished by a restart; "resume" picks it up again.
            dwv_core::State::Interrupted => DownloadStatus::Paused,
        };
        Self {
            id: -stored.id,
            url: stored.url.clone(),
            category: category_of(&filename),
            total_size: stored.total_bytes.unwrap_or(0),
            downloaded: stored.downloaded_bytes,
            speed: 0,
            status,
            error: stored.error.clone(),
            eta: 0,
            added_at: stored.created_at as u64,
            file_path: Some(stored.target.to_string_lossy().to_string()),
            connections: stored.connections,
            kind: stored_kind(stored).to_string(),
            db_id: Some(stored.id),
            filename,
        }
    }
}

/// The kind recorded with the row, or a guess for rows from 0.2.x.
pub fn stored_kind(stored: &dwv_core::StoredDownload) -> &'static str {
    match stored.kind.as_deref() {
        Some("torrent") => "torrent",
        Some("video") => "video",
        Some("file") => "file",
        _ => infer_kind(&stored.url, &file_name_of(&stored.target)),
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
