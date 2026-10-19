use std::path::{Path, PathBuf};
use std::time::Duration;

use librqbit::{AddTorrent, Session};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("torrent error: {0}")]
    Torrent(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("torrent produced no download handle")]
    NoHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TorrentProgress {
    pub downloaded: u64,
    pub total: u64,
    pub down_bps: u64,
}

/// Downloads a torrent (magnet link, http(s) .torrent url or local .torrent file)
/// into `dir` and returns the path of the downloaded file or folder.
/// `on_progress` is called from the polling loop, keep it quick.
pub async fn download_torrent(
    source: &str,
    dir: &Path,
    mut on_progress: impl FnMut(TorrentProgress),
) -> Result<PathBuf, Error> {
    tokio::fs::create_dir_all(dir).await?;
    let session = Session::new(dir.to_path_buf())
        .await
        .map_err(|e| Error::Torrent(e.to_string()))?;
    let add = if Path::new(source).is_file() {
        AddTorrent::from_local_filename(source).map_err(|e| Error::Torrent(e.to_string()))?
    } else {
        AddTorrent::from_url(source)
    };
    // shortcut: no peer/upload/seed limits yet, add when they matter
    let handle = session
        .add_torrent(add, None)
        .await
        .map_err(|e| Error::Torrent(e.to_string()))?
        .into_handle()
        .ok_or(Error::NoHandle)?;

    let mut last_bytes = 0u64;
    let mut last_tick = tokio::time::Instant::now();
    let mut last_emit = tokio::time::Instant::now() - Duration::from_secs(1);
    let stats = loop {
        let stats = handle.stats();
        if let Some(error) = stats.error {
            return Err(Error::Torrent(error));
        }
        if last_emit.elapsed() >= Duration::from_millis(500) {
            let now = tokio::time::Instant::now();
            let seconds = now.duration_since(last_tick).as_secs_f64().max(0.0001);
            let down_bps =
                (stats.progress_bytes.saturating_sub(last_bytes) as f64 / seconds) as u64;
            last_bytes = stats.progress_bytes;
            last_tick = now;
            last_emit = now;
            on_progress(TorrentProgress {
                downloaded: stats.progress_bytes,
                total: stats.total_bytes,
                down_bps,
            });
        }
        if stats.finished {
            break stats;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    on_progress(TorrentProgress {
        downloaded: stats.progress_bytes,
        total: stats.total_bytes,
        down_bps: 0,
    });
    Ok(dir.join(handle.name().unwrap_or_default()))
}
