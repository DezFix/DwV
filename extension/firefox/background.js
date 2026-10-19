const WS_URL = "ws://127.0.0.1:41763";

// Chrome downloads everything eagerly; only grab file types a download manager cares about.
const INTERCEPT_EXTENSIONS = new Set([
  "exe", "msi", "zip", "rar", "7z", "tar", "gz", "xz", "iso", "dmg", "apk",
  "pdf", "mp3", "flac", "wav", "mp4", "mkv", "avi", "mov", "doc", "docx",
  "xls", "xlsx", "ppt", "pptx", "torrent",
]);

let socket = null;
let connected = false;
let nextId = 1;
const pending = new Map();

function connect() {
  if (socket) return;
  const ws = new WebSocket(WS_URL);
  socket = ws;
  ws.onopen = () => {
    connected = true;
  };
  ws.onmessage = (event) => {
    let reply;
    try {
      reply = JSON.parse(event.data);
    } catch {
      return;
    }
    const resolve = pending.get(reply.id);
    if (resolve) {
      pending.delete(reply.id);
      resolve(reply);
    }
  };
  ws.onclose = () => {
    connected = false;
    socket = null;
    for (const resolve of pending.values()) resolve({ ok: false, error: "disconnected" });
    pending.clear();
    setTimeout(connect, 5000);
  };
  ws.onerror = () => {};
}

function send(request) {
  return new Promise((resolve) => {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      resolve({ ok: false, error: "DwV is not running" });
      return;
    }
    const id = nextId++;
    request.id = id;
    pending.set(id, resolve);
    socket.send(JSON.stringify(request));
    setTimeout(() => {
      if (pending.delete(id)) resolve({ ok: false, error: "timeout" });
    }, 3000);
  });
}

// the service worker may be asleep right when a download starts
async function ensureConnected(timeoutMs = 500) {
  if (connected) return true;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 50));
    if (connected) return true;
  }
  return false;
}

function fileExtension(url, filename) {
  const name = filename || url.split(/[?#]/)[0];
  const match = name.match(/\.([a-z0-9]+)$/i);
  return match ? match[1].toLowerCase() : "";
}

function basename(path) {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1];
}

chrome.contextMenus.create({
  id: "dwv-download",
  title: "Download with DwV",
  contexts: ["link"],
});

chrome.contextMenus.onClicked.addListener(async (info) => {
  if (info.menuItemId !== "dwv-download" || !info.linkUrl) return;
  if (!(await ensureConnected())) return;
  await send({ type: "add", url: info.linkUrl, referrer: info.pageUrl });
});

chrome.downloads.onCreated.addListener(async (item) => {
  const url = item.finalUrl || item.url;
  if (!/^https?:/i.test(url)) return;
  if (!INTERCEPT_EXTENSIONS.has(fileExtension(url, item.filename))) return;
  if (!(await ensureConnected())) return;

  const reply = await send({
    type: "add",
    url,
    filename: item.filename ? basename(item.filename) : undefined,
    referrer: item.referrer || undefined,
  });
  if (reply.ok) {
    // the download continues in DwV, remove the browser's copy
    chrome.downloads.cancel(item.id).then(() => chrome.downloads.erase({ id: item.id }));
  }
});

connect();
setInterval(() => send({ type: "ping" }), 20000);
