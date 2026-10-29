use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as HandshakeRequest, Response as HandshakeResponse,
};
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

pub const DEFAULT_PORT: u16 = 41763;

const MAX_BODY: usize = 64 * 1024;
const MAX_HEAD: usize = 16 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Add {
        url: String,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        referrer: Option<String>,
    },
    Download {
        url: String,
        #[serde(default)]
        final_url: Option<String>,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        referrer: Option<String>,
        #[serde(default)]
        cookies: Option<String>,
        #[serde(default)]
        user_agent: Option<String>,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Torrent {
        source: String,
        #[serde(default)]
        referrer: Option<String>,
        #[serde(default)]
        cookies: Option<String>,
        #[serde(default)]
        user_agent: Option<String>,
    },
    Video {
        url: String,
        #[serde(default)]
        referrer: Option<String>,
    },
    Ping,
}

#[derive(Debug, Serialize)]
struct Response {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub async fn serve(
    port: u16,
    tx: mpsc::UnboundedSender<Request>,
) -> Result<SocketAddr, std::io::Error> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    let bound_port = addr.port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let _ = handle_conn(stream, tx, bound_port).await;
            });
        }
    });
    Ok(addr)
}

async fn handle_conn(
    stream: TcpStream,
    tx: mpsc::UnboundedSender<Request>,
    port: u16,
) -> std::io::Result<()> {
    let head = peek_head(&stream).await?;
    if is_websocket_upgrade(&head) {
        handle_ws(stream, tx, port)
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))
    } else {
        match tokio::time::timeout(HTTP_TIMEOUT, handle_http(stream, tx, port)).await {
            Ok(result) => result,
            Err(_) => Ok(()),
        }
    }
}

async fn peek_head(stream: &TcpStream) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; MAX_HEAD];
    let deadline = tokio::time::Instant::now() + HTTP_TIMEOUT;
    loop {
        let n = stream.peek(&mut buf).await?;
        if n == 0 {
            return Ok(Vec::new());
        }
        if header_end(&buf[..n]).is_some()
            || n == buf.len()
            || tokio::time::Instant::now() >= deadline
        {
            return Ok(buf[..n].to_vec());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn is_websocket_upgrade(head: &[u8]) -> bool {
    let Ok(head) = std::str::from_utf8(head) else {
        return false;
    };
    let mut lines = head.split("\r\n");
    let Some(request_line) = lines.next() else {
        return false;
    };
    if !request_line.starts_with("GET ") {
        return false;
    }
    lines.any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.eq_ignore_ascii_case("upgrade") && value.trim().eq_ignore_ascii_case("websocket")
    })
}

// callback signature is fixed by tungstenite, hence the large Err type
#[allow(clippy::result_large_err)]
async fn handle_ws(
    stream: TcpStream,
    tx: mpsc::UnboundedSender<Request>,
    port: u16,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let config = WebSocketConfig::default().max_message_size(Some(64 * 1024));
    let callback = move |req: &HandshakeRequest, resp: HandshakeResponse| {
        let host = req
            .headers()
            .get("host")
            .and_then(|value| value.to_str().ok());
        let origin = req
            .headers()
            .get("origin")
            .and_then(|value| value.to_str().ok());
        if host_allowed(host, port) && origin_allowed(origin) {
            Ok(resp)
        } else {
            let mut err = ErrorResponse::new(Some("host or origin not allowed".to_string()));
            *err.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
            Err(err)
        }
    };
    let mut ws =
        tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config)).await?;

    while let Some(message) = ws.next().await {
        let message = message?;
        let text = match message {
            Message::Text(text) => text,
            Message::Ping(payload) => {
                ws.send(Message::Pong(payload)).await?;
                continue;
            }
            Message::Close(_) => break,
            _ => continue,
        };

        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&text);
        let id = parsed
            .as_ref()
            .ok()
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_u64());
        let response = match parsed.map(serde_json::from_value::<Request>) {
            Ok(Ok(Request::Ping)) => Response {
                id,
                ok: true,
                error: None,
            },
            Ok(Ok(request)) => match validate(&request) {
                Ok(()) => {
                    let _ = tx.send(request);
                    Response {
                        id,
                        ok: true,
                        error: None,
                    }
                }
                Err(error) => Response {
                    id,
                    ok: false,
                    error: Some(error),
                },
            },
            Ok(Err(error)) => Response {
                id,
                ok: false,
                error: Some(error.to_string()),
            },
            Err(error) => Response {
                id,
                ok: false,
                error: Some(error.to_string()),
            },
        };
        let text = serde_json::to_string(&response)
            .unwrap_or_else(|_| r#"{"ok":false,"error":"serialize failed"}"#.to_string());
        ws.send(Message::Text(text.into())).await?;
    }
    Ok(())
}

