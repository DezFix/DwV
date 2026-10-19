use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Row, params};

use crate::engine::DownloadSpec;
use crate::error::Error;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS downloads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    target TEXT NOT NULL,
    state TEXT NOT NULL,
    total_bytes INTEGER,
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    connections INTEGER NOT NULL,
    headers_json TEXT NOT NULL DEFAULT '[]',
    error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    finished_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_downloads_state ON downloads(state);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Queued => "queued",
            State::Downloading => "downloading",
            State::Paused => "paused",
            State::Completed => "completed",
            State::Failed => "failed",
            State::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(State::Queued),
            "downloading" => Some(State::Downloading),
            "paused" => Some(State::Paused),
            "completed" => Some(State::Completed),
            "failed" => Some(State::Failed),
            "cancelled" => Some(State::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoredDownload {
    pub id: i64,
    pub url: String,
    pub target: PathBuf,
    pub state: State,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub connections: usize,
    pub headers: Vec<(String, String)>,
    pub error: Option<String>,
    pub created_at: i64,
}

impl StoredDownload {
    pub fn to_spec(&self) -> DownloadSpec {
        DownloadSpec {
            url: self.url.clone(),
            target: self.target.clone(),
            connections: self.connections,
            headers: self.headers.clone(),
            limiter: None,
        }
    }
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn insert(&self, spec: &DownloadSpec) -> Result<i64, Error> {
        let now = now_secs();
        let headers = serde_json::to_string(&spec.headers)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO downloads (url, target, state, connections, headers_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                spec.url,
                // shortcut: non-UTF-8 path bytes are lost in history
                spec.target.to_string_lossy(),
                State::Queued.as_str(),
                spec.connections as i64,
                headers,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn set_state(&self, id: i64, state: State) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE downloads SET state = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, state.as_str(), now_secs()],
        )?;
        Ok(())
    }

    pub fn set_progress(&self, id: i64, downloaded: u64, total: Option<u64>) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE downloads SET downloaded_bytes = ?2, total_bytes = COALESCE(?3, total_bytes), updated_at = ?4 WHERE id = ?1",
            params![id, downloaded as i64, total.map(|t| t as i64), now_secs()],
        )?;
        Ok(())
    }

    pub fn set_error(&self, id: i64, error: &str) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE downloads SET error = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, error, now_secs()],
        )?;
        Ok(())
    }

    pub fn finish(&self, id: i64, state: State) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        let now = now_secs();
        conn.execute(
            "UPDATE downloads SET state = ?2, finished_at = ?3, updated_at = ?3 WHERE id = ?1",
            params![id, state.as_str(), now],
        )?;
        Ok(())
    }

    pub fn load_pending(&self) -> Result<Vec<StoredDownload>, Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM downloads WHERE state IN ('queued', 'downloading') ORDER BY id",
        )?;
        let rows = stmt.query_map([], row_to_stored)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<StoredDownload>, Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM downloads ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], row_to_stored)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }
}

fn row_to_stored(row: &Row<'_>) -> rusqlite::Result<StoredDownload> {
    let headers_json: String = row.get("headers_json")?;
    let state: String = row.get("state")?;
    Ok(StoredDownload {
        id: row.get("id")?,
        url: row.get("url")?,
        target: PathBuf::from(row.get::<_, String>("target")?),
        state: State::parse(&state).unwrap_or(State::Queued),
        total_bytes: row
            .get::<_, Option<i64>>("total_bytes")?
            .map(|v| v.max(0) as u64),
        downloaded_bytes: row.get::<_, i64>("downloaded_bytes")?.max(0) as u64,
        connections: row.get::<_, i64>("connections")?.max(1) as usize,
        headers: serde_json::from_str(&headers_json).unwrap_or_default(),
        error: row.get("error")?,
        created_at: row.get("created_at")?,
    })
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
