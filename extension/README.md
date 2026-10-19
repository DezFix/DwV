# DwV browser extensions

The extension talks to the running DwV app over `ws://127.0.0.1:41763`.
DwV must be running; the extension reconnects every 5 seconds while the browser is up.
Launch-on-demand (native messaging) comes with the installer in a later phase.

## Chrome (MV3)

1. Start DwV (`cargo run` from the repo, or the built `dwv.exe`).
2. Open `chrome://extensions`, enable **Developer mode**.
3. Click **Load unpacked** and select the `extension/chrome` folder.
4. Downloads of common file types (exe, zip, pdf, mp4, ...) are now intercepted
   and handed to DwV; the browser copy is cancelled. Links can also be sent
   through the right-click menu ("Download with DwV").

## Firefox (MV3)

1. Start DwV.
2. Open `about:debugging#/runtime/this-firefox`.
3. Click **Load Temporary Add-on...** and select `extension/firefox/manifest.json`.
4. Same behaviour as Chrome: interception plus the right-click menu.

Temporary add-ons disappear when Firefox closes; use
`about:config` -> `xpinstall.signatures.required = false` and
`web-ext build` for a persistent unsigned install during development.

## How it works

- `chrome.downloads.onCreated` fires for every download; the extension only
  grabs file types a download manager cares about (see `INTERCEPT_EXTENSIONS`).
- The URL, suggested filename and referrer go to DwV over the WebSocket.
- When DwV accepts, the browser download is cancelled and erased.
- The WS server only accepts browser-extension origins and native clients;
  web pages are rejected (403).
