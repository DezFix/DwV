use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{
    CONTENT_LENGTH, CONTENT_RANGE, ETAG, HeaderMap, HeaderName, HeaderValue, LAST_MODIFIED, RANGE,
};
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use crate::error::Error;
use crate::limiter::SpeedLimiter;

const TICK: Duration = Duration::from_millis(250);
const MAX_RETRIES: u32 = 5;
const WRITE_SLICE: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct DownloadSpec {
    pub url: String,
    pub target: PathBuf,
    pub connections: usize,
    pub headers: Vec<(String, String)>,
    pub limiter: Option<SpeedLimiter>,
}

impl DownloadSpec {
    pub fn new(url: impl Into<String>, target: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            target: target.into(),
            connections: 8,
            headers: Vec::new(),
            limiter: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Started {
        total: Option<u64>,
        connections: usize,
        resumed: bool,
    },
    Progress {
        downloaded: u64,
        total: Option<u64>,
        speed_bps: u64,
    },
    Finished {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Cancelled,
}

#[derive(Clone)]
pub struct DownloadController {
    cancel: watch::Sender<bool>,
}

impl DownloadController {
    pub fn cancel(&self) {
        let _ = self.cancel.send(true);
    }
}

pub struct Download {
    events: mpsc::Receiver<Event>,
    controller: DownloadController,
    task: tokio::task::JoinHandle<Result<Outcome, Error>>,
}

impl Download {
    pub fn start(spec: DownloadSpec) -> Self {
        let (tx, events) = mpsc::channel(64);
        let (cancel, cancel_rx) = watch::channel(false);
        let task = tokio::spawn(run(spec, tx, cancel_rx));
        Self {
            events,
            controller: DownloadController { cancel },
            task,
        }
    }

    pub fn events(&mut self) -> &mut mpsc::Receiver<Event> {
        &mut self.events
    }

    pub fn controller(&self) -> DownloadController {
        self.controller.clone()
    }

    pub async fn join(self) -> Result<Outcome, Error> {
        self.task
            .await
            .map_err(|e| Error::Internal(e.to_string()))?
    }

    pub fn into_parts(
        self,
    ) -> (
        mpsc::Receiver<Event>,
        DownloadController,
        tokio::task::JoinHandle<Result<Outcome, Error>>,
    ) {
        (self.events, self.controller, self.task)
    }
}

#[derive(Serialize, Deserialize)]
struct Meta {
    url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    total: u64,
    connections: usize,
    done: Vec<u64>,
}

struct Probe {
    total: Option<u64>,
    segmented: bool,
    etag: Option<String>,
    last_modified: Option<String>,
}

struct SegCtx {
    client: reqwest::Client,
    url: String,
    part: PathBuf,
    headers: HeaderMap,
    ranged: bool,
    limiter: Option<SpeedLimiter>,
}

async fn run(
    spec: DownloadSpec,
    tx: mpsc::Sender<Event>,
    cancel: watch::Receiver<bool>,
) -> Result<Outcome, Error> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("DwV/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let headers = build_headers(&spec.headers)?;
    let info = probe(&client, &spec, &headers).await?;
    let connections = if info.segmented {
        spec.connections.max(1)
    } else {
        1
    };

    if let Some(parent) = spec.target.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).await?;
    }

    let part = with_suffix(&spec.target, ".part");
    let meta_path = with_suffix(&spec.target, ".dwvmeta");

    let resumed = match read_meta(&meta_path).await {
        Some(meta)
            if info.segmented
                && meta.url == spec.url
                && meta.etag == info.etag
                && meta.last_modified == info.last_modified
                && Some(meta.total) == info.total
                && meta.connections == connections
                && meta.done.len() == connections
                && fs::metadata(&part)
                    .await
                    .map(|m| m.is_file())
                    .unwrap_or(false) =>
        {
            Some(meta)
        }
        _ => None,
    };

