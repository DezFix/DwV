use dwv_app::commands::{
    AppState, Category, DownloadContext, DownloadInfo, DownloadStatus, VIDEO_ID_BASE,
    add_download_inner, add_download_with_context, apply_event, category_of, download_limit_bps,
    filename_from_disposition, from_url, infer_kind, is_torrent_source, looks_like_torrent,
    parse_deep_link, remove_download_inner, resolve_filename, torrent_target_path,
    unique_download_name, upload_limit_bps,
};
use dwv_core::{DownloadSpec, QueueEvent, State as DbState};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn serve(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => break,
            };
            use std::io::{Read, Write};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        }
    });

    format!("http://127.0.0.1:{port}/clip.mp4")
}

#[test]
fn categorizes_by_extension() {
    assert_eq!(category_of("movie.mp4"), dwv_app::commands::Category::Video);
    assert_eq!(category_of("song.flac"), dwv_app::commands::Category::Audio);
    assert_eq!(
        category_of("book.pdf"),
        dwv_app::commands::Category::Document
    );
    assert_eq!(
        category_of("pack.zip"),
        dwv_app::commands::Category::Archive
    );
    assert_eq!(
        category_of("setup.exe"),
        dwv_app::commands::Category::Program
    );
    assert_eq!(category_of("photo.gif"), dwv_app::commands::Category::Image);
    assert_eq!(category_of("photo.jpg"), dwv_app::commands::Category::Image);
    assert_eq!(category_of("blob.bin"), dwv_app::commands::Category::Other);
}

#[test]
fn takes_name_from_url() {
    assert_eq!(from_url("http://host/path/file.zip?x=1"), "file.zip");
    assert_eq!(from_url("http://host/"), "download");
    assert_eq!(from_url("http://host/a%20b.txt"), "a b.txt");
}

