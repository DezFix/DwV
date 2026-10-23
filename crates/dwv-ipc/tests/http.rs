use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use dwv_ipc::{Request, serve};

const ORIGIN: &str = "chrome-extension://abcdefghijklmnop";

async fn raw(addr: std::net::SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

fn host(addr: std::net::SocketAddr) -> String {
    format!("127.0.0.1:{}", addr.port())
}

fn request(
    method: &str,
    path: &str,
    host: Option<&str>,
    origin: Option<&str>,
    body: &str,
) -> String {
    let host_line = host
        .map(|host| format!("Host: {host}\r\n"))
        .unwrap_or_default();
    let origin_line = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    format!(
        "{method} {path} HTTP/1.1\r\n{host_line}Content-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n{origin_line}\r\n{body}",
        body.len()
    )
}

fn post(addr: std::net::SocketAddr, path: &str, body: &str) -> String {
    request("POST", path, Some(&host(addr)), Some(ORIGIN), body)
}

fn status(response: &str) -> u16 {
    response.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn response_header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    response
        .split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .find_map(|line| {
            let (header, value) = line.split_once(':')?;
            header.eq_ignore_ascii_case(name).then(|| value.trim())
        })
}

fn json(response: &str) -> serde_json::Value {
    let body = response.split_once("\r\n\r\n").unwrap().1;
    serde_json::from_str(body).unwrap()
}

#[tokio::test]
async fn download_is_accepted_and_delivered() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let response = raw(
        addr,
        &post(
            addr,
            "/",
            r#"{"type":"download","url":"https://example.com/a.zip","final_url":"https://cdn.example.com/a.zip","filename":"a.zip","referrer":"https://example.com/","cookies":"a=b","user_agent":"test-agent","headers":{"Accept":"*/*"}}"#,
        ),
    )
    .await;

    assert_eq!(status(&response), 200, "{response}");
    assert_eq!(json(&response)["ok"], true);

    match rx.recv().await.unwrap() {
        Request::Download {
            url,
            final_url,
            filename,
            referrer,
            cookies,
            user_agent,
            headers,
        } => {
            assert_eq!(url, "https://example.com/a.zip");
            assert_eq!(final_url.as_deref(), Some("https://cdn.example.com/a.zip"));
            assert_eq!(filename.as_deref(), Some("a.zip"));
            assert_eq!(referrer.as_deref(), Some("https://example.com/"));
            assert_eq!(cookies.as_deref(), Some("a=b"));
            assert_eq!(user_agent.as_deref(), Some("test-agent"));
            assert_eq!(headers.get("Accept").map(String::as_str), Some("*/*"));
        }
        other => panic!("unexpected request: {other:?}"),
    }
}

#[tokio::test]
async fn torrent_is_accepted() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let response = raw(
        addr,
        &post(
            addr,
            "/ipc",
            r#"{"type":"torrent","source":"magnet:?xt=urn:btih:abcdef","referrer":"https://example.com/"}"#,
        ),
    )
    .await;

    assert_eq!(status(&response), 200, "{response}");
    assert_eq!(json(&response)["ok"], true);

    match rx.recv().await.unwrap() {
        Request::Torrent {
            source,
            referrer,
            cookies,
            user_agent,
        } => {
            assert_eq!(source, "magnet:?xt=urn:btih:abcdef");
            assert_eq!(referrer.as_deref(), Some("https://example.com/"));
            assert_eq!(cookies, None);
            assert_eq!(user_agent, None);
        }
        other => panic!("unexpected request: {other:?}"),
    }
}

#[tokio::test]
async fn ping_is_answered_and_reaches_the_app() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let response = raw(addr, &post(addr, "/", r#"{"type":"ping"}"#)).await;

    assert_eq!(status(&response), 200, "{response}");
    assert_eq!(json(&response)["ok"], true);
    assert!(
        matches!(rx.try_recv(), Ok(Request::Ping)),
        "the app needs the ping to restore a hidden window"
    );
}

#[tokio::test]
async fn malformed_json_gets_400() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let response = raw(addr, &post(addr, "/", "not json")).await;

    assert_eq!(status(&response), 400, "{response}");
    assert_eq!(json(&response)["ok"], false);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn extension_origin_gets_200_and_is_echoed() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let host = format!("LocalHost:{}", addr.port());
    let request = request("POST", "/", Some(&host), Some(ORIGIN), r#"{"type":"ping"}"#);
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 200, "{response}");
    assert_eq!(
        response_header(&response, "access-control-allow-origin"),
        Some(ORIGIN)
    );
}

