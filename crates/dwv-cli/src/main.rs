use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use dwv_core::{Download, DownloadSpec, Event, Outcome};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (url, out, connections) = match parse_args(&args) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            eprintln!("usage: dwv-cli <url> [-o <path>] [-n <connections>]");
            return ExitCode::from(2);
        }
    };
    let target = out.unwrap_or_else(|| default_filename(&url));
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(run(url, target, connections))
}

fn parse_args(args: &[String]) -> Result<(String, Option<PathBuf>, usize), String> {
    let mut url = None;
    let mut out = None;
    let mut connections = 8usize;
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
            other if other.starts_with('-') => return Err(format!("unknown flag: {other}")),
            other => {
                if url.is_some() {
                    return Err("only one url is supported".to_string());
                }
                url = Some(other.to_string());
            }
        }
        i += 1;
    }
    let url = url.ok_or("url is required")?;
    if connections == 0 {
        return Err("-n must be at least 1".to_string());
    }
    Ok((url, out, connections))
}

fn default_filename(url: &str) -> PathBuf {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = no_query.rsplit('/').next().unwrap_or("").trim();
    let name = if last.is_empty() { "download" } else { last };
    // shortcut: percent-encoding left as-is, decode when it becomes a problem
    PathBuf::from(name)
}

async fn run(url: String, target: PathBuf, connections: usize) -> ExitCode {
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
