mod common;

use std::time::{Duration, Instant};

use common::{spec_for, start_server, test_body, with_suffix};
use dwv_core::{Download, Event, Outcome, SpeedLimiter};

#[tokio::test]
async fn single_stream_when_server_ignores_ranges() {
    let body = test_body(7, 256 * 1024);
    let server = start_server(body.clone(), false, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");

    let mut dl = Download::start(spec_for(&server, &target, 4));
    let mut started_connections = 0;
    let mut finished = false;
    while let Some(ev) = dl.events().recv().await {
        match ev {
            Event::Started { connections, .. } => started_connections = connections,
            Event::Finished { .. } => finished = true,
            Event::Progress { .. } => {}
        }
    }
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert!(finished);
    assert_eq!(started_connections, 1, "must fall back to a single stream");
    assert_eq!(std::fs::read(&target).unwrap(), body);
    assert!(!with_suffix(&target, ".part").exists());
    assert!(!with_suffix(&target, ".dwvmeta").exists());

    let ranges = server.ranges();
    assert_eq!(ranges.len(), 2, "probe + one plain request");
    assert_eq!(ranges[1], None, "download request must not use ranges");
}

#[tokio::test]
async fn segmented_download_matches_body() {
    let body = test_body(3, 1024 * 1024);
    let server = start_server(body.clone(), true, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");

    let mut dl = Download::start(spec_for(&server, &target, 4));
    let mut started_connections = 0;
    while let Some(ev) = dl.events().recv().await {
        if let Event::Started { connections, .. } = ev {
            started_connections = connections;
        }
    }
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert_eq!(started_connections, 4);
    assert_eq!(std::fs::read(&target).unwrap(), body);

    let ranges = server.ranges();
    let segment_requests = ranges
        .iter()
        .filter(|r| matches!(r.as_deref(), Some(s) if s != "bytes=0-0"))
        .count();
    assert_eq!(segment_requests, 4);
}

#[tokio::test]
async fn resume_after_cancel() {
    let body = test_body(9, 2 * 1024 * 1024);
    let server = start_server(body.clone(), true, Duration::from_millis(20)).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");
    let spec = spec_for(&server, &target, 2);

    let mut dl = Download::start(spec.clone());
    let controller = dl.controller();
    loop {
        match dl.events().recv().await.unwrap() {
            Event::Progress { downloaded, .. } if downloaded > 0 => break,
            Event::Finished { .. } => panic!("finished before cancel"),
            _ => {}
        }
    }
    controller.cancel();
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Cancelled);

    let part = with_suffix(&target, ".part");
    let meta_path = with_suffix(&target, ".dwvmeta");
    assert!(part.exists(), "part file must be kept for resume");
    assert!(meta_path.exists());
    // part file is preallocated to full size, so progress lives in the metadata
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    let done: u64 = meta["done"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert!(done > 0 && done < body.len() as u64);

    let before = server.ranges().len();
    let mut dl2 = Download::start(spec);
    let mut resumed = false;
    while let Some(ev) = dl2.events().recv().await {
        match ev {
            Event::Started { resumed: r, .. } => resumed = r,
            Event::Finished { .. } => break,
            _ => {}
        }
    }
    let outcome = dl2.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert!(resumed, "second run must resume, not restart");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        body,
        "resumed file is corrupt"
    );

    let ranges = server.ranges();
    let resumed_from_offset = ranges[before..].iter().any(|r| {
        r.as_deref()
            .and_then(|s| s.strip_prefix("bytes="))
            .and_then(|s| s.split('-').next())
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|start| start > 0)
    });
    assert!(resumed_from_offset, "no ranged request past offset 0");
}

#[tokio::test]
async fn speed_limit_slows_download() {
    let body = test_body(4, 256 * 1024);
    let server = start_server(body.clone(), false, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");

    let mut spec = spec_for(&server, &target, 1);
    spec.limiter = Some(SpeedLimiter::new(Some(256 * 1024)));

    let started = Instant::now();
    let mut dl = Download::start(spec);
    while dl.events().recv().await.is_some() {}
    let outcome = dl.join().await.unwrap();
    let elapsed = started.elapsed();

    assert_eq!(outcome, Outcome::Completed);
    assert_eq!(std::fs::read(&target).unwrap(), body);
    // 256 KiB at 256 KiB/s takes about one second
    assert!(
        elapsed >= Duration::from_millis(700),
        "download was not throttled: {elapsed:?}"
    );
}