struct HttpHead {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl HttpHead {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

async fn handle_http(
    mut stream: TcpStream,
    tx: mpsc::UnboundedSender<Request>,
    port: u16,
) -> std::io::Result<()> {
    let (head, rest) = match read_http_head(&mut stream).await {
        Ok(value) => value,
        Err(_) => return write_json(&mut stream, 400, &error_body("bad request"), None).await,
    };
    if !host_allowed(head.header("host"), port) {
        return write_json(&mut stream, 403, &error_body("host not allowed"), None).await;
    }
    let origin = head.header("origin");
    let allowed_origin = origin.filter(|value| extension_origin(value));
    if head.method == "POST" && allowed_origin.is_none() {
        return write_json(&mut stream, 403, &error_body("origin not allowed"), None).await;
    }

    match head.method.as_str() {
        "OPTIONS" => write_response(&mut stream, 204, "text/plain", b"", allowed_origin).await,
        "GET" if head.path == "/health" => {
            write_json(&mut stream, 200, r#"{"ok":true}"#, allowed_origin).await
        }
        "POST" if head.path == "/" || head.path == "/ipc" => {
            let length = match content_length(&head) {
                Ok(value) => value,
                Err(()) => {
                    return write_json(
                        &mut stream,
                        400,
                        &error_body("bad content-length"),
                        allowed_origin,
                    )
                    .await;
                }
            };
            let body = match read_body(&mut stream, rest, length).await {
                Ok(value) => value,
                Err(status) => {
                    return write_json(
                        &mut stream,
                        status,
                        &error_body("request body rejected"),
                        allowed_origin,
                    )
                    .await;
                }
            };
            let request: Request = match serde_json::from_slice(&body) {
                Ok(value) => value,
                Err(error) => {
                    return write_json(
                        &mut stream,
                        400,
                        &error_body(&error.to_string()),
                        allowed_origin,
                    )
                    .await;
                }
            };
            let (status, body) = match request {
                Request::Ping => {
                    // Let the app react (show a hidden window) but answer at once.
                    let _ = tx.send(Request::Ping);
                    (200, r#"{"ok":true}"#.to_string())
                }
                request => match validate(&request) {
                    Ok(()) => {
                        let _ = tx.send(request);
                        (200, r#"{"ok":true}"#.to_string())
                    }
                    Err(error) => (200, error_body(&error)),
                },
            };
            write_json(&mut stream, status, &body, allowed_origin).await
        }
        "POST" => write_json(&mut stream, 404, &error_body("not found"), allowed_origin).await,
        _ => {
            write_json(
                &mut stream,
                405,
                &error_body("method not allowed"),
                allowed_origin,
            )
            .await
        }
    }
}

async fn read_http_head(stream: &mut TcpStream) -> std::io::Result<(HttpHead, Vec<u8>)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before request head",
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = header_end(&buf) {
            let head = parse_head(&buf[..end])
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            return Ok((head, buf[end + 4..].to_vec()));
        }
        if buf.len() > MAX_HEAD {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request head too large",
            ));
        }
    }
}

fn parse_head(bytes: &[u8]) -> Result<HttpHead, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "request head is not utf-8")?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or("missing request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("missing method")?.to_string();
    let target = parts.next().ok_or("missing request target")?;
    let version = parts.next().ok_or("missing http version")?;
    if !version.starts_with("HTTP/1.") {
        return Err("unsupported http version");
    }
    let path = target.split(['?', '#']).next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or("malformed header")?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    Ok(HttpHead {
        method,
        path,
        headers,
    })
}

fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(head: &HttpHead) -> Result<Option<usize>, ()> {
    match head.header("content-length") {
        None => Ok(None),
        Some(value) => value.trim().parse::<usize>().map(Some).map_err(|_| ()),
    }
}

async fn read_body(
    stream: &mut TcpStream,
    rest: Vec<u8>,
    content_length: Option<usize>,
) -> Result<Vec<u8>, u16> {
    match content_length {
        Some(length) => {
            if length > MAX_BODY {
                return Err(413);
            }
            let mut body = rest;
            body.truncate(length);
            while body.len() < length {
                let mut chunk = vec![0u8; (length - body.len()).min(8192)];
                let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
                if n == 0 {
                    return Err(400);
                }
                body.extend_from_slice(&chunk[..n]);
            }
            Ok(body)
        }
        None => {
            let mut body = rest;
            if body.len() > MAX_BODY {
                return Err(413);
            }
            let mut chunk = [0u8; 8192];
            loop {
                let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
                if n == 0 {
                    return Ok(body);
                }
                body.extend_from_slice(&chunk[..n]);
                if body.len() > MAX_BODY {
                    return Err(413);
                }
            }
        }
    }
}

