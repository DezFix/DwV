use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use librqbit::api::TorrentIdOrHash;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, DhtSessionConfig, Session, SessionOptions, TorrentStatsState,
};

type ManagedTorrentHandle = Arc<librqbit::ManagedTorrent>;

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
pub enum TorrentState {
    Pending,
    Downloading,
    Paused,
    Completed,
    Error,
}

/// Events the app layer turns into `download-*` Tauri events.
#[derive(Debug, Clone)]
pub enum TorrentEvent {
    Progress {
        id: i64,
        downloaded: u64,
        total: u64,
        speed: u64,
        uploaded: u64,
        connections: usize,
        eta: u64,
    },
    Status {
        id: i64,
        state: TorrentState,
        total: u64,
        error: Option<String>,
    },
    Complete {
        id: i64,
        path: PathBuf,
        total: u64,
    },
    Error {
        id: i64,
        error: String,
    },
}

#[derive(Debug, Clone)]
pub struct TorrentPeer {
    pub address: String,
    pub state: String,
    pub client: Option<String>,
    pub downloaded: u64,
    pub uploaded: u64,
}

#[derive(Debug, Clone)]
pub struct TorrentDetails {
    pub info_hash: String,
    pub downloaded: u64,
    pub uploaded: u64,
    pub peers_live: u32,
    pub peers_seen: u32,
    pub peers: Vec<TorrentPeer>,
}

#[derive(Debug, Clone, Copy)]
pub struct TorrentSessionConfig {
    pub dht_enabled: bool,
    pub peer_limit: Option<usize>,
    pub download_bps: Option<NonZeroU32>,
    pub upload_bps: Option<NonZeroU32>,
}

type EventSink = Arc<dyn Fn(TorrentEvent) + Send + Sync>;

struct Inner {
    default_dir: PathBuf,
    config: Mutex<TorrentSessionConfig>,
    session: tokio::sync::OnceCell<Arc<Session>>,
    torrents: Mutex<HashMap<i64, ManagedTorrentHandle>>,
    tasks: Mutex<HashMap<i64, tokio::task::JoinHandle<()>>>,
}

/// Owns the single librqbit session for the whole app lifetime.
#[derive(Clone)]
pub struct TorrentManager {
    inner: Arc<Inner>,
}