#[test]
fn duplicate_names_get_a_counter() {
    let dir = std::env::temp_dir().join(format!("dwv-dup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("archive.zip"), b"first").unwrap();
    assert_eq!(unique_download_name(&dir, "archive.zip"), "archive (1).zip");
    std::fs::write(dir.join("archive (1).zip"), b"second").unwrap();
    assert_eq!(unique_download_name(&dir, "archive.zip"), "archive (2).zip");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn url_names_cannot_escape_the_download_folder() {
    assert_eq!(from_url("http://host/%2e%2e"), "download");
    assert_eq!(from_url("http://host/%2e%2e%2fevil.bin"), "_evil.bin");
    assert_eq!(from_url("http://host/a%5Cb.bin"), "a_b.bin");
    assert_eq!(from_url("http://host/%2e%2e%5Cevil.bin"), "_evil.bin");
}

#[test]
fn reads_rfc5987_disposition() {
    let value = "attachment; filename*=UTF-8''%D0%9E%D1%82%D1%87%D0%B5%D1%82.pdf";
    assert_eq!(
        filename_from_disposition(value),
        Some("Отчет.pdf".to_string())
    );
}

#[tokio::test]
async fn resolves_real_filename_from_server() {
    let url = serve(b"hello".to_vec());
    let name = resolve_filename(&url).await;
    assert_eq!(name, "clip.mp4");
}

#[tokio::test]
async fn downloads_file_into_the_download_folder() {
    let payload: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();
    let url = serve(payload.clone());

    let dir = std::env::temp_dir().join(format!("dwv-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let (state, mut events) = AppState::new(dir.join("settings.json"), dir.clone());

    let id = add_download_inner(&state, &url)
        .await
        .expect("add download");

    let expected = dir.join("Downloads").join("clip.mp4");

    let mut finished = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut downloads = state.downloads.lock().unwrap();

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Some(QueueEvent::Finished { id: done, path })) if done == id => {
                apply_event(&mut downloads, &QueueEvent::Finished { id: done, path });
                finished = true;
                break;
            }
            Ok(Some(QueueEvent::Failed { id: bad, error })) if bad == id => {
                panic!("download failed: {error}");
            }
            Ok(Some(event)) => {
                apply_event(&mut downloads, &event);
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    assert!(finished, "download did not finish in time");

    assert!(expected.exists(), "missing file at {}", expected.display());
    assert_eq!(
        std::fs::metadata(&expected).unwrap().len(),
        payload.len() as u64
    );

    let item = downloads.get(&id).expect("item in state");
    assert_eq!(item.status, DownloadStatus::Completed);
    assert_eq!(item.category, Category::Video);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn downloads_into_the_chosen_folder() {
    let payload = b"chosen".to_vec();
    let url = serve(payload.clone());

    let root = std::env::temp_dir().join(format!("dwv-chosen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let chosen = root.join("chosen");
    std::fs::create_dir_all(&chosen).unwrap();

    let (state, mut events) = AppState::new(root.join("settings.json"), root.clone());
    let id = add_download_with_context(
        &state,
        DownloadContext {
            url,
            dir: Some(chosen.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("add download");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!("download did not finish in time");
        }
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Some(QueueEvent::Finished { id: done, .. })) if done == id => break,
            Ok(Some(QueueEvent::Failed { id: bad, error })) if bad == id => {
                panic!("download failed: {error}");
            }
            Ok(None) => break,
            Ok(Some(_)) | Err(_) => {}
        }
    }

    assert!(
        chosen.join("clip.mp4").exists(),
        "file was not saved into the chosen folder"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn uses_the_configured_connections() {
    let dir = std::env::temp_dir().join(format!("dwv-conn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (state, _events) = AppState::new(dir.join("settings.json"), dir.clone());
    state.settings.lock().unwrap().connections_per_download = 4;

    let url = serve(b"x".to_vec());
    let id = add_download_inner(&state, &url)
        .await
        .expect("add download");

    let target = state
        .downloads
        .lock()
        .unwrap()
        .get(&id)
        .and_then(|item| item.file_path.clone())
        .expect("target path");
    let db = state.db.as_ref().expect("db");
    let row = db
        .latest_id_by_target(std::path::Path::new(&target))
        .unwrap()
        .expect("history row");
    let stored = db.get(row).unwrap().expect("stored row");
    assert_eq!(stored.connections, 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn removing_a_video_with_files_deletes_the_file() {
    let dir = std::env::temp_dir().join(format!("dwv-video-del-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (state, _events) = AppState::new(dir.join("settings.json"), dir.clone());

    let path = dir.join("clip.mp4");
    std::fs::write(&path, b"video").unwrap();
    let part = dir.join("clip.mp4.part");
    std::fs::write(&part, b"partial").unwrap();
    let id = VIDEO_ID_BASE + 1;
    let mut item = DownloadInfo::new(id as u64, path.clone());
    item.kind = "video".to_string();
    item.status = DownloadStatus::Completed;
    item.file_path = Some(path.to_string_lossy().to_string());
    state.downloads.lock().unwrap().insert(id as u64, item);

    remove_download_inner(&state, id, true)
        .await
        .expect("remove video");

    assert!(!path.exists(), "video file must be deleted");
    assert!(!part.exists(), "leftover .part must be deleted");
    assert!(state.downloads.lock().unwrap().get(&(id as u64)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn removing_only_the_item_keeps_the_video_file() {
    let dir = std::env::temp_dir().join(format!("dwv-video-keep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (state, _events) = AppState::new(dir.join("settings.json"), dir.clone());

    let path = dir.join("clip.mp4");
    std::fs::write(&path, b"video").unwrap();
    let id = VIDEO_ID_BASE + 2;
    let mut item = DownloadInfo::new(id as u64, path.clone());
    item.kind = "video".to_string();
    item.file_path = Some(path.to_string_lossy().to_string());
    state.downloads.lock().unwrap().insert(id as u64, item);

    remove_download_inner(&state, id, false)
        .await
        .expect("remove video");

    assert!(
        path.exists(),
        "the file must stay when only the item is removed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn removing_a_history_row_deletes_the_stored_file() {
    let dir = std::env::temp_dir().join(format!("dwv-history-del-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (state, _events) = AppState::new(dir.join("settings.json"), dir.clone());

    let path = dir.join("old-video.mp4");
    std::fs::write(&path, b"video").unwrap();
    let db = state.db.as_ref().expect("db").clone();
    let row = db
        .insert(&DownloadSpec::new(
            "https://youtu.be/abc".to_string(),
            path.clone(),
        ))
        .unwrap();
    db.finish(row, DbState::Completed).unwrap();

    remove_download_inner(&state, -row, true)
        .await
        .expect("remove history row");

    assert!(!path.exists(), "history file must be deleted");
    assert!(db.get(row).unwrap().is_none(), "history row must be gone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn detects_torrent_payloads_by_content() {
    let dir = std::env::temp_dir().join(format!("dwv-sniff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let torrent = dir.join("13562");
    std::fs::write(&torrent, b"d8:announce20:http://tracker4:infod4:name3:abc").unwrap();
    assert!(looks_like_torrent(&torrent));
    assert_eq!(torrent_target_path(&torrent), dir.join("13562.torrent"));

    let page = dir.join("page.html");
    std::fs::write(&page, b"<html>login</html>").unwrap();
    assert!(!looks_like_torrent(&page));

    let named = dir.join("movie.torrent");
    assert_eq!(torrent_target_path(&named), named);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parses_deep_link() {
    assert_eq!(
        parse_deep_link("dwv://add?url=https%3A%2F%2Fexample.com%2Fa.zip"),
        Some("https://example.com/a.zip".to_string())
    );
    assert_eq!(
        parse_deep_link("dwv://add?url=https://example.com/x"),
        Some("https://example.com/x".to_string())
    );
    assert_eq!(parse_deep_link("dwv://nope"), None);
}

#[test]
fn speed_limits_follow_the_contract() {
    assert_eq!(download_limit_bps(true, 2.0), Some(2_000_000));
    assert_eq!(download_limit_bps(true, 0.1), Some(500_000));
    assert_eq!(download_limit_bps(false, 2.0), None);
    assert_eq!(upload_limit_bps(0), None);
    assert_eq!(upload_limit_bps(128), Some(128_000));
}

#[test]
fn detects_torrent_sources_and_history_kinds() {
    assert!(is_torrent_source("magnet:?xt=urn:btih:abc"));
    assert!(is_torrent_source("https://example.com/a.torrent?x=1"));
    assert!(is_torrent_source("C:\\Downloads\\a.torrent"));
    assert!(!is_torrent_source("https://example.com/a.zip"));

    assert_eq!(infer_kind("magnet:?xt=urn:btih:abc", "a"), "torrent");
    assert_eq!(infer_kind("https://youtu.be/xyz", "clip.mp4"), "video");
    assert_eq!(
        infer_kind("https://example.com/clip.mp4", "clip.mp4"),
        "file"
    );
}