    let (done, resumed_flag) = match &resumed {
        Some(meta) => (
            meta.done
                .iter()
                .map(|d| Arc::new(AtomicU64::new(*d)))
                .collect::<Vec<_>>(),
            true,
        ),
        None => {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&part)
                .await?;
            file.set_len(info.total.unwrap_or(0)).await?;
            (
                (0..connections)
                    .map(|_| Arc::new(AtomicU64::new(0)))
                    .collect(),
                false,
            )
        }
    };

    let total = info.total;
    let mut segments: Vec<(u64, u64)> = Vec::with_capacity(connections);
    if info.segmented {
        // segmented implies a known total
        let size = total.unwrap_or(0);
        let chunk = size.div_ceil(connections as u64);
        for i in 0..connections as u64 {
            let start = i * chunk;
            if start >= size {
                break;
            }
            segments.push((start, (start + chunk).min(size) - 1));
        }
    } else {
        segments.push((0, total.map(|t| t.saturating_sub(1)).unwrap_or(u64::MAX)));
    }

    if segments.is_empty() {
        finalize(&part, &spec.target, &tx).await;
        return Ok(Outcome::Completed);
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let mut ext_cancel = cancel;
    let bridge = stop_tx.clone();
    tokio::spawn(async move {
        loop {
            if *ext_cancel.borrow() {
                let _ = bridge.send(true);
                return;
            }
            if ext_cancel.changed().await.is_err() {
                return;
            }
        }
    });

    let ctx = Arc::new(SegCtx {
        client: client.clone(),
        url: spec.url.clone(),
        part: part.clone(),
        headers,
        ranged: info.segmented,
        limiter: spec.limiter.clone(),
    });
    let seg_count = segments.len();
    let mut tasks = JoinSet::new();
    for (i, (start, end)) in segments.iter().enumerate() {
        let expected = if info.segmented { None } else { total };
        tasks.spawn(run_segment(
            ctx.clone(),
            *start,
            *end,
            expected,
            done[i].clone(),
            stop_rx.clone(),
        ));
    }

    let started = tx
        .send(Event::Started {
            total,
            connections: seg_count,
            resumed: resumed_flag,
        })
        .await;
    if started.is_err() {
        let _ = stop_tx.send(true);
        while tasks.join_next().await.is_some() {}
        return Ok(Outcome::Cancelled);
    }

    let build_meta = || Meta {
        url: spec.url.clone(),
        etag: info.etag.clone(),
        last_modified: info.last_modified.clone(),
        total: total.unwrap_or(0),
        connections: seg_count,
        done: done.iter().map(|d| d.load(Ordering::Relaxed)).collect(),
    };

    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_bytes = sum_done(&done);
    let mut last_tick = Instant::now();
    let mut result: Result<Outcome, Error> = Ok(Outcome::Completed);

    while !tasks.is_empty() {
        tokio::select! {
            joined = tasks.join_next() => match joined {
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(Error::Cancelled))) => {
                    result = Ok(Outcome::Cancelled);
                    let _ = stop_tx.send(true);
                    break;
                }
                Some(Ok(Err(e))) => {
                    result = Err(e);
                    let _ = stop_tx.send(true);
                    break;
                }
                Some(Err(e)) => {
                    result = Err(Error::Internal(e.to_string()));
                    let _ = stop_tx.send(true);
                    break;
                }
                None => break,
            },
            _ = interval.tick() => {
                let now = Instant::now();
                let downloaded = sum_done(&done);
                let dt = now.duration_since(last_tick).as_secs_f64().max(0.001);
                let speed = ((downloaded.saturating_sub(last_bytes)) as f64 / dt) as u64;
                last_bytes = downloaded;
                last_tick = now;
                // single-stream downloads are not resumable, so no metadata for them
                if info.segmented {
                    let _ = persist_meta(&meta_path, &build_meta()).await;
                }
                let sent = tx
                    .send(Event::Progress {
                        downloaded,
                        total,
                        speed_bps: speed,
                    })
                    .await;
                if sent.is_err() {
                    result = Ok(Outcome::Cancelled);
                    let _ = stop_tx.send(true);
                    break;
                }
            }
        }
    }
    while tasks.join_next().await.is_some() {}

    match result {
        Ok(Outcome::Completed) => {
            finalize(&part, &spec.target, &tx).await;
            let _ = fs::remove_file(&meta_path).await;
            Ok(Outcome::Completed)
        }
        Ok(Outcome::Cancelled) => {
            if info.segmented {
                let _ = persist_meta(&meta_path, &build_meta()).await;
            }
            Ok(Outcome::Cancelled)
        }
        Err(e) => {
            if info.segmented {
                let _ = persist_meta(&meta_path, &build_meta()).await;
            }
            Err(e)
        }
    }
}