async fn write_json(
    stream: &mut TcpStream,
    status: u16,
    body: &str,
    origin: Option<&str>,
) -> std::io::Result<()> {
    write_response(stream, status, "application/json", body.as_bytes(), origin).await
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    origin: Option<&str>,
) -> std::io::Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nVary: Origin\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n",
        status_reason(status),
        body.len()
    );
    if let Some(origin) = origin {
        response.push_str("Access-Control-Allow-Origin: ");
        response.push_str(origin);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    stream.shutdown().await
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

fn error_body(error: &str) -> String {
    serde_json::to_string(&Response {
        id: None,
        ok: false,
        error: Some(error.to_string()),
    })
    .unwrap_or_else(|_| r#"{"ok":false,"error":"serialize failed"}"#.to_string())
}

fn validate(request: &Request) -> Result<(), String> {
    match request {
        Request::Add { url, .. } => validate_add(url),
        Request::Download {
            url,
            filename,
            referrer,
            cookies,
            user_agent,
            headers,
            ..
        } => {
            validate_add(url)?;
            if has_crlf(url) {
                return Err("url contains line breaks".to_string());
            }
            if filename.as_deref().is_some_and(has_crlf)
                || referrer.as_deref().is_some_and(has_crlf)
                || cookies.as_deref().is_some_and(has_crlf)
                || user_agent.as_deref().is_some_and(has_crlf)
                || headers
                    .iter()
                    .any(|(name, value)| has_crlf(name) || has_crlf(value))
            {
                return Err("line breaks are not allowed".to_string());
            }
            Ok(())
        }
        Request::Torrent {
            source,
            referrer,
            cookies,
            user_agent,
        } => {
            validate_torrent_source(source)?;
            if referrer.as_deref().is_some_and(has_crlf)
                || cookies.as_deref().is_some_and(has_crlf)
                || user_agent.as_deref().is_some_and(has_crlf)
            {
                return Err("line breaks are not allowed".to_string());
            }
            Ok(())
        }
        Request::Video { url, referrer } => {
            validate_add(url)?;
            if referrer.as_deref().is_some_and(has_crlf) {
                return Err("line breaks are not allowed".to_string());
            }
            Ok(())
        }
        Request::Ping => Ok(()),
    }
}

fn validate_add(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http:// and https:// URLs are supported".to_string());
    }
    if url.len() > 8 * 1024 {
        return Err("url is too long".to_string());
    }
    Ok(())
}

fn validate_torrent_source(source: &str) -> Result<(), String> {
    if has_crlf(source) {
        return Err("source contains line breaks".to_string());
    }
    if source.len() > 8 * 1024 {
        return Err("source is too long".to_string());
    }
    if source.starts_with("magnet:") {
        return Ok(());
    }
    // Trackers often serve .torrent files from URLs without the extension
    // (`download.php?id=...`), so any http(s) source is allowed; librqbit
    // rejects it later if the body is not a torrent.
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(());
    }
    Err("source must be a magnet link or an http(s) URL".to_string())
}

fn has_crlf(value: &str) -> bool {
    value.contains('\r') || value.contains('\n')
}

// only the loopback names the listener can actually be reached by, with the bound port
fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else {
        return false;
    };
    let (name_allowed, port_part) = if let Some(rest) = host.strip_prefix('[') {
        let Some((name, rest)) = rest.split_once(']') else {
            return false;
        };
        let Some(port_part) = rest.strip_prefix(':') else {
            return false;
        };
        (name == "::1", port_part)
    } else {
        let Some((name, port_part)) = host.rsplit_once(':') else {
            return false;
        };
        let name_allowed = !name.contains(':')
            && (name.eq_ignore_ascii_case("127.0.0.1") || name.eq_ignore_ascii_case("localhost"));
        (name_allowed, port_part)
    };
    name_allowed
        && port_part.bytes().all(|digit| digit.is_ascii_digit())
        && port_part.parse::<u16>() == Ok(port)
}

fn origin_allowed(origin: Option<&str>) -> bool {
    origin.is_some_and(extension_origin)
}

// browsers send extension origins as scheme://id with no path; anything else is malformed
fn extension_origin(origin: &str) -> bool {
    let id = origin
        .strip_prefix("chrome-extension://")
        .or_else(|| origin.strip_prefix("moz-extension://"));
    match id {
        Some(id) => {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        }
        None => false,
    }
}

// shortcut: any local process can spoof an extension Origin and Host, so this only
// blocks web pages and DNS-rebinding style callers. Real authentication needs a
// native messaging host or a per-install token; upgrade when local malware is in scope.
