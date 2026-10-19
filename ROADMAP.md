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

## Phase 3 - Browser integration

- Chrome (MV3) and Firefox extensions: download interception, context menu, media list on page
- Local WebSocket API + native messaging host registration for wake-up

## Phase 4 - Video (yt-dlp)

- Sidecar yt-dlp with self-update, format picker, ffmpeg bundling for merge
- Site support: any yt-dlp extractor

## Phase 5 - Torrents

- librqbit engine: `.torrent` and magnet links, file selection, seeding limits

## Phase 6 - Main window, FDM-inspired (not a clone)

Reference: screenshot of Free Download Manager 6 dark UI. Same information density, own layout and identity.

- Toolbar: `+ Add download` (accent), search box, filter tabs (All / Active / Completed / Video / Torrents), overflow menu
- Table view: name, size, status, downloaded, speed, ETA; per-row pause/resume/cancel; sortable columns; multi-select
- Empty state: dashed drop zone, "Drag URL or torrent file"
- Drag and drop: links, text, `.torrent` files onto the window
- Status bar: total down/up speed, active count, support link
- Pause/resume in the engine (currently cancel keeps the `.part`; add explicit pause and resume-by-id)
- Settings page: default folder, connections, speed limit, theme
- Tray icon: close to tray, notifications on finish/failure

## Phase 7 - Packaging

- NSIS installer, bundle yt-dlp + ffmpeg, auto-update, code signing, store submission (MS Store, winget)
