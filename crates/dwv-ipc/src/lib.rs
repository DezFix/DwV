use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as HandshakeRequest, Response as HandshakeResponse,
};
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

pub const DEFAULT_PORT: u16 = 41763;

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
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let _ = handle_conn(stream, tx).await;
            });
        }
    });
    Ok(addr)
}

// callback signature is fixed by tungstenite, hence the large Err type
#[allow(clippy::result_large_err)]
async fn handle_conn(
    stream: TcpStream,
    tx: mpsc::UnboundedSender<Request>,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let config = WebSocketConfig::default().max_message_size(Some(64 * 1024));
    let callback = |req: &HandshakeRequest, resp: HandshakeResponse| {
        if origin_allowed(req.headers().get("origin")) {
            Ok(resp)
        } else {
            let mut err = ErrorResponse::new(Some("origin not allowed".to_string()));
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
            Ok(Ok(request @ Request::Add { .. })) => match validate_add(&request) {
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

fn validate_add(request: &Request) -> Result<(), String> {
    let Request::Add { url, .. } = request else {
        return Ok(());
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http:// and https:// URLs are supported".to_string());
    }
    if url.len() > 8 * 1024 {
        return Err("url is too long".to_string());
    }
    Ok(())
}

// web pages carry an http(s) origin; only browser extensions and native clients may talk to us
fn origin_allowed(origin: Option<&tokio_tungstenite::tungstenite::http::HeaderValue>) -> bool {
    match origin {
        None => true,
        Some(value) => match value.to_str() {
            Ok(origin) => {
                origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://")
            }
            Err(_) => false,
        },
    }
}
