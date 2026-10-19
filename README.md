# DwV

Download manager written in Rust with file interception from the browser.

- Segmented downloads with resume (HTTP/HTTPS)
- Chrome and Firefox extensions: download interception, context menu, media gallery for the current page
- Video from any site via yt-dlp (updates itself on start)
- BitTorrent via librqbit
- Slint UI, tray icon, Windows installer

Status: early development. The download engine (segments, queue, speed limit, SQLite history) and a basic GUI are in place; browser extensions, video and torrents are next.

## Build

```powershell
cargo build --release
```

Application binary: `target/release/dwv.exe`

## CLI (current download engine)

```powershell
cargo run -p dwv-cli -- https://example.com/file.zip -o file.zip -n 8
cargo run -p dwv-cli -- <url1> <url2> <url3> -j 3
```

`-n` is connections per file, `-j` is parallel downloads in the queue. Ctrl+C stops the download; `.part` and `.dwvmeta` files are kept, and running the same command again resumes from where it stopped.

## Layout

```
crates/
  dwv-core/     download engine (HTTP segments, queue, resume, SQLite history)
  dwv-media/    yt-dlp / ffmpeg
  dwv-torrent/  librqbit
  dwv-ipc/      native messaging for extensions + local WS API
  dwv-app/      GUI (Slint)
  dwv-cli/      headless build for tests and scripts
ui/             Slint files
extension/      Chrome/Firefox extensions
```

## License

GPL-3.0-or-later, see [LICENSE](LICENSE)

## Support

https://ko-fi.com/dezfix
