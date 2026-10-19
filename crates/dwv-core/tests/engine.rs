use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dwv_core::{Download, DownloadSpec, Event, Outcome};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct TestServer {
    addr: SocketAddr,
    ranges_seen: Arc<Mutex<Vec<Option<String>>>>,
}

impl TestServer {
    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn ranges(&self) -> Vec<Option<String>> {
        self.ranges_seen.lock().unwrap().clone()
    }
}

async fn start_server(body: Vec<u8>, advertise_ranges: bool, chunk_delay: Duration) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let ranges_seen = Arc::new(Mutex::new(Vec::new()));
    let body = Arc::new(body);
    let seen = ranges_seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let _ = handle_conn(sock, body, advertise_ranges, chunk_delay, seen).await;
            });
        }
    });
    TestServer { addr, ranges_seen }
}

async fn handle_conn(
    mut sock: TcpStream,
    body: Arc<Vec<u8>>,
    advertise_ranges: bool,
    chunk_delay: Duration,
    seen: Arc<Mutex<Vec<Option<String>>>>,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    loop {
        let n = sock.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            return Ok(());
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let mut range = None;
    for line in head.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("range")
        {
            range = Some(v.trim().to_string());
        }
    }
    seen.lock().unwrap().push(range.clone());

    let len = body.len() as u64;
    let use_range = advertise_ranges && range.is_some();
    let (status, start, end) = if use_range {
        match parse_range(range.as_deref().unwrap_or(""), len) {
            Some((s, e)) => ("206 Partial Content", s, e),
            None => ("416 Range Not Satisfiable", 0, 0),
        }
    } else {
        ("200 OK", 0, len.saturating_sub(1))
    };

    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nETag: \"test-etag\"\r\n",
        end - start + 1
    );
    if advertise_ranges {
        response.push_str("Accept-Ranges: bytes\r\n");
    }
    if use_range {
        response.push_str(&format!("Content-Range: bytes {start}-{end}/{len}\r\n"));
    }
    response.push_str("\r\n");
    sock.write_all(response.as_bytes()).await?;

    if status.starts_with('4') {
        return Ok(());
    }
    for chunk in body[start as usize..=end as usize].chunks(16 * 1024) {
        sock.write_all(chunk).await?;
        sock.flush().await?;
        if !chunk_delay.is_zero() {
            tokio::time::sleep(chunk_delay).await;
        }
    }
    let _ = sock.shutdown().await;
    Ok(())
}

fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    let (s, e) = spec.split_once('-')?;
    let start: u64 = s.parse().ok()?;
    let end: u64 = if e.is_empty() {
        len.saturating_sub(1)
    } else {
        e.parse().ok()?
    };
    if start > end || start >= len {
        return None;
    }
    Some((start, end.min(len - 1)))
}

fn test_body(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| seed.wrapping_add(i as u8).wrapping_mul(31))
        .collect()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

fn spec_for(server: &TestServer, target: &Path, connections: usize) -> DownloadSpec {
    let mut spec = DownloadSpec::new(server.url("/file.bin"), target);
    spec.connections = connections;
    spec
}

#[tokio::test]
async fn single_stream_when_server_ignores_ranges() {
    let body = test_body(7, 256 * 1024);
    let server = start_server(body.clone(), false, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");

    let mut dl = Download::start(spec_for(&server, &target, 4));
    let mut started_connections = 0;
    let mut finished = false;
    while let Some(ev) = dl.events().recv().await {
        match ev {
            Event::Started { connections, .. } => started_connections = connections,
            Event::Finished { .. } => finished = true,
            Event::Progress { .. } => {}
        }
    }
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert!(finished);
    assert_eq!(started_connections, 1, "must fall back to a single stream");
    assert_eq!(std::fs::read(&target).unwrap(), body);
    assert!(!with_suffix(&target, ".part").exists());
    assert!(!with_suffix(&target, ".dwvmeta").exists());

    let ranges = server.ranges();
    assert_eq!(ranges.len(), 2, "probe + one plain request");
    assert_eq!(ranges[1], None, "download request must not use ranges");
}

#[tokio::test]
async fn segmented_download_matches_body() {
    let body = test_body(3, 1024 * 1024);
    let server = start_server(body.clone(), true, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");

    let mut dl = Download::start(spec_for(&server, &target, 4));
    let mut started_connections = 0;
    while let Some(ev) = dl.events().recv().await {
        if let Event::Started { connections, .. } = ev {
            started_connections = connections;
        }
    }
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert_eq!(started_connections, 4);
    assert_eq!(std::fs::read(&target).unwrap(), body);

    let ranges = server.ranges();
    let segment_requests = ranges
        .iter()
        .filter(|r| matches!(r.as_deref(), Some(s) if s != "bytes=0-0"))
        .count();
    assert_eq!(segment_requests, 4);
}

#[tokio::test]
async fn resume_after_cancel() {
    let body = test_body(9, 2 * 1024 * 1024);
    let server = start_server(body.clone(), true, Duration::from_millis(20)).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");
    let spec = spec_for(&server, &target, 2);

    let mut dl = Download::start(spec.clone());
    let controller = dl.controller();
    loop {
        match dl.events().recv().await.unwrap() {
            Event::Progress { downloaded, .. } if downloaded > 0 => break,
            Event::Finished { .. } => panic!("finished before cancel"),
            _ => {}
        }
    }
    controller.cancel();
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Cancelled);

    let part = with_suffix(&target, ".part");
    let meta_path = with_suffix(&target, ".dwvmeta");
    assert!(part.exists(), "part file must be kept for resume");
    assert!(meta_path.exists());
    // part file is preallocated to full size, so progress lives in the metadata
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    let done: u64 = meta["done"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert!(done > 0 && done < body.len() as u64);

    let before = server.ranges().len();
    let mut dl2 = Download::start(spec);
    let mut resumed = false;
    while let Some(ev) = dl2.events().recv().await {
        match ev {
            Event::Started { resumed: r, .. } => resumed = r,
            Event::Finished { .. } => break,
            _ => {}
        }
    }
    let outcome = dl2.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert!(resumed, "second run must resume, not restart");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        body,
        "resumed file is corrupt"
    );

    let ranges = server.ranges();
    let resumed_from_offset = ranges[before..].iter().any(|r| {
        r.as_deref()
            .and_then(|s| s.strip_prefix("bytes="))
            .and_then(|s| s.split('-').next())
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|start| start > 0)
    });
    assert!(resumed_from_offset, "no ranged request past offset 0");
}