async fn finalize(part: &Path, target: &Path, tx: &mpsc::Sender<Event>) {
    if fs::metadata(target).await.is_ok() {
        let _ = fs::remove_file(target).await;
    }
    if fs::rename(part, target).await.is_ok() {
        let _ = tx
            .send(Event::Finished {
                path: target.to_path_buf(),
            })
            .await;
    }
}

async fn probe(
    client: &reqwest::Client,
    spec: &DownloadSpec,
    headers: &HeaderMap,
) -> Result<Probe, Error> {
    let resp = client
        .get(&spec.url)
        .headers(headers.clone())
        .header(RANGE, "bytes=0-0")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(Error::Status(status.as_u16(), spec.url.clone()));
    }
    let etag = header_str(resp.headers(), ETAG);
    let last_modified = header_str(resp.headers(), LAST_MODIFIED);
    if status == StatusCode::PARTIAL_CONTENT {
        let total =
            header_str(resp.headers(), CONTENT_RANGE).and_then(|v| parse_content_range_total(&v));
        return Ok(Probe {
            total,
            segmented: total.is_some(),
            etag,
            last_modified,
        });
    }
    let total = header_str(resp.headers(), CONTENT_LENGTH).and_then(|v| v.parse().ok());
    Ok(Probe {
        total,
        segmented: false,
        etag,
        last_modified,
    })
}

async fn run_segment(
    ctx: Arc<SegCtx>,
    start: u64,
    end: u64,
    expected: Option<u64>,
    done: Arc<AtomicU64>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let mut file = OpenOptions::new().write(true).open(&ctx.part).await?;
    let mut attempt: u32 = 0;
    loop {
        if *stop.borrow() {
            return Err(Error::Cancelled);
        }
        let rel = done.load(Ordering::Relaxed);
        let finished_before_start = if ctx.ranged {
            start + rel > end
        } else {
            expected.is_some_and(|t| rel >= t)
        };
        if finished_before_start {
            return Ok(());
        }

        file.seek(SeekFrom::Start(start + rel)).await?;
        let mut req = ctx.client.get(&ctx.url).headers(ctx.headers.clone());
        if ctx.ranged {
            req = req.header(RANGE, format!("bytes={}-{}", start + rel, end));
        }
        let sent = req.send().await;
        let resp = match sent {
            Ok(resp) => resp,
            Err(e) => {
                if attempt >= MAX_RETRIES {
                    return Err(e.into());
                }
                attempt += 1;
                backoff(attempt, &mut stop).await?;
                if !ctx.ranged {
                    done.store(0, Ordering::Relaxed);
                }
                continue;
            }
        };
        let status_ok = if ctx.ranged {
            resp.status() == StatusCode::PARTIAL_CONTENT
        } else {
            resp.status().is_success()
        };
        if !status_ok {
            return Err(Error::Status(resp.status().as_u16(), ctx.url.clone()));
        }
        match consume(resp, &mut file, &mut stop, &done, start, end, &ctx).await {
            Ok(false) => return Err(Error::Cancelled),
            Ok(true) => {
                let rel_now = done.load(Ordering::Relaxed);
                let finished = if ctx.ranged {
                    start + rel_now > end
                } else {
                    expected.is_none_or(|t| rel_now >= t)
                };
                if finished {
                    return Ok(());
                }
                if attempt >= MAX_RETRIES {
                    return Err(Error::Truncated);
                }
                attempt += 1;
                backoff(attempt, &mut stop).await?;
                // range requests resume from current offset, plain ones must restart
                if !ctx.ranged {
                    done.store(0, Ordering::Relaxed);
                }
            }
            Err(e) => {
                if attempt >= MAX_RETRIES {
                    return Err(e);
                }
                attempt += 1;
                backoff(attempt, &mut stop).await?;
                if !ctx.ranged {
                    done.store(0, Ordering::Relaxed);
                }
            }
        }
    }
}