#[tokio::test]
async fn moz_extension_origin_gets_200() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = request(
        "POST",
        "/",
        Some(&host(addr)),
        Some("moz-extension://1234abcd-1234-abcd-1234-abcdef123456"),
        r#"{"type":"ping"}"#,
    );
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 200, "{response}");
}

#[tokio::test]
async fn missing_origin_gets_403() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = request("POST", "/", Some(&host(addr)), None, r#"{"type":"ping"}"#);
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 403, "{response}");
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn null_origin_gets_403() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = request(
        "POST",
        "/ipc",
        Some(&host(addr)),
        Some("null"),
        r#"{"type":"ping"}"#,
    );
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 403, "{response}");
    assert_eq!(
        response_header(&response, "access-control-allow-origin"),
        None
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn web_origin_gets_403() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = request(
        "POST",
        "/",
        Some(&host(addr)),
        Some("http://evil.example"),
        r#"{"type":"ping"}"#,
    );
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 403, "{response}");
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn malformed_extension_origin_gets_403() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    for origin in ["chrome-extension://", "chrome-extension://id/path"] {
        let request = request(
            "POST",
            "/",
            Some(&host(addr)),
            Some(origin),
            r#"{"type":"ping"}"#,
        );
        let response = raw(addr, &request).await;
        assert_eq!(status(&response), 403, "{origin}: {response}");
    }
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn bad_host_gets_403() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    for bad_host in [
        format!("evil.example:{}", addr.port()),
        format!("127.0.0.1:{}", addr.port().saturating_add(1)),
        format!("[::2]:{}", addr.port()),
        String::new(),
    ] {
        let host = Some(bad_host.as_str()).filter(|host| !host.is_empty());
        let request = request("POST", "/", host, Some(ORIGIN), r#"{"type":"ping"}"#);
        let response = raw(addr, &request).await;
        assert_eq!(status(&response), 403, "{bad_host:?}: {response}");
    }
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn missing_host_gets_403() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = request("POST", "/", None, Some(ORIGIN), r#"{"type":"ping"}"#);
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 403, "{response}");
}

#[tokio::test]
async fn ipv6_loopback_host_is_accepted() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let host = format!("[::1]:{}", addr.port());
    let request = request("POST", "/", Some(&host), Some(ORIGIN), r#"{"type":"ping"}"#);
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 200, "{response}");
}

#[tokio::test]
async fn oversized_body_gets_413() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = format!(
        "POST / HTTP/1.1\r\nHost: {}\r\nOrigin: {ORIGIN}\r\n\
         Content-Type: application/json\r\nContent-Length: 65537\r\n\
         Connection: close\r\n\r\n",
        host(addr)
    );
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 413, "{response}");
}

#[tokio::test]
async fn health_returns_ok() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = format!(
        "GET /health HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host(addr)
    );
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 200, "{response}");
    assert_eq!(json(&response)["ok"], true);
    assert_eq!(
        response_header(&response, "access-control-allow-origin"),
        None
    );
}

#[tokio::test]
async fn health_rejects_bad_host() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = "GET /health HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n";
    let response = raw(addr, request).await;

    assert_eq!(status(&response), 403, "{response}");
}

#[tokio::test]
async fn options_is_allowed_without_origin() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let request = request("OPTIONS", "/ipc", Some(&host(addr)), None, "");
    let response = raw(addr, &request).await;

    assert_eq!(status(&response), 204, "{response}");
    assert_eq!(
        response_header(&response, "access-control-allow-origin"),
        None
    );
}

#[tokio::test]
async fn video_is_accepted() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let response = raw(
        addr,
        &post(
            addr,
            "/ipc",
            r#"{"type":"video","url":"https://www.youtube.com/watch?v=abc","referrer":"https://www.youtube.com/"}"#,
        ),
    )
    .await;

    assert_eq!(status(&response), 200, "{response}");
    assert_eq!(json(&response)["ok"], true);

    match rx.recv().await.unwrap() {
        Request::Video { url, referrer } => {
            assert_eq!(url, "https://www.youtube.com/watch?v=abc");
            assert_eq!(referrer.as_deref(), Some("https://www.youtube.com/"));
        }
        other => panic!("unexpected request: {other:?}"),
    }
}
