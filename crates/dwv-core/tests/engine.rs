mod common;

use std::time::{Duration, Instant};

use common::{
    spec_for, start_failing_server, start_hanging_server, start_limited_server,
    start_ranged_403_server, start_server, test_body, with_suffix,
};
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
    // the part file is not preallocated, so progress lives in the metadata
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
async fn cancel_interrupts_a_stuck_probe() {
    let server = start_hanging_server().await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("stuck.bin");

    let dl = Download::start(spec_for(&server, &target, 2));
    let controller = dl.controller();
    tokio::time::sleep(Duration::from_millis(100)).await;
    controller.cancel();

    let outcome = tokio::time::timeout(Duration::from_secs(5), dl.join())
        .await
        .expect("a stuck server must not hang the download")
        .unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
}

#[tokio::test]
async fn retries_transient_server_errors() {
    let body = test_body(11, 512 * 1024);
    let server = start_failing_server(body.clone(), 2).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("flaky.bin");

    let mut dl = Download::start(spec_for(&server, &target, 2));
    while dl.events().recv().await.is_some() {}
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert_eq!(std::fs::read(&target).unwrap(), body);
}

#[tokio::test]
async fn retries_forbidden_ranges_from_connection_limits() {
    let body = test_body(13, 512 * 1024);
    // pixeldrain and mixdrop answer 403 when too many ranges are open at once
    let server = start_limited_server(body.clone(), 2).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("limited.bin");

    let mut dl = Download::start(spec_for(&server, &target, 2));
    while dl.events().recv().await.is_some() {}
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert_eq!(std::fs::read(&target).unwrap(), body);
}

#[tokio::test]
async fn falls_back_to_a_single_stream_when_ranges_are_forbidden() {
    let body = test_body(14, 512 * 1024);
    let server = start_ranged_403_server(body.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("capped.bin");

    let mut dl = Download::start(spec_for(&server, &target, 8));
    while dl.events().recv().await.is_some() {}
    let outcome = dl.join().await.unwrap();
    assert_eq!(outcome, Outcome::Completed);
    assert_eq!(std::fs::read(&target).unwrap(), body);

    let ranges = server.ranges();
    assert!(
        ranges.iter().any(Option::is_none),
        "the transfer must end as a plain request without ranges"
    );
}

#[tokio::test]
async fn rename_failure_is_reported() {
    let body = test_body(12, 64 * 1024);
    let server = start_server(body, true, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("blocked.bin");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"x").unwrap();

    let mut dl = Download::start(spec_for(&server, &target, 2));
    let mut finished = false;
    while let Some(event) = dl.events().recv().await {
        if matches!(event, Event::Finished { .. }) {
            finished = true;
        }
    }
    assert!(
        dl.join().await.is_err(),
        "a blocked target must surface as an error"
    );
    assert!(!finished, "a blocked target must not report success");
    assert!(with_suffix(&target, ".part").exists());
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
