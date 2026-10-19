# Roadmap

## Phase 0 - Skeleton (done)

- Workspace, CI (fmt, clippy, test), GPL-3.0 license, README

## Phase 1 - Download engine (done)

- Segmented HTTP/HTTPS downloads, resume via `.part` + `.dwvmeta`
- Queue with parallel limit, cancel, global speed limit
- SQLite history and settings
- `dwv-cli`

## Phase 2 - Basic GUI (done)

- Slint window: URL field, download list with progress and speed, cancel
- History from SQLite, save folder picker

## Phase 3 - Browser integration (in progress)

- [x] Chrome (MV3) extension in `extension/chrome`: intercepts common file types, right-click menu
- [x] Firefox (MV3) extension in `extension/firefox`
- [x] Local WebSocket API `ws://127.0.0.1:41763` in `dwv-ipc` (origin-checked, only extension and native clients)
- [ ] Extension options: choose intercepted file types, notifications
- Native messaging host (launch DwV when closed) moved to Phase 7: it needs a stable
  extension ID, which means a signing key in the manifests and registration done by the installer

## Phase 4 - Video (yt-dlp) (in progress)

- [x] `dwv-media`: downloads yt-dlp and an LGPL ffmpeg build into `%APPDATA%\DwV\bin` on first use
- [x] `dwv-cli video <url> [-d <dir>]` with live progress, video+audio merged by ffmpeg
- [ ] GUI: video button and per-row detection
- [ ] Media gallery for the current page in the extension (list formats, pick quality)
- [ ] Format picker (resolution), playlist confirmation
- [ ] Bundle a JS runtime (deno) so YouTube works without Node installed
- [ ] Checksum verification for tool updates

## Phase 5 - Torrents (in progress)

- [x] `dwv-torrent` on librqbit: magnet links, `.torrent` URLs and local files; `dwv-cli torrent <source> [-d <dir>]` with progress
- [ ] GUI: torrent source in the add dialog, drag and drop of `.torrent` files
- [ ] File selection inside a torrent, upload/seed limits
- [ ] Resume torrents across restarts

## Phase 6 - Main window, FDM-inspired (not a clone)

Reference: screenshot of Free Download Manager 6 dark UI. Same information density, own layout and identity.

- [x] Pause/resume in the engine and queue (paused downloads keep their `.part` and resume by id)
- [ ] Toolbar: `+ Add download` (accent), search box, filter tabs (All / Active / Completed / Video / Torrents), overflow menu
- [ ] Table view: name, size, status, downloaded, speed, ETA; per-row pause/resume/cancel; sortable columns; multi-select
- [ ] Empty state: dashed drop zone, "Drag URL or torrent file"
- [ ] Drag and drop: links, text, `.torrent` files onto the window
- [ ] Status bar: total down/up speed, active count, support link
- [ ] Settings page: default folder, connections, speed limit, theme
- [ ] Tray icon: close to tray, notifications on finish/failure

## Phase 7 - Packaging

- NSIS installer, bundle yt-dlp + ffmpeg, auto-update, code signing, store submission (MS Store, winget)
