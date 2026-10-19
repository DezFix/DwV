# DwV browser extensions

The extension talks to the running DwV app over `ws://127.0.0.1:41763`.

## Chrome (MV3)

1. Start DwV (`cargo run` from the repo, or the built `dwv.exe`).
2. Open `chrome://extensions`, enable **Developer mode**.
3. Click **Load unpacked** and select the `extension/chrome` folder.
4. Downloads of common file types (exe, zip, pdf, mp4, ...) are now intercepted
   and handed to DwV; the browser copy is cancelled. Links can also be sent
   through the right-click menu ("Download with DwV").

If the extension shows nothing in `chrome://extensions` console it means DwV is
not running; the extension reconnects every 5 seconds while the browser is up.
