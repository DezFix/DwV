#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dwv_core::DownloadSpec;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub struct TestServer {
    addr: SocketAddr,
    ranges_seen: Arc<Mutex<Vec<Option<String>>>>,
}

impl TestServer {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn ranges(&self) -> Vec<Option<String>> {
        self.ranges_seen.lock().unwrap().clone()
    }
}

pub async fn start_server(
    body: Vec<u8>,
    advertise_ranges: bool,
    chunk_delay: Duration,
) -> TestServer {
    start_server_with_failures(body, advertise_ranges, chunk_delay, 0).await
}

/// Refuses the first `failures` range requests (after the probe) with 503.
pub async fn start_failing_server(body: Vec<u8>, failures: usize) -> TestServer {
    start_server_with_failures(body, true, Duration::ZERO, failures).await
}

async fn start_server_with_failures(
    body: Vec<u8>,
    advertise_ranges: bool,
    chunk_delay: Duration,
    failures: usize,
) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let ranges_seen = Arc::new(Mutex::new(Vec::new()));
    let body = Arc::new(body);
    let seen = ranges_seen.clone();
    let failures = Arc::new(Mutex::new(failures));
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            let seen = seen.clone();
            let failures = failures.clone();
            tokio::spawn(async move {
                let _ =
                    handle_conn(sock, body, advertise_ranges, chunk_delay, seen, failures).await;
            });
        }
    });
    TestServer { addr, ranges_seen }
}

/// Accepts connections and never answers, like a server that is up but stuck.
pub async fn start_hanging_server() -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let ranges_seen = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                // hold the socket open until the client gives up
                let _ = sock.read(&mut [0u8; 1]).await;
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
    failures: Arc<Mutex<usize>>,
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

    let fail = range.as_deref().is_some_and(|r| r != "bytes=0-0") && {
        let mut remaining = failures.lock().unwrap();
        if *remaining > 0 {
            *remaining -= 1;
            true
        } else {
            false
        }
    };
    if fail {
        sock.write_all(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await?;
        return Ok(());
    }

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

pub fn test_body(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| seed.wrapping_add(i as u8).wrapping_mul(31))
        .collect()
}

pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

pub fn spec_for(server: &TestServer, target: &Path, connections: usize) -> DownloadSpec {
    let mut spec = DownloadSpec::new(server.url("/file.bin"), target);
    spec.connections = connections;
    spec
}
