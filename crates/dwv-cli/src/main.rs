use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use dwv_core::{Download, DownloadSpec, Event, Outcome, Queue, QueueConfig, QueueEvent};

struct Args {
    urls: Vec<String>,
    out: Option<PathBuf>,
    connections: usize,
    parallel: usize,
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("video") {
        return run_video_mode(&raw[1..]);
    }
    if raw.first().map(String::as_str) == Some("torrent") {
        return run_torrent_mode(&raw[1..]);
    }
    let args = match parse_args(&raw) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            eprintln!(
                "usage: dwv-cli <url> [<url>...] [-o <path>] [-n <connections>] [-j <parallel>]"
            );
            return ExitCode::from(2);
        }
    };
    if args.out.is_some() && args.urls.len() > 1 {
        eprintln!("-o is only supported with a single url");
        return ExitCode::from(2);
    }
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(run(args))
}

async fn run(args: Args) -> ExitCode {
    if args.urls.len() == 1 {
        let url = &args.urls[0];
        let target = args.out.unwrap_or_else(|| default_filename(url));
        run_single(url.clone(), target, args.connections).await
    } else {
        run_many(args.urls, args.connections, args.parallel).await
    }
}

fn run_video_mode(args: &[String]) -> ExitCode {
    let mut url = None;
    let mut dir = PathBuf::from(".");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-d" | "--dir" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    eprintln!("-d needs a value");
                    return ExitCode::from(2);
                };
                dir = PathBuf::from(value);
            }
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {other}");
                return ExitCode::from(2);
            }
            other => {
                if url.is_none() {
                    url = Some(other.to_string());
                }
            }
        }
        i += 1;
    }
    let Some(url) = url else {
        eprintln!("usage: dwv-cli video <url> [-d <dir>]");
        return ExitCode::from(2);
    };
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(async move {
        let result = dwv_media::download_video(&url, &dir, |p| {
            print!(
                "\r{} / {}  {}/s          ",
                fmt_size(Some(p.downloaded)),
                fmt_size(p.total),
                fmt_size(p.speed_bps)
            );
            let _ = std::io::stdout().flush();
        })
        .await;
        match result {
            Ok(Some(path)) => {
                println!("\rsaved: {}", path.display());
                ExitCode::SUCCESS
            }
            Ok(None) => {
                println!("\rdone");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("\nvideo download failed: {e}");
                ExitCode::FAILURE
            }
        }
    })
}

fn run_torrent_mode(args: &[String]) -> ExitCode {
    let mut source = None;
    let mut dir = PathBuf::from(".");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-d" | "--dir" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    eprintln!("-d needs a value");
                    return ExitCode::from(2);
                };
                dir = PathBuf::from(value);
            }
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {other}");
                return ExitCode::from(2);
            }
            other => {
                if source.is_none() {
                    source = Some(other.to_string());
                }
            }
        }
        i += 1;
    }
    let Some(source) = source else {
        eprintln!("usage: dwv-cli torrent <magnet|url|file.torrent> [-d <dir>]");
        return ExitCode::from(2);
    };
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(async move {
        let result = dwv_torrent::download_torrent(&source, &dir, |p| {
            let total = if p.total == 0 { None } else { Some(p.total) };
            print!(
                "\r{} / {}  {}/s          ",
                fmt_size(Some(p.downloaded)),
                fmt_size(total),
                fmt_size(Some(p.down_bps))
            );
            let _ = std::io::stdout().flush();
        })
        .await;
        match result {
            Ok(path) => {
                println!("\rsaved: {}", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("\ntorrent download failed: {e}");
                ExitCode::FAILURE
            }
        }
    })
}

