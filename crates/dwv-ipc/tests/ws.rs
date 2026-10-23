use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use dwv_ipc::{Request, serve};

const ORIGIN: &str = "chrome-extension://abcdefghijklmnop";

async fn connect(
    addr: std::net::SocketAddr,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>> {
    let mut request = format!("ws://{addr}").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("origin", HeaderValue::from_static(ORIGIN));
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("handshake failed");
    ws
}

#[tokio::test]
async fn add_roundtrip() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();
    let mut ws = connect(addr).await;

    ws.send(Message::Text(
        r#"{"id":7,"type":"add","url":"https://example.com/f.zip","filename":"f.zip","referrer":"https://example.com/"}"#.into(),
    ))
    .await
    .unwrap();

    let reply = ws.next().await.unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["id"], 7);

    match rx.recv().await.unwrap() {
        Request::Add {
            url,
            filename,
            referrer,
        } => {
            assert_eq!(url, "https://example.com/f.zip");
            assert_eq!(filename.as_deref(), Some("f.zip"));
            assert_eq!(referrer.as_deref(), Some("https://example.com/"));
        }
        other => panic!("unexpected request: {other:?}"),
    }
}

#[tokio::test]
async fn ping_is_answered_without_reaching_the_queue() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();
    let mut ws = connect(addr).await;

    ws.send(Message::Text(r#"{"id":1,"type":"ping"}"#.into()))
        .await
        .unwrap();
    let reply = ws.next().await.unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert_eq!(value["ok"], true);

    assert!(rx.try_recv().is_err(), "ping must not appear in the queue");
}

#[tokio::test]
async fn rejects_web_page_origin() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let denied = raw_handshake(addr, &addr.to_string(), Some("http://evil.example")).await;
    assert!(denied.starts_with("HTTP/1.1 403"), "got: {denied}");

    let allowed = raw_handshake(addr, &addr.to_string(), Some(ORIGIN)).await;
    assert!(allowed.starts_with("HTTP/1.1 101"), "got: {allowed}");
}

#[tokio::test]
async fn missing_origin_is_rejected() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let response = raw_handshake(addr, &addr.to_string(), None).await;
    assert!(response.starts_with("HTTP/1.1 403"), "got: {response}");
}

#[tokio::test]
async fn null_origin_is_rejected() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    let response = raw_handshake(addr, &addr.to_string(), Some("null")).await;
    assert!(response.starts_with("HTTP/1.1 403"), "got: {response}");
}

#[tokio::test]
async fn bad_host_is_rejected() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();

    for host in [format!("evil.example:{}", addr.port()), String::new()] {
        let response = raw_handshake(addr, &host, Some(ORIGIN)).await;
        assert!(response.starts_with("HTTP/1.1 403"), "{host:?}: {response}");
    }
}

async fn raw_handshake(addr: std::net::SocketAddr, host: &str, origin: Option<&str>) -> String {
    let host_line = if host.is_empty() {
        String::new()
    } else {
        format!("Host: {host}\r\n")
    };
    let origin_line = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "GET / HTTP/1.1\r\n{host_line}Upgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{origin_line}\r\n"
    );
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

#[tokio::test]
async fn bad_json_gets_an_error_reply() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let addr = serve(0, tx).await.unwrap();
    let mut ws = connect(addr).await;

    ws.send(Message::Text("not json".into())).await.unwrap();
    let reply = ws.next().await.unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert_eq!(value["ok"], false);
    assert!(rx.try_recv().is_err());

    ws.send(Message::Text(
        r#"{"id":2,"type":"add","url":"ftp://example.com/f.zip"}"#.into(),
    ))
    .await
    .unwrap();
    let reply = ws.next().await.unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert_eq!(value["ok"], false);
    assert!(rx.try_recv().is_err());
}
