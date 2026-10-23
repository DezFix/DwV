use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.21/yt-dlp.exe";
const YTDLP_SHA256: &str = "33121d192d706e62f684a06f80918e3f3a8ba52ac3a65213b6951c39c7d9f555";
const FFMPEG_URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-21-13-06/ffmpeg-n9.0.2-31-gc89d6ee1ca-win64-lgpl-9.0.zip";
const FFMPEG_SHA256: &str = "1840634b3f2ce1512d9208f1ea96084bed42cf4b80a94546ff1a8757a594e389";
const DENO_URL: &str =
    "https://github.com/denoland/deno/releases/download/v2.9.8/deno-x86_64-pc-windows-msvc.zip";
const DENO_SHA256: &str = "cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf";
const PROGRESS_PREFIX: &str = "dwv:";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("APPDATA is not set")]
    NoAppData,
    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    Checksum {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("yt-dlp failed ({code:?}): {stderr}")]
    Process { code: Option<i32>, stderr: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed_bps: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoQuality {
    Best,
    P1080,
    P720,
    P480,
}

impl VideoQuality {
    // Prefer H.264 + AAC in MP4: the best streams (AV1/VP9 + Opus) land in
    // .webm/.mkv files that Windows players often refuse to open.
    pub fn ytdlp_format(self) -> &'static str {
        match self {
            Self::Best => {
                "bv*[vcodec^=avc1][ext=mp4]+ba[acodec^=mp4a][ext=m4a]/bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b"
            }
            Self::P1080 => {
                "bv*[height<=1080][vcodec^=avc1][ext=mp4]+ba[acodec^=mp4a][ext=m4a]/bv*[height<=1080][ext=mp4]+ba[ext=m4a]/b[height<=1080][ext=mp4]/bv*[height<=1080]+ba/b[height<=1080]"
            }
            Self::P720 => {
                "bv*[height<=720][vcodec^=avc1][ext=mp4]+ba[acodec^=mp4a][ext=m4a]/bv*[height<=720][ext=mp4]+ba[ext=m4a]/b[height<=720][ext=mp4]/bv*[height<=720]+ba/b[height<=720]"
            }
            Self::P480 => {
                "bv*[height<=480][vcodec^=avc1][ext=mp4]+ba[acodec^=mp4a][ext=m4a]/bv*[height<=480][ext=mp4]+ba[ext=m4a]/b[height<=480][ext=mp4]/bv*[height<=480]+ba/b[height<=480]"
            }
        }
    }
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
    let stamp = path.with_file_name("yt-dlp.stamp");
    if path.is_file() {
        // yt-dlp can update itself. Check at most once a day so a video
        // download is not delayed by a network round trip every time.
        let stale = tokio::fs::metadata(&stamp)
            .await
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.elapsed().ok())
            .map(|age| age > Duration::from_secs(24 * 60 * 60))
            .unwrap_or(true);
        if stale {
            let mut command = Command::new(&path);
            command.arg("-U");
            hide_console(&mut command);
            let _ = command.output().await;
            let _ = tokio::fs::write(&stamp, b"").await;
        }
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("exe.part");
    download_to(YTDLP_URL, &tmp, YTDLP_SHA256).await?;
    tokio::fs::rename(&tmp, &path).await?;
    let _ = tokio::fs::write(&stamp, b"").await;
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
    download_to(FFMPEG_URL, &zip_path, FFMPEG_SHA256).await?;
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

/// yt-dlp needs a JS runtime (deno) for YouTube; without one the format list is limited.
/// The caller may ignore failures and download anyway.
pub async fn ensure_deno() -> Result<PathBuf, Error> {
    let path = bin_dir()?.join("deno.exe");
    if path.is_file() {
        return Ok(path);
    }
    let bin = bin_dir()?;
    tokio::fs::create_dir_all(&bin).await?;
    let zip_path = bin.join("deno.zip.part");
    download_to(DENO_URL, &zip_path, DENO_SHA256).await?;
    let mut command = Command::new(tar_exe());
    command.arg("-xf").arg(&zip_path).arg("-C").arg(&bin);
    hide_console(&mut command);
    let status = command.status().await?;
    let _ = tokio::fs::remove_file(&zip_path).await;
    if !status.success() || !path.is_file() {
        return Err(Error::Io(std::io::Error::other(
            "deno.exe not found after extraction",
        )));
    }
    Ok(path)
}

async fn download_to(url: &str, path: &Path, expected: &str) -> Result<(), Error> {
    let response = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let mut file = tokio::fs::File::create(path).await?;
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = tokio::fs::remove_file(path).await;
        return Err(Error::Checksum {
            path: path.display().to_string(),
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(())
}

// shortcut: uses the built-in tar.exe of Windows 10+ to read the zip
async fn extract_bin_files(zip_path: &Path, bin: &Path) -> Result<(), Error> {
    let mut command = Command::new(tar_exe());
    command.arg("-xf").arg(zip_path).arg("-C").arg(bin);
    hide_console(&mut command);
    let status = command.status().await?;
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

#[cfg(windows)]
fn tar_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join("tar.exe")
}

#[cfg(not(windows))]
fn tar_exe() -> PathBuf {
    PathBuf::from("tar")
}

fn path_with_bin(bin: &Path) -> std::ffi::OsString {
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&existing));
    std::env::join_paths(dirs).unwrap_or_else(|_| bin.as_os_str().to_owned())
}

/// Downloads a single video. `on_progress` is called from the reading loop, keep it quick.
pub async fn download_video(
    url: &str,
    dir: &Path,
    quality: VideoQuality,
    on_progress: impl FnMut(VideoProgress),
) -> Result<Option<PathBuf>, Error> {
    download_video_with_limit(url, dir, quality, None, on_progress).await
}

/// Same as [`download_video`], but caps the transfer rate in bytes per second.
pub async fn download_video_with_limit(
    url: &str,
    dir: &Path,
    quality: VideoQuality,
    limit_bps: Option<u64>,
    mut on_progress: impl FnMut(VideoProgress),
) -> Result<Option<PathBuf>, Error> {
    let ytdlp = ensure_ytdlp().await?;
    let ffmpeg_dir = ensure_ffmpeg().await?;
    let _ = ensure_deno().await;
    static PRINT_SEQ: AtomicU64 = AtomicU64::new(0);
    let print_path = std::env::temp_dir().join(format!(
        "dwv-ytdlp-{}-{}.txt",
        std::process::id(),
        PRINT_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let print_guard = TempFile(print_path.clone());
    let mut command = Command::new(&ytdlp);
    command
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        // yt-dlp finds deno/ffmpeg on PATH; keep the bin dir first
        .env("PATH", path_with_bin(&ffmpeg_dir))
        .arg("--newline")
        .arg("--no-playlist")
        .arg("--no-overwrites")
        .arg("--windows-filenames")
        .arg("--ffmpeg-location")
        .arg(&ffmpeg_dir)
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--trim-filenames")
        .arg("150")
        .arg("--progress-template")
        .arg(format!(
            "{PROGRESS_PREFIX}%(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(progress.speed)s"
        ))
        // stdout can be console-encoded; the file is always UTF-8.
        .arg("--print-to-file")
        .arg("after_move:%(filepath)s")
        .arg(&print_path);
    if let Some(limit) = limit_bps.filter(|value| *value > 0) {
        command.arg("--limit-rate").arg(limit.to_string());
    }
    let started = SystemTime::now();
    let before = snapshot_files(dir);
    hide_console(&mut command);
    let mut child = command
        .arg("-f")
        .arg(quality.ytdlp_format())
        .arg("-P")
        .arg(dir)
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let stderr_task = tokio::spawn(collect_stderr(stderr));

    let mut last_path = None;
    let mut progress = |line: String| {
        if let Some(update) = parse_progress(&line) {
            on_progress(update);
        } else if let Some(path) = parse_destination(&line) {
            last_path = Some(path);
        }
    };
    for_each_line(BufReader::new(stdout), &mut progress).await?;

    let status = child.wait().await?;
    let stderr_lines = stderr_task.await.unwrap_or_default();
    let printed = std::fs::read_to_string(&print_path)
        .ok()
        .map(|text| text.trim_start_matches('\u{feff}').trim().to_string())
        .filter(|text| !text.is_empty())
        .map(PathBuf::from);
    drop(print_guard);
    if !status.success() {
        return Err(Error::Process {
            code: status.code(),
            stderr: stderr_lines.join("\n"),
        });
    }
    Ok(pick_final_path(printed, last_path, dir, started, &before))
}

struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Prefers the UTF-8 path printed by yt-dlp, then its stdout, then the newest
/// media file that appeared in the output folder after the download started.
fn pick_final_path(
    printed: Option<PathBuf>,
    stdout_path: Option<PathBuf>,
    dir: &Path,
    started: SystemTime,
    before: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = printed {
        let path = if path.is_absolute() {
            path
        } else {
            dir.join(path)
        };
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(path) = stdout_path.filter(|path| path.is_file()) {
        return Some(path);
    }
    newest_output(dir, started, before)
}

fn snapshot_files(dir: &Path) -> HashSet<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn newest_output(dir: &Path, started: SystemTime, before: &HashSet<PathBuf>) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || is_temporary(&path) {
            continue;
        }
        let Some(extension) = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        else {
            continue;
        };
        if !matches!(
            extension.as_str(),
            "mp4" | "mkv" | "webm" | "m4v" | "mov" | "avi"
        ) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if modified < started && before.contains(&path) {
            continue;
        }
        if newest.as_ref().is_none_or(|(time, _)| modified >= *time) {
            newest = Some((modified, path));
        }
    }
    newest.map(|(_, path)| path)
}

fn is_temporary(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.starts_with('.')
        || name.ends_with(".part")
        || name.ends_with(".ytdl")
        || name.ends_with(".temp")
}

async fn for_each_line<R: tokio::io::AsyncRead + Unpin>(
    mut reader: BufReader<R>,
    on_line: &mut impl FnMut(String),
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).await? == 0 {
            return Ok(());
        }
        let line = String::from_utf8_lossy(&buf);
        on_line(line.trim_end_matches(['\r', '\n']).to_string());
    }
}