async fn consume(
    resp: reqwest::Response,
    file: &mut tokio::fs::File,
    stop: &mut watch::Receiver<bool>,
    done: &AtomicU64,
    start: u64,
    end: u64,
    ctx: &SegCtx,
) -> Result<bool, Error> {
    let mut stream = resp.bytes_stream();
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => return Ok(false),
            chunk = stream.next() => match chunk {
                Some(Ok(bytes)) => {
                    let written = if ctx.ranged {
                        let done_now = done.load(Ordering::Relaxed);
                        let remaining = end.saturating_sub(start + done_now).saturating_add(1);
                        let want = remaining.min(bytes.len() as u64) as usize;
                        if want == 0 {
                            continue;
                        }
                        write_throttled(file, &bytes[..want], ctx.limiter.as_ref()).await?
                    } else {
                        write_throttled(file, &bytes, ctx.limiter.as_ref()).await?
                    };
                    done.fetch_add(written, Ordering::Relaxed);
                }
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(true),
            },
        }
    }
}

async fn write_throttled(
    file: &mut tokio::fs::File,
    data: &[u8],
    limiter: Option<&SpeedLimiter>,
) -> Result<u64, Error> {
    let mut offset = 0;
    while offset < data.len() {
        let n = (data.len() - offset).min(WRITE_SLICE);
        if let Some(limiter) = limiter {
            limiter.acquire(n as u64).await;
        }
        file.write_all(&data[offset..offset + n]).await?;
        offset += n;
    }
    Ok(data.len() as u64)
}

async fn backoff(attempt: u32, stop: &mut watch::Receiver<bool>) -> Result<(), Error> {
    let ms = 250u64.saturating_mul(1 << attempt.min(5));
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(ms)) => Ok(()),
        _ = stop.changed() => Err(Error::Cancelled),
    }
}

fn sum_done(done: &[Arc<AtomicU64>]) -> u64 {
    done.iter().map(|d| d.load(Ordering::Relaxed)).sum()
}

fn build_headers(pairs: &[(String, String)]) -> Result<HeaderMap, Error> {
    let mut map = HeaderMap::new();
    for (k, v) in pairs {
        let name = HeaderName::try_from(k.as_str()).map_err(|_| Error::BadHeader(k.clone()))?;
        let value = HeaderValue::from_str(v).map_err(|_| Error::BadHeader(k.clone()))?;
        map.insert(name, value);
    }
    Ok(map)
}

fn header_str(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn parse_content_range_total(value: &str) -> Option<u64> {
    let (_, total) = value.rsplit_once('/')?;
    let total = total.trim();
    if total == "*" {
        None
    } else {
        total.parse().ok()
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

async fn read_meta(path: &Path) -> Option<Meta> {
    let data = fs::read(path).await.ok()?;
    serde_json::from_slice(&data).ok()
}

async fn persist_meta(path: &Path, meta: &Meta) -> Result<(), Error> {
    let json = serde_json::to_vec(meta)?;
    let tmp = with_suffix(path, ".tmp");
    fs::write(&tmp, json).await?;
    let _ = fs::remove_file(path).await;
    fs::rename(&tmp, path).await?;
    Ok(())
}
