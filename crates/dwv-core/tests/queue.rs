mod common;

use std::time::Duration;

use common::{spec_for, start_server, test_body};
use dwv_core::queue::{Queue, QueueConfig, QueueEvent};

#[tokio::test]
async fn queue_limits_parallel_jobs() {
    let body = test_body(5, 64 * 1024);
    let server = start_server(body, true, Duration::from_millis(40)).await;
    let dir = tempfile::tempdir().unwrap();

    let (queue, mut events) = Queue::start(
        QueueConfig {
            max_parallel: 2,
            speed_limit: None,
        },
        None,
    );
    for i in 0..3 {
        let target = dir.path().join(format!("file{i}.bin"));
        queue.add(spec_for(&server, &target, 1)).await.unwrap();
    }

    let mut active = 0usize;
    let mut max_active = 0usize;
    let mut finished = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while finished < 3 {
        let ev = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("queue test timed out")
            .unwrap();
        match ev {
            QueueEvent::Started { .. } => {
                active += 1;
                max_active = max_active.max(active);
            }
            QueueEvent::Finished { .. } => {
                active -= 1;
                finished += 1;
            }
            QueueEvent::Failed { error, .. } => panic!("job failed: {error}"),
            QueueEvent::Cancelled { .. } => panic!("unexpected cancel"),
            QueueEvent::Paused { .. } => panic!("unexpected pause"),
            QueueEvent::Progress { .. } => {}
        }
    }
    assert!(max_active <= 2, "max_parallel violated: {max_active}");
    assert_eq!(max_active, 2, "jobs did not overlap");
    for i in 0..3 {
        assert!(dir.path().join(format!("file{i}.bin")).exists());
    }
}

#[tokio::test]
async fn cancel_pending_job_skips_it() {
    let body = test_body(6, 64 * 1024);
    let server = start_server(body, true, Duration::from_millis(30)).await;
    let dir = tempfile::tempdir().unwrap();

    let (queue, mut events) = Queue::start(
        QueueConfig {
            max_parallel: 1,
            speed_limit: None,
        },
        None,
    );
    let target1 = dir.path().join("a.bin");
    let target2 = dir.path().join("b.bin");
    let id1 = queue.add(spec_for(&server, &target1, 1)).await.unwrap();
    let id2 = queue.add(spec_for(&server, &target2, 1)).await.unwrap();

    loop {
        if let QueueEvent::Started { id, .. } = events.recv().await.unwrap() {
            assert_eq!(id, id1);
            break;
        }
    }
    queue.cancel(id2);

    let mut saw_cancelled = false;
    let mut saw_started_second = false;
    let mut done_first = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !(done_first && saw_cancelled) {
        let ev = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("queue test timed out")
            .unwrap();
        match ev {
            QueueEvent::Started { id, .. } if id == id2 => saw_started_second = true,
            QueueEvent::Cancelled { id } if id == id2 => saw_cancelled = true,
            QueueEvent::Finished { id, .. } if id == id1 => done_first = true,
            QueueEvent::Failed { error, .. } => panic!("job failed: {error}"),
            _ => {}
        }
    }
    assert!(!saw_started_second, "cancelled job must not start");
    assert!(target1.exists());
    assert!(!target2.exists());
}

#[tokio::test]
async fn pause_keeps_part_and_resume_finishes() {
    let body = test_body(9, 512 * 1024);
    let server = start_server(body.clone(), true, Duration::from_millis(25)).await;
    let dir = tempfile::tempdir().unwrap();

    let (queue, mut events) = Queue::start(
        QueueConfig {
            max_parallel: 1,
            speed_limit: None,
        },
        None,
    );
    let target = dir.path().join("resume.bin");
    let id = queue.add(spec_for(&server, &target, 1)).await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let ev = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("pause test timed out")
            .unwrap();
        if let QueueEvent::Progress { downloaded, .. } = ev
            && downloaded > 0
        {
            break;
        }
    }
    queue.pause(id);

    loop {
        let ev = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("pause event timed out")
            .unwrap();
        match ev {
            QueueEvent::Paused { id: pid } => {
                assert_eq!(pid, id);
                break;
            }
            QueueEvent::Failed { error, .. } => panic!("failed during pause: {error}"),
            QueueEvent::Cancelled { .. } => panic!("pause was reported as cancel"),
            _ => {}
        }
    }
    let part = dir.path().join("resume.bin.part");
    assert!(part.exists(), "part file must survive a pause");
    assert!(!target.exists(), "paused download must not be finalized");

    queue.resume(id);
    loop {
        let ev = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("resume timed out")
            .unwrap();
        match ev {
            QueueEvent::Finished { id: fid, .. } => {
                assert_eq!(fid, id);
                break;
            }
            QueueEvent::Failed { error, .. } => panic!("failed during resume: {error}"),
            QueueEvent::Paused { .. } => panic!("unexpected second pause"),
            _ => {}
        }
    }
    assert_eq!(std::fs::read(&target).unwrap(), body);
}