async fn collect_stderr(stderr: tokio::process::ChildStderr) -> Vec<String> {
    let mut tail = Vec::new();
    let mut push = |line: String| {
        if tail.len() == 20 {
            tail.remove(0);
        }
        tail.push(line);
    };
    let _ = for_each_line(BufReader::new(stderr), &mut push).await;
    tail
}

fn parse_progress(line: &str) -> Option<VideoProgress> {
    let rest = line.trim().strip_prefix(PROGRESS_PREFIX)?;
    let mut parts = rest.split_whitespace();
    let downloaded = parts.next()?.parse().ok()?;
    let total_bytes = parts.next()?;
    let total_estimate = parts.next()?;
    let total = total_bytes
        .parse()
        .ok()
        .or_else(|| total_estimate.parse().ok());
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
    let rest = line
        .strip_prefix("[download] ")?
        .strip_suffix(" has already been downloaded")?
        .trim()
        .trim_matches('"')
        .trim();
    (!rest.is_empty()).then(|| PathBuf::from(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_quality_formats_prefer_playable_mp4() {
        for format in [
            VideoQuality::Best,
            VideoQuality::P1080,
            VideoQuality::P720,
            VideoQuality::P480,
        ] {
            let selector = format.ytdlp_format();
            assert!(
                selector.starts_with("bv*[") && selector.contains("avc1"),
                "{selector}"
            );
            assert!(selector.contains("acodec^=mp4a"), "{selector}");
            assert!(
                selector.ends_with("/bv*+ba/b") || selector.contains("+ba/b["),
                "{selector}"
            );
        }
        assert!(VideoQuality::P720.ytdlp_format().contains("height<=720"));
    }

    #[test]
    fn parses_progress_lines() {
        assert_eq!(
            parse_progress("dwv:1024 4096 NA 512"),
            Some(VideoProgress {
                downloaded: 1024,
                total: Some(4096),
                speed_bps: Some(512)
            })
        );
        assert_eq!(
            parse_progress("dwv:1024 NA 4096 512"),
            Some(VideoProgress {
                downloaded: 1024,
                total: Some(4096),
                speed_bps: Some(512)
            })
        );
        assert_eq!(
            parse_progress("dwv:1024 NA NA NA"),
            Some(VideoProgress {
                downloaded: 1024,
                total: None,
                speed_bps: None
            })
        );
        assert_eq!(parse_progress("dwv:junk 1 2 3"), None);
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
        assert_eq!(
            parse_destination("[download] C:\\videos\\clip.mp4 has already been downloaded"),
            Some(PathBuf::from("C:\\videos\\clip.mp4"))
        );
        assert_eq!(parse_destination("[download] 12.3% of 10MiB"), None);
    }

    #[test]
    fn prefers_printed_path_then_newest_output_file() {
        let dir = std::env::temp_dir().join(format!(
            "dwv-media-path-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let old = dir.join("old.mp4");
        std::fs::write(&old, b"old").unwrap();
        let before = snapshot_files(&dir);
        let started = SystemTime::now();
        std::thread::sleep(Duration::from_millis(50));
        let newest = dir.join("new.mp4");
        std::fs::write(&newest, b"new").unwrap();
        std::fs::write(dir.join("new.mp4.part"), b"partial").unwrap();
        std::fs::write(dir.join("notes.txt"), b"text").unwrap();

        let printed = dir.join("printed.mp4");
        std::fs::write(&printed, b"printed").unwrap();
        assert_eq!(
            pick_final_path(Some(printed.clone()), None, &dir, started, &before),
            Some(printed.clone())
        );

        // A stale printed path must fall back to the real file on disk.
        std::fs::remove_file(&printed).unwrap();
        let missing = PathBuf::from(r"C:\video\missing.mkv");
        assert_eq!(
            pick_final_path(Some(missing), None, &dir, started, &before),
            Some(newest.clone())
        );
        assert_eq!(
            pick_final_path(None, None, &dir, started, &before),
            Some(newest)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn decodes_invalid_utf8_lines_lossily() {
        let mut lines = Vec::new();
        let mut push = |line: String| lines.push(line);
        for_each_line(BufReader::new(&b"a\xffb\nc\r\n"[..]), &mut push)
            .await
            .unwrap();
        assert_eq!(lines, vec!["a\u{FFFD}b".to_string(), "c".to_string()]);
    }
}