async fn run_single(url: String, target: PathBuf, connections: usize) -> ExitCode {
    let mut spec = DownloadSpec::new(url, target);
    spec.connections = connections;

    let mut download = Download::start(spec);
    let controller = download.controller();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("cancelling, partial file kept for resume");
            controller.cancel();
        }
    });

    let events = download.events();
    while let Some(event) = events.recv().await {
        match event {
            Event::Started {
                total,
                connections,
                resumed,
            } => println!(
                "size: {}  connections: {connections}  resumed: {resumed}",
                fmt_size(total)
            ),
            Event::Progress {
                downloaded,
                total,
                speed_bps,
            } => {
                print!(
                    "\r{} / {}  {}/s          ",
                    fmt_size(Some(downloaded)),
                    fmt_size(total),
                    fmt_size(Some(speed_bps))
                );
                let _ = std::io::stdout().flush();
            }
            Event::Finished { path } => println!("\rdownloaded: {}", path.display()),
        }
    }

    match download.join().await {
        Ok(Outcome::Completed) => ExitCode::SUCCESS,
        Ok(Outcome::Cancelled) => ExitCode::from(3),
        Err(e) => {
            eprintln!("\nfailed: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run_many(urls: Vec<String>, connections: usize, parallel: usize) -> ExitCode {
    let (queue, mut events) = Queue::start(
        QueueConfig {
            max_parallel: parallel,
            speed_limit: None,
        },
        None,
    );

    let mut queue_ids: Vec<u64> = Vec::new();
    for url in &urls {
        let target = default_filename(url);
        let mut spec = DownloadSpec::new(url.clone(), &target);
        spec.connections = connections;
        match queue.add(spec).await {
            Ok(id) => queue_ids.push(id),
            Err(e) => {
                eprintln!("failed to queue {url}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    let cancel_queue = queue.clone();
    let cancel_ids = queue_ids.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("cancelling all downloads");
            for id in cancel_ids {
                cancel_queue.cancel(id);
            }
        }
    });

    let total = queue_ids.len();
    let mut done = 0usize;
    let mut failed = 0usize;
    let mut last_print: HashMap<u64, Instant> = HashMap::new();
    while done + failed < total {
        let Some(event) = events.recv().await else {
            break;
        };
        match event {
            QueueEvent::Started {
                id,
                path,
                total,
                connections,
                resumed,
            } => {
                println!(
                    "[{id}] {}: {}  connections: {connections}  resumed: {resumed}",
                    path.display(),
                    fmt_size(total)
                );
            }
            QueueEvent::Progress {
                id,
                downloaded,
                total,
                speed_bps,
            } => {
                let now = Instant::now();
                let due = last_print
                    .get(&id)
                    .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(1));
                if due {
                    last_print.insert(id, now);
                    println!(
                        "[{id}] {} / {}  {}/s",
                        fmt_size(Some(downloaded)),
                        fmt_size(total),
                        fmt_size(Some(speed_bps))
                    );
                }
            }
            QueueEvent::Finished { id, path } => {
                done += 1;
                println!("[{id}] done: {}", path.display());
            }
            QueueEvent::Failed { id, error } => {
                failed += 1;
                eprintln!("[{id}] failed: {error}");
            }
            QueueEvent::Cancelled { id } => {
                failed += 1;
                eprintln!("[{id}] cancelled");
            }
        }
    }

    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut urls = Vec::new();
    let mut out = None;
    let mut connections = 8usize;
    let mut parallel = 3usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                let value = args.get(i).ok_or("-o needs a value")?;
                out = Some(PathBuf::from(value));
            }
            "-n" | "--connections" => {
                i += 1;
                let value = args.get(i).ok_or("-n needs a value")?;
                connections = value
                    .parse()
                    .map_err(|_| format!("-n must be a number, got {value}"))?;
            }
            "-j" | "--parallel" => {
                i += 1;
                let value = args.get(i).ok_or("-j needs a value")?;
                parallel = value
                    .parse()
                    .map_err(|_| format!("-j must be a number, got {value}"))?;
            }
            other if other.starts_with('-') => return Err(format!("unknown flag: {other}")),
            other => urls.push(other.to_string()),
        }
        i += 1;
    }
    if urls.is_empty() {
        return Err("at least one url is required".to_string());
    }
    if connections == 0 {
        return Err("-n must be at least 1".to_string());
    }
    if parallel == 0 {
        return Err("-j must be at least 1".to_string());
    }
    Ok(Args {
        urls,
        out,
        connections,
        parallel,
    })
}

fn default_filename(url: &str) -> PathBuf {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = no_query.rsplit('/').next().unwrap_or("").trim();
    let name = if last.is_empty() { "download" } else { last };
    // shortcut: percent-encoding left as-is, decode when it becomes a problem
    PathBuf::from(name)
}

fn fmt_size(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return "?".to_string();
    };
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
