# DwV browser extensions

The extension hands downloads to the DwV app through a local HTTP IPC at
`http://127.0.0.1:41763/`. Each hand-off is one `POST` with a JSON body and a
3 second timeout. DwV may be closed: the background opens
`dwv://add?url=...&type=...` in an inactive tab, closes it after a second, then
retries the `POST` for about 20 seconds. If DwV still does not answer, files
fall back to the browser download and videos report a failure. The extension
does not read or forward cookies, so authenticated downloads are not supported.

The Chrome and Firefox versions share the same `background.js`, `content.js`,
`popup.js` and options page.

Installation for Firefox and LibreWolf: see [INSTALL.md](INSTALL.md).

## Chrome (MV3)

1. Start DwV (`cargo run` from the repo, or the built `dwv.exe`).
2. Open `chrome://extensions` and enable **Developer mode**.
3. Click **Load unpacked** and select `extension/chrome`.
4. Click the toolbar icon to check the connection, toggle capture, list videos
   and hand them to DwV.

Chrome 116 or newer is required.

## Firefox (MV3)

1. Start DwV.
2. Open `about:debugging#/runtime/this-firefox`.
3. Click **Load Temporary Add-on...** and select
   `extension/firefox/manifest.json` or `extension/dist/dwv-1.2.8.xpi`.
4. Grant the requested host access if Firefox asks.

Temporary add-ons disappear when Firefox closes. For a persistent unsigned
install use LibreWolf with `about:config` ->
`xpinstall.signatures.required = false`, then install the XPI from
`about:addons`. Full steps are in [INSTALL.md](INSTALL.md).

Build the XPI with `node extension/build-xpi.mjs`; the file lands in
`extension/dist/`.

## What gets captured

- Settings live in `storage.local`: `enabled` (master switch),
  `captureFiles` and `captureVideo`, all defaulting to `true`. The popup
  writes each key separately. When `enabled` is off, automatic capture and
  hand-offs stop; the «Открыть DwV» button still works.
- Automatic file capture only considers `GET` requests to `http://` or
  `https://` URLs. A download is taken when the response has
  `Content-Disposition: attachment` or its filename/URL ends in one of:
  zip, rar, 7z, tar, gz, tgz, bz2, xz, iso, dmg, apk, exe, msi, pdf, doc,
  docx, xls, xlsx, ppt, pptx, mp3, flac, wav, mp4, mkv, avi, mov, torrent.
- Chrome uses `downloads.onDeterminingFilename`. A request header capture
  supplies `Referer`, `Origin`, `Accept` and the user agent.
- Firefox uses a blocking `webRequest.onHeadersReceived` listener for
  `main_frame` attachment responses; `sub_frame` responses are ignored.
  Downloads that do not reach that path are picked up by
  `downloads.onCreated`; a 30 second set prevents double handling.
- When DwV accepts a normal file, the browser copy is cancelled, removed from
  disk and erased from history. When it accepts a `.torrent` download, the
  browser copy is handled the same way and DwV receives
  `{"type":"torrent","source":...}`.
- Videos are collected by `content.js` (`<video>`, `<source>`, media links)
  and by a non-blocking `webRequest.onHeadersReceived` sniff for HLS, DASH,
  MP4, WebM, MKV and MOV responses, grouped per tab. Media responses are
  recorded, never cancelled. The popup merges both lists, shows title, kind,
  size and duration, and sends the chosen URL as
  `{"type":"video","url":...}` so DwV routes it to yt-dlp. «Смотреть»
  scrolls to the page player.
- The right-click menu item **Download with DwV** handles links and images.
  Magnet links and `.torrent` URLs are sent as torrents; other HTTP(S) links
  are sent as downloads with the page URL as referrer.
- The toolbar popup checks `/health` with a 2.5 second timeout and can launch
  DwV through the background. A red `!` badge marks a failed hand-off and
  clears on the next success.

## Limitations

- Offline hand-offs take up to about 20 seconds before falling back, because
  the background waits for DwV to start. The browser copy of a file is held
  back until then.
- Launching relies on the `dwv://` protocol being registered. DwV registers it
  on every launch; portable copies need the registry commands below.
- Automatic capture is global, not per site. The old `paused` key and
  per-site `site:<host>` keys are not used.
- Only `GET` downloads of known file types or attachment responses are
  captured. HTML pages, POST downloads and streaming media are left alone.
- Video discovery covers files and streams exposed by the page or by response
  headers. Site-specific extractors are not implemented; yt-dlp receives the
  URL and decides what it can download.
- Firefox blocks `ws://` from extension pages, which is why the transport uses
  `fetch` over HTTP.
- Host access is limited to `http://*/*` and `https://*/*`. Downloads from
  other schemes, including `file://` and browser pages, are not captured.
- Cookies are neither read nor forwarded, so authenticated downloads are not
  supported and can fail or return a login page.

## Tests

The background script runs against mocked Chrome and Firefox APIs plus a
mocked `fetch`:

```powershell
deno run --allow-read extension/test/background.test.mjs
```

If `deno` is not on `PATH`, use
`C:\Users\User\AppData\Roaming\DwV\bin\deno.exe`. The test checks accepted
archives, all transport failure modes, non-matching downloads, `.torrent`
hand-off, the context menu, the Firefox attachment, `sub_frame` and fallback
paths, `enabled` / `captureFiles` / `captureVideo`, video hand-off with launch
and retry, network media merging, the content response shape, and that both
background scripts stay identical.

## Deep link (dwv://)

The installer registers the dwv:// URL protocol automatically. For a
portable dwv.exe, register it manually (run once in cmd/PowerShell):

`
reg add "HKCU\Software\Classes\dwv" /ve /d "URL:dwv Protocol" /f
reg add "HKCU\Software\Classes\dwv" /v "URL Protocol" /d "" /f
reg add "HKCU\Software\Classes\dwv\shell\open\command" /ve /d "\"C:\Path\To\dwv.exe\" \"%1\"" /f
`

The app also re-registers the protocol for its own executable on every
launch. Link format: `dwv://add?url=<percent-encoded-url>&type=<file|video|torrent>`.
The extension opens `dwv://` for «Открыть DwV». If DwV is already running, a
second launch forwards the link to the running instance over
http://127.0.0.1:41763/ and exits.