impl TorrentManager {
    pub fn new(default_dir: PathBuf, config: TorrentSessionConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                default_dir,
                config: Mutex::new(config),
                session: tokio::sync::OnceCell::new(),
                torrents: Mutex::new(HashMap::new()),
                tasks: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Peer limit and DHT only affect a session that has not been created yet.
    pub fn configure(&self, config: TorrentSessionConfig) {
        *self.inner.config.lock().unwrap() = config;
        self.apply_session_limits(config.download_bps, config.upload_bps);
    }

    pub fn set_limits(&self, download_bps: Option<NonZeroU32>, upload_bps: Option<NonZeroU32>) {
        {
            let mut config = self.inner.config.lock().unwrap();
            config.download_bps = download_bps;
            config.upload_bps = upload_bps;
        }
        self.apply_session_limits(download_bps, upload_bps);
    }

    fn apply_session_limits(
        &self,
        download_bps: Option<NonZeroU32>,
        upload_bps: Option<NonZeroU32>,
    ) {
        if let Some(session) = self.inner.session.get() {
            session.ratelimits.set_download_bps(download_bps);
            session.ratelimits.set_upload_bps(upload_bps);
        }
    }

    async fn session(&self) -> Result<Arc<Session>, Error> {
        let config = *self.inner.config.lock().unwrap();
        let default_dir = self.inner.default_dir.clone();
        let session = self
            .inner
            .session
            .get_or_try_init(|| async move {
                let options = SessionOptions {
                    dht: config.dht_enabled.then(DhtSessionConfig::default),
                    peer_limit: config.peer_limit,
                    ratelimits: LimitsConfig {
                        download_bps: config.download_bps,
                        upload_bps: config.upload_bps,
                    },
                    client_name_and_version: Some(format!("DwV/{}", env!("CARGO_PKG_VERSION"))),
                    ..Default::default()
                };
                Session::new_with_opts(default_dir, options)
                    .await
                    .map_err(|e| Error::Torrent(e.to_string()))
            })
            .await?;
        Ok(session.clone())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add(
        &self,
        id: i64,
        source: &str,
        dir: &Path,
        paused: bool,
        seed_after_complete: bool,
        fallback_name: String,
        on_event: EventSink,
    ) -> Result<(), Error> {
        let session = self.session().await?;
        let add = if Path::new(source).is_file() {
            AddTorrent::from_local_filename(source).map_err(|e| Error::Torrent(e.to_string()))?
        } else {
            AddTorrent::from_url(source.to_string())
        };
        let options = AddTorrentOptions {
            paused,
            output_folder: Some(dir.to_string_lossy().into_owned()),
            overwrite: true,
            ..Default::default()
        };
        let handle = session
            .add_torrent(add, Some(options))
            .await
            .map_err(|e| Error::Torrent(e.to_string()))?
            .into_handle()
            .ok_or(Error::NoHandle)?;

        self.inner
            .torrents
            .lock()
            .unwrap()
            .insert(id, handle.clone());
        let task = tokio::spawn(poll(
            id,
            handle,
            session,
            paused,
            seed_after_complete,
            fallback_name,
            on_event,
        ));
        self.inner.tasks.lock().unwrap().insert(id, task);
        Ok(())
    }

    pub async fn pause(&self, id: i64) -> Result<(), Error> {
        let Some(handle) = self.handle(id) else {
            return Ok(());
        };
        if handle.is_paused() {
            return Ok(());
        }
        let session = self.session().await?;
        session
            .pause(&handle)
            .await
            .map_err(|e| Error::Torrent(e.to_string()))
    }

    pub async fn resume(&self, id: i64) -> Result<(), Error> {
        let Some(handle) = self.handle(id) else {
            return Ok(());
        };
        if !handle.is_paused() {
            return Ok(());
        }
        let session = self.session().await?;
        session
            .unpause(&handle)
            .await
            .map_err(|e| Error::Torrent(e.to_string()))
    }

    pub async fn remove(&self, id: i64, delete_files: bool) -> Result<(), Error> {
        let Some(handle) = self.inner.torrents.lock().unwrap().remove(&id) else {
            return Ok(());
        };
        if let Some(task) = self.inner.tasks.lock().unwrap().remove(&id) {
            task.abort();
        }
        let session = self.session().await?;
        session
            .delete(TorrentIdOrHash::Id(handle.id()), delete_files)
            .await
            .map_err(|e| Error::Torrent(e.to_string()))
    }

    pub fn ids(&self) -> Vec<i64> {
        self.inner
            .torrents
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect()
    }

    pub fn details(&self, id: i64) -> Option<TorrentDetails> {
        let handle = self.handle(id)?;
        let stats = handle.stats();
        let live_stats = stats.live.as_ref();
        let (peers_live, peers_seen) = live_stats
            .map(|live| (live.snapshot.peer_stats.live, live.snapshot.peer_stats.seen))
            .unwrap_or((0, 0));
        let peers = handle
            .live()
            .map(|live| {
                let snapshot = live.per_peer_stats_snapshot(Default::default());
                let mut peers: Vec<TorrentPeer> = snapshot
                    .peers
                    .into_iter()
                    .map(|(address, stats)| TorrentPeer {
                        address,
                        state: stats.state.to_string(),
                        client: stats.client_name.clone(),
                        downloaded: stats.counters.fetched_bytes,
                        uploaded: stats.counters.uploaded_bytes,
                    })
                    .collect();
                peers.sort_by(|a, b| a.address.cmp(&b.address));
                peers
            })
            .unwrap_or_default();
        Some(TorrentDetails {
            info_hash: handle.info_hash().as_string(),
            downloaded: stats.progress_bytes,
            uploaded: stats.uploaded_bytes,
            peers_live,
            peers_seen,
            peers,
        })
    }

    fn handle(&self, id: i64) -> Option<ManagedTorrentHandle> {
        self.inner.torrents.lock().unwrap().get(&id).cloned()
    }
}

#[allow(clippy::too_many_arguments)]
async fn poll(
    id: i64,
    handle: ManagedTorrentHandle,
    session: Arc<Session>,
    started_paused: bool,
    seed_after_complete: bool,
    fallback_name: String,
    on_event: EventSink,
) {
    let mut last_state = started_paused.then_some(TorrentState::Paused);
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;
        let stats = handle.stats();

        if let Some(error) = stats.error.clone() {
            on_event(TorrentEvent::Error { id, error });
            return;
        }

        if stats.finished {
            if !seed_after_complete {
                let _ = session.pause(&handle).await;
            }
            let name = handle.name().unwrap_or(fallback_name);
            on_event(TorrentEvent::Complete {
                id,
                path: handle.output_folder().join(name),
                total: stats.total_bytes,
            });
            return;
        }

        let state = classify(&stats);
        if last_state != Some(state) {
            on_event(TorrentEvent::Status {
                id,
                state,
                total: stats.total_bytes,
                error: None,
            });
            last_state = Some(state);
        }

        if state == TorrentState::Downloading {
            let live = stats.live.as_ref();
            let speed = live.map(|l| l.download_speed.as_bytes()).unwrap_or(0);
            let connections = live
                .map(|l| l.snapshot.peer_stats.live as usize)
                .unwrap_or(0);
            let eta = stats
                .total_bytes
                .saturating_sub(stats.progress_bytes)
                .checked_div(speed)
                .unwrap_or(0);
            on_event(TorrentEvent::Progress {
                id,
                downloaded: stats.progress_bytes,
                total: stats.total_bytes,
                speed,
                uploaded: stats.uploaded_bytes,
                connections,
                eta,
            });
        }
    }
}

fn classify(stats: &librqbit::TorrentStats) -> TorrentState {
    if stats.finished {
        return TorrentState::Completed;
    }
    if stats.error.is_some() {
        return TorrentState::Error;
    }
    match stats.state {
        TorrentStatsState::Initializing { paused } => {
            if paused {
                TorrentState::Paused
            } else {
                TorrentState::Pending
            }
        }
        TorrentStatsState::Live => TorrentState::Downloading,
        TorrentStatsState::Paused => TorrentState::Paused,
        TorrentStatsState::Error => TorrentState::Error,
    }
}

/// Magnet `dn`, else the `.torrent` file/URL stem, else `torrent`.
/// The result is safe to use as a single path segment.
pub fn torrent_name(source: &str) -> String {
    let source = source.trim();
    if let Ok(magnet) = librqbit::Magnet::parse(source) {
        return sanitize_name(
            &magnet
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "torrent".to_string()),
        );
    }
    if source.starts_with("magnet:") {
        return "torrent".to_string();
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        let path = source.split(['?', '#']).next().unwrap_or(source);
        let name = path.rsplit('/').next().unwrap_or("").trim();
        return sanitize_name(&clean_torrent_name(&percent_decode(name)));
    }
    let name = Path::new(source)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    sanitize_name(&clean_torrent_name(&name))
}

