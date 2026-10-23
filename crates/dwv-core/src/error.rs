#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("server returned status {0}")]
    Status(u16),
    #[error("invalid header: {0}")]
    BadHeader(String),
    #[error("connection closed before download completed")]
    Truncated,
    #[error("cancelled")]
    Cancelled,
    #[error("internal task error: {0}")]
    Internal(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}
