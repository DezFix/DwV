use std::path::{Path, PathBuf};
use std::process::Stdio;

use futures_util::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const FFMPEG_URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-lgpl.zip";
const PROGRESS_PREFIX: &str = "dwv:";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("APPDATA is not set")]
    NoAppData,
    #[error("yt-dlp failed ({code:?}): {stderr}")]
    Process { code: Option<i32>, stderr: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed_bps: Option<u64>,
}

pub fn bin_dir() -> Result<PathBuf, Error> {
    let appdata = std::env::var_os("APPDATA").ok_or(Error::NoAppData)?;
    Ok(PathBuf::from(appdata).join("DwV").join("bin"))
}

pub fn ytdlp_path() -> Result<PathBuf, Error> {
    Ok(bin_dir()?.join("yt-dlp.exe"))
}

pub async fn ensure_ytdlp() -> Result<PathBuf, Error> {
    let path = ytdlp_path()?;
    if path.is_file() {
        return Ok(path);
    }
    // todo: version check and self-update on start
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("exe.part");
    download_to(YTDLP_URL, &tmp).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(path)
}

/// Makes sure ffmpeg.exe and ffprobe.exe are in the bin dir. Returns the bin dir.
pub async fn ensure_ffmpeg() -> Result<PathBuf, Error> {
    let bin = bin_dir()?;
    if bin.join("ffmpeg.exe").is_file() && bin.join("ffprobe.exe").is_file() {
        return Ok(bin);
    }
    tokio::fs::create_dir_all(&bin).await?;
    let zip_path = bin.join("ffmpeg.zip.part");
    // todo: verify a checksum; the installer will ship a pinned build later
    download_to(FFMPEG_URL, &zip_path).await?;
    let extracted = extract_bin_files(&zip_path, &bin).await;
    let _ = tokio::fs::remove_file(&zip_path).await;
    extracted?;
    if !bin.join("ffmpeg.exe").is_file() {
        return Err(Error::Io(std::io::Error::other(
            "ffmpeg.exe not found after extraction",
        )));
    }
    Ok(bin)
}

async fn download_to(url: &str, path: &Path) -> Result<(), Error> {
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let mut file = tokio::fs::File::create(path).await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    Ok(())
}

// shortcut: uses the built-in tar.exe of Windows 10+ to read the zip
async fn extract_bin_files(zip_path: &Path, bin: &Path) -> Result<(), Error> {
    let status = Command::new("tar")
        .arg("-xf")
        .arg(zip_path)
        .arg("-C")
        .arg(bin)
        .status()
        .await?;
    if !status.success() {
        return Err(Error::Io(std::io::Error::other("tar failed to extract")));
    }
    // the archive layout is ffmpeg-*/bin/*.exe
    let mut dirs = tokio::fs::read_dir(bin).await?;
    while let Some(entry) = dirs.next_entry().await? {
        let path = entry.path();
        if !entry.file_type().await?.is_dir() || !path.join("bin").is_dir() {
            continue;
        }
        let mut files = tokio::fs::read_dir(path.join("bin")).await?;
        while let Some(file) = files.next_entry().await? {
            let name = file.file_name();
            if Path::new(&name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            {
                tokio::fs::rename(file.path(), bin.join(&name)).await?;
            }
        }
        let _ = tokio::fs::remove_dir_all(&path).await;
    }
    Ok(())
}

/// Downloads a single video. `on_progress` is called from the reading loop, keep it quick.
pub async fn download_video(
    url: &str,
    dir: &Path,
    mut on_progress: impl FnMut(VideoProgress),
) -> Result<Option<PathBuf>, Error> {
    let ytdlp = ensure_ytdlp().await?;
    let ffmpeg_dir = ensure_ffmpeg().await?;
    let mut child = Command::new(&ytdlp)
        .arg("--newline")
        .arg("--no-playlist")
        .arg("--windows-filenames")
        // shortcut: rely on a system JS runtime; bundle deno in the installer later
        .arg("--js-runtimes")
        .arg("node")
        .arg("--ffmpeg-location")
        .arg(&ffmpeg_dir)
        .arg("--trim-filenames")
        .arg("150")
        .arg("--progress-template")
        .arg(format!(
            "{PROGRESS_PREFIX}%(progress.downloaded_bytes)s %(progress.total_bytes_estimate)s %(progress.speed)s"
        ))
        .arg("-f")
        .arg("bv*+ba/b")
        .arg("-P")
        .arg(dir)
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let stderr_task = tokio::spawn(collect_stderr(stderr));

    let mut last_path = None;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        if let Some(progress) = parse_progress(&line) {
            on_progress(progress);
        } else if let Some(path) = parse_destination(&line) {
            last_path = Some(path);
        }
    }

    let status = child.wait().await?;
    let stderr_lines = stderr_task.await.unwrap_or_default();
    if !status.success() {
        return Err(Error::Process {
            code: status.code(),
            stderr: stderr_lines.join("\n"),
        });
    }
    Ok(last_path)
}

async fn collect_stderr(stderr: tokio::process::ChildStderr) -> Vec<String> {
    let mut tail = Vec::new();
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tail.len() == 20 {
            tail.remove(0);
        }
        tail.push(line);
    }
    tail
}

fn parse_progress(line: &str) -> Option<VideoProgress> {
    let rest = line.trim().strip_prefix(PROGRESS_PREFIX)?;
    let mut parts = rest.split_whitespace();
    let downloaded = parts.next()?.parse().ok()?;
    let total = parts.next().and_then(|v| v.parse().ok());
    let speed_bps = parts.next().and_then(|v| v.parse().ok());
    Some(VideoProgress {
        downloaded,
        total,
        speed_bps,
    })
}

fn parse_destination(line: &str) -> Option<PathBuf> {
    let line = line.trim();
    for marker in ["[download] Destination: ", "[Merger] Merging formats into "] {
        if let Some(rest) = line.strip_prefix(marker) {
            let path = rest.trim().trim_matches('"').trim();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines() {
        assert_eq!(
            parse_progress("dwv:1024 4096 512"),
            Some(VideoProgress {
                downloaded: 1024,
                total: Some(4096),
                speed_bps: Some(512)
            })
        );
        assert_eq!(
            parse_progress("dwv:1024 NA NA"),
            Some(VideoProgress {
                downloaded: 1024,
                total: None,
                speed_bps: None
            })
        );
        assert_eq!(parse_progress("dwv:junk 1 2"), None);
        assert_eq!(parse_progress("[download] 12.3%"), None);
    }

    #[test]
    fn parses_destination_lines() {
        assert_eq!(
            parse_destination("[download] Destination: C:\\videos\\clip.mp4"),
            Some(PathBuf::from("C:\\videos\\clip.mp4"))
        );
        assert_eq!(
            parse_destination("[Merger] Merging formats into \"C:\\videos\\clip.mp4\""),
            Some(PathBuf::from("C:\\videos\\clip.mp4"))
        );
        assert_eq!(parse_destination("[download] 12.3% of 10MiB"), None);
    }
}