fn clean_torrent_name(name: &str) -> String {
    let name = name.trim();
    let lower = name.to_ascii_lowercase();
    let stem = if lower.ends_with(".torrent") {
        &name[..name.len() - ".torrent".len()]
    } else {
        name
    };
    if stem.is_empty() {
        "torrent".to_string()
    } else {
        stem.to_string()
    }
}

/// Keeps a magnet display name usable as one file-system path segment.
fn sanitize_name(name: &str) -> String {
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
    let cleaned: String = cleaned.chars().take(180).collect();
    if cleaned.is_empty() {
        "torrent".to_string()
    } else {
        cleaned
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TorrentProgress {
    pub downloaded: u64,
    pub total: u64,
    pub down_bps: u64,
}

/// One-shot download used by the CLI. The app uses [`TorrentManager`] instead.
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

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "a621779b5e3d486e127c3efbca9b6f8d135f52e5";

    #[test]
    fn magnet_name_is_percent_decoded() {
        let source = format!("magnet:?xt=urn:btih:{HASH}&dn=My+File%20Name");
        assert_eq!(torrent_name(&source), "My File Name");
    }

    #[test]
    fn magnet_without_name_falls_back_to_torrent() {
        assert_eq!(
            torrent_name(&format!("magnet:?xt=urn:btih:{HASH}")),
            "torrent"
        );
    }

    #[test]
    fn torrent_file_and_url_names_lose_the_extension() {
        assert_eq!(torrent_name("C:\\Downloads\\ubuntu.torrent"), "ubuntu");
        assert_eq!(torrent_name("https://example.com/a%20b.torrent?x=1"), "a b");
    }

    #[test]
    fn magnet_names_cannot_escape_the_download_folder() {
        let source = format!("magnet:?xt=urn:btih:{HASH}&dn=..%2F..%2Fevil");
        let name = torrent_name(&source);
        assert!(!name.contains('/'), "{name}");
        assert!(!name.contains('\\'), "{name}");
        assert_ne!(name, "..");
    }
}
