//! Media details (resolution, codecs) for finished files, read with ffprobe.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::commands::AppState;
use crate::model::{Category, DownloadInfo, MediaInfo, category_of};
use crate::names::file_name_of;
use crate::platform::hide_console;

pub async fn media_for(state: &AppState, item: &DownloadInfo) -> Option<MediaInfo> {
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
