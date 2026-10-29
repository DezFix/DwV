mod common;

use std::time::Duration;

use common::{spec_for, start_server, test_body};
use dwv_core::db::{Database, State};
use dwv_core::{DownloadSpec, Queue, QueueConfig, QueueEvent};

#[test]
fn state_roundtrips_through_strings() {
    for state in [
        State::Queued,
        State::Downloading,
        State::Paused,
        State::Completed,
        State::Failed,
        State::Cancelled,
        State::Interrupted,
    ] {
        assert_eq!(State::parse(state.as_str()), Some(state));
    }
}

#[tokio::test]
async fn settings_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("dwv.db")).unwrap();

    assert_eq!(db.get_setting("download_dir").unwrap(), None);
    db.set_setting("download_dir", "D:/Downloads").unwrap();
    assert_eq!(
        db.get_setting("download_dir").unwrap(),
        Some("D:/Downloads".to_string())
    );
    db.set_setting("download_dir", "E:/Other").unwrap();
    assert_eq!(
        db.get_setting("download_dir").unwrap(),
        Some("E:/Other".to_string())
    );
}

#[tokio::test]
async fn load_pending_returns_unfinished_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("dwv.db")).unwrap();

    let mut spec = DownloadSpec::new("http://example.com/a.bin", dir.path().join("a.bin"));
    spec.connections = 4;
    spec.headers = vec![("Referer".to_string(), "http://example.com".to_string())];
    let id = db.insert(&spec).unwrap();
    db.set_state(id, State::Downloading).unwrap();
    db.set_progress(id, 1024, Some(4096)).unwrap();

    let pending = db.load_pending().unwrap();
    assert_eq!(pending.len(), 1);
    let stored = &pending[0];
    assert_eq!(stored.url, spec.url);
    assert_eq!(stored.connections, 4);
    assert_eq!(stored.downloaded_bytes, 1024);
    assert_eq!(stored.total_bytes, Some(4096));
    assert!(stored.headers.is_empty(), "headers must not be persisted");

    let restored = stored.to_spec();
    assert_eq!(restored.url, spec.url);
    assert_eq!(restored.target, spec.target);
    assert!(restored.headers.is_empty());

    db.finish(id, State::Completed).unwrap();
    assert!(db.load_pending().unwrap().is_empty());
}

#[test]
fn redact_url_strips_query_fragment_and_userinfo() {
    assert_eq!(
        dwv_core::db::redact_url("https://user:pass@example.com/dir/file.zip?token=secret#frag"),
        "https://example.com/dir/file.zip"
    );
    assert_eq!(
        dwv_core::db::redact_url("http://example.com/a.bin?x=1"),
        "http://example.com/a.bin"
    );
}

#[test]
fn insert_stores_no_headers() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("dwv.db")).unwrap();
    let mut spec = DownloadSpec::new(
        "https://user:pass@example.com/file.bin?token=secret",
        dir.path().join("file.bin"),
    );
    spec.headers = vec![
        ("Cookie".to_string(), "sid=abc".to_string()),
        ("Authorization".to_string(), "Bearer xyz".to_string()),
        ("Referer".to_string(), "https://example.com/".to_string()),
        ("User-Agent".to_string(), "Mozilla/5.0".to_string()),
    ];
    db.insert(&spec).unwrap();

    let stored = &db.recent(1).unwrap()[0];
    assert_eq!(stored.url, "https://example.com/file.bin");
    assert!(stored.headers.is_empty());
}

#[test]
fn finish_clears_headers_on_existing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dwv.db");
    let db = Database::open(&path).unwrap();
    let id = db
        .insert(&DownloadSpec::new(
            "https://example.com/file.bin",
            dir.path().join("file.bin"),
        ))
        .unwrap();

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE downloads SET headers_json = ?1 WHERE id = ?2",
        rusqlite::params![r#"[["Cookie","sid=abc"]]"#, id],
    )
    .unwrap();

    db.finish(id, State::Completed).unwrap();
    assert!(db.recent(1).unwrap()[0].headers.is_empty());
}

#[tokio::test]
async fn queue_persists_lifecycle_to_db() {
    let body = test_body(8, 64 * 1024);
    let server = start_server(body.clone(), true, Duration::from_millis(10)).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("dwv.db")).unwrap();

    let (queue, mut events) = Queue::start(
        QueueConfig {
            max_parallel: 1,
            speed_limit: None,
        },
        Some(db.clone()),
    );
    let target = dir.path().join("liza.bin");
    queue.add(spec_for(&server, &target, 2)).await.unwrap();
    let pending_id = {
        let target = dir.path().join("pending.bin");
        queue.add(spec_for(&server, &target, 2)).await.unwrap()
    };
    queue.cancel(pending_id);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut finished = false;
    let mut cancelled = false;
    while !(finished && cancelled) {
        let ev = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("queue test timed out")
            .unwrap();
        match ev {
            QueueEvent::Finished { .. } => finished = true,
            QueueEvent::Cancelled { .. } => cancelled = true,
            QueueEvent::Failed { error, .. } => panic!("job failed: {error}"),
            _ => {}
        }
    }

    // the relay persists the final state slightly after the event
    let mut recent = Vec::new();
    for _ in 0..100 {
        recent = db.recent(10).unwrap();
        if recent.len() == 2
            && recent
                .iter()
                .all(|r| matches!(r.state, State::Completed | State::Cancelled))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(recent.len(), 2);
    let completed = recent
        .iter()
        .find(|r| r.state == State::Completed)
        .expect("completed row missing");
    assert_eq!(completed.downloaded_bytes, body.len() as u64);
    assert_eq!(completed.total_bytes, Some(body.len() as u64));
    let cancelled = recent
        .iter()
        .find(|r| r.state == State::Cancelled)
        .expect("cancelled row missing");
    assert_eq!(cancelled.downloaded_bytes, 0);
    assert!(db.load_pending().unwrap().is_empty());
}

#[test]
fn get_returns_one_row_or_none() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("dwv.db")).unwrap();
    let id = db
        .insert(&DownloadSpec::new(
            "https://example.com/file.bin",
            dir.path().join("file.bin"),
        ))
        .unwrap();

    let stored = db.get(id).unwrap().expect("row");
    assert_eq!(stored.id, id);
    assert_eq!(stored.url, "https://example.com/file.bin");
    assert!(db.get(id + 1).unwrap().is_none());
}

#[test]
fn set_target_rewrites_the_finished_path() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("dwv.db")).unwrap();
    let id = db
        .insert(&DownloadSpec::new(
            "magnet:?xt=urn:btih:abc",
            dir.path().join("folder"),
        ))
        .unwrap();
    let real = dir.path().join("single-file.mkv");
    db.set_target(id, &real).unwrap();
    assert_eq!(db.get(id).unwrap().expect("row").target, real);
}

#[test]
fn target_helpers_find_and_remove_history() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("dwv.db")).unwrap();
    let target = dir.path().join("same.bin");
    let _first = db
        .insert(&DownloadSpec::new(
            "https://example.com/a.bin",
            target.clone(),
        ))
        .unwrap();
    let second = db
        .insert(&DownloadSpec::new(
            "https://example.com/b.bin",
            target.clone(),
        ))
        .unwrap();

    assert_eq!(db.latest_id_by_target(&target).unwrap(), Some(second));
    db.delete_by_target(&target).unwrap();
    assert!(db.recent(10).unwrap().is_empty());
    assert_eq!(db.latest_id_by_target(&target).unwrap(), None);
}
