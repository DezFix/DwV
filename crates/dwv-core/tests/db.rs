mod common;

use std::time::Duration;

use common::{spec_for, start_server, test_body};
use dwv_core::db::{Database, State};
use dwv_core::{DownloadSpec, Queue, QueueConfig, QueueEvent};

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
    assert_eq!(stored.headers, spec.headers);

    let restored = stored.to_spec();
    assert_eq!(restored.url, spec.url);
    assert_eq!(restored.target, spec.target);
    assert_eq!(restored.headers, spec.headers);

    db.finish(id, State::Completed).unwrap();
    assert!(db.load_pending().unwrap().is_empty());
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
