const IPC_URL = "http://127.0.0.1:41763/";
const TIMEOUT_MS = 3000;
const HANDOFF_TTL_MS = 30000;
const MAX_TRACKED = 200;
const RETRY_MS = 500;
const RETRY_WINDOW_MS = 20000;
const TAB_CLOSE_MS = 1000;
const MAX_MEDIA_PER_TAB = 50;
const MAX_VIDEOS_PER_TAB = 100;

const FILE_EXTENSIONS = new Set([
  "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "iso", "dmg", "apk",
  "exe", "msi", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "mp3",
  "flac", "wav", "mp4", "mkv", "avi", "mov", "torrent",
]);

const MEDIA_EXTENSIONS = /\.(m3u8|mpd|mp4|m4v|webm|mkv|mov)(?:[?#]|$)/i;
const STREAM_EXTENSIONS = /\.(m3u8|mpd)(?:[?#]|$)/i;
const MEDIA_MIME = /^(?:video\/(?:mp4|webm|x-matroska|quicktime|3gpp)|application\/(?:vnd\.apple\.mpegurl|x-mpegurl|dash\+xml|mpd))$/i;

const DEFAULT_SETTINGS = { enabled: true, captureFiles: true, captureVideo: true };
const settings = { ...DEFAULT_SETTINGS };

const api = globalThis.browser ?? globalThis.chrome;
const IS_CHROME = Boolean(api.downloads?.onDeterminingFilename);

const contexts = new Map();
const responses = new Map();
const handedOff = new Map();
const mediaByTab = new Map();
const videosByTab = new Map();

function remember(map, key, value) {
  map.set(key, value);
  if (map.size > MAX_TRACKED) {
    map.delete(map.keys().next().value);
  }
}

function rememberForTab(map, tabId, key, value, limit) {
  let items = map.get(tabId);
  if (!items) {
    items = new Map();
    map.set(tabId, items);
  }
  items.delete(key);
  items.set(key, value);
  if (items.size > limit) {
    items.delete(items.keys().next().value);
  }
  if (map.size > MAX_TRACKED) {
    const oldest = map.keys().next().value;
    if (oldest !== tabId) map.delete(oldest);
  }
}

function captureOn(key) {
  return settings.enabled && settings[key] !== false;
}

async function loadSettings() {
  try {
    const state = await api.storage.local.get(DEFAULT_SETTINGS);
    for (const key of Object.keys(DEFAULT_SETTINGS)) {
      settings[key] = state[key] !== false;
    }
  } catch {}
}

const settingsReady = loadSettings();

api.storage.onChanged?.addListener((changes, area) => {
  if (area && area !== "local") return;
  for (const key of Object.keys(DEFAULT_SETTINGS)) {
    if (changes[key]) settings[key] = changes[key].newValue !== false;
  }
});

function isHttp(url) {
  return /^https?:\/\//i.test(url || "");
}

function isTorrent(url, filename) {
  return /^magnet:/i.test(url || "")
    || /\.torrent(?:[?#]|$)/i.test(url || "")
    || fileExtension(url, filename) === "torrent";
}

function basename(path) {
  const parts = (path || "").split(/[\\/]/);
  return parts[parts.length - 1];
}

function fileExtension(url, filename) {
  const name = (filename || url || "").split(/[?#]/)[0];
  const match = name.match(/\.([a-z0-9]+)$/i);
  return match ? match[1].toLowerCase() : "";
}

function headerValue(headers, wanted) {
  if (!headers) return "";
  for (const header of headers) {
    if (header.name && String(header.name).toLowerCase() === wanted) {
      return header.value || "";
    }
  }
  return "";
}

async function send(message) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  try {
    const response = await fetch(IPC_URL, {
      method: "POST",
      headers: { "Content-Type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(message),
      signal: controller.signal,
    });
    if (response.status !== 200) {
      return { ok: false, error: `HTTP ${response.status}` };
    }
    let reply;
    try {
      reply = await response.json();
    } catch {
      return { ok: false, error: "invalid response" };
    }
    if (!reply || reply.ok !== true) {
      return { ok: false, error: reply?.error || "declined" };
    }
    return { ok: true };
  } catch (error) {
    return { ok: false, offline: true, error: error?.message || "DwV is not running" };
  } finally {
    clearTimeout(timer);
  }
}

async function setBadge(failed) {
  try {
    if (failed && api.action?.setBadgeBackgroundColor) {
      await api.action.setBadgeBackgroundColor({ color: "#d93025" });
    }
    if (api.action?.setBadgeText) {
      await api.action.setBadgeText({ text: failed ? "!" : "" });
    }
  } catch {}
}

function deepLink() {
  // Only wake the app. The original message is retried with cookies and
  // headers after launch, so adding the URL here would duplicate the download.
  return "dwv://";
}

async function launchApp(deep) {
  let tab;
  try {
    tab = await api.tabs.create({ url: deep, active: false });
  } catch {
    return false;
  }
  if (tab?.id != null) {
    setTimeout(() => {
      try {
        api.tabs.remove(tab.id).catch(() => {});
      } catch {}
    }, TAB_CLOSE_MS);
  }
  return true;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function retrySend(message) {
  const deadline = Date.now() + RETRY_WINDOW_MS;
  while (Date.now() < deadline) {
    await sleep(RETRY_MS);
    const reply = await send(message);
    if (reply.ok) return reply;
  }
  return { ok: false, error: "DwV did not start" };
}

async function handOff(message) {
  let reply = await send(message);
  if (!reply.ok && reply.offline && await launchApp(deepLink(message))) {
    reply = await retrySend(message);
  }
  await setBadge(!reply.ok);
  return reply;
}

function dispositionFilename(value) {
  if (!value) return "";
  const encoded = /filename\*\s*=\s*UTF-8''([^;]+)/i.exec(value);
  if (encoded) {
    try {
      return decodeURIComponent(encoded[1].trim().replace(/^"|"$/g, ""));
    } catch {}
  }
  const plain = /filename\s*=\s*(?:"([^"]*)"|([^;]+))/i.exec(value);
  return plain ? (plain[1] ?? plain[2] ?? "").trim() : "";
}

function onSendHeaders(details) {
  if (!isHttp(details.url)) return;
  const headers = details.requestHeaders || [];
  remember(contexts, details.url, {
    at: Date.now(),
    method: (details.method || "GET").toUpperCase(),
    userAgent: headerValue(headers, "user-agent"),
    referrer: headerValue(headers, "referer"),
    origin: headerValue(headers, "origin"),
    accept: headerValue(headers, "accept"),
  });
}

function recordResponse(details) {
  if (!isHttp(details.url)) return null;
  if (details.statusCode && details.statusCode >= 300) return null;
  const headers = details.responseHeaders || [];
  const disposition = headerValue(headers, "content-disposition");
  const length = Number.parseInt(headerValue(headers, "content-length"), 10);
  const record = {
    at: Date.now(),
    attachment: /^\s*attachment\b/i.test(disposition),
    filename: dispositionFilename(disposition),
    mime: headerValue(headers, "content-type"),
    size: Number.isFinite(length) ? length : undefined,
  };
  remember(responses, details.url, record);
  return record;
}

function mediaKind(url, mime) {
  if (STREAM_EXTENSIONS.test(url)) return "stream";
  if (/mpegurl|dash\+xml|(?:^|\/)mpd$/i.test(mime)) return "stream";
  if (MEDIA_EXTENSIONS.test(url) || MEDIA_MIME.test(mime)) return "file";
  return "";
}

function mediaTitle(url) {
  const name = basename(url.split(/[?#]/)[0]);
  try {
    return decodeURIComponent(name) || "Видео";
  } catch {
    return name || "Видео";
  }
}

function recordMedia(details) {
  if (!isHttp(details.url)) return;
  if (details.tabId == null || details.tabId < 0) return;

  if (details.type === "main_frame") {
    mediaByTab.delete(details.tabId);
    videosByTab.delete(details.tabId);
    return;
  }

  if (!captureOn("captureVideo")) return;
  if (details.statusCode && details.statusCode >= 300) return;

  const headers = details.responseHeaders || [];
  const mime = headerValue(headers, "content-type").split(";")[0].trim().toLowerCase();
  const kind = mediaKind(details.url, mime);
  if (!kind) return;

  const length = Number.parseInt(headerValue(headers, "content-length"), 10);
  rememberForTab(mediaByTab, details.tabId, details.url, {
    url: details.url,
    kind,
    title: mediaTitle(details.url),
    mime,
    size: Number.isFinite(length) ? length : 0,
    poster: "",
    width: 0,
    height: 0,
    duration: 0,
    page: details.documentUrl || details.originUrl || "",
    source: "network",
    at: Date.now(),
  }, MAX_MEDIA_PER_TAB);
}

function mergeVideos(base, extra) {
  const out = { ...extra, ...base };
  for (const key of Object.keys(extra)) {
    const value = out[key];
    if (value === undefined || value === null || value === "" || value === 0) {
      out[key] = extra[key];
    }
  }
  return out;
}

function mergeVideoList(list) {
  const byUrl = new Map();
  for (const item of list) {
    if (!item || !isHttp(item.url)) continue;
    const existing = byUrl.get(item.url);
    byUrl.set(item.url, existing ? mergeVideos(existing, item) : item);
  }
  return [...byUrl.values()];
}

function ingestContentVideos(tabId, videos) {
  const merged = mergeVideoList([...(videosByTab.get(tabId) || []), ...videos]);
  videosByTab.set(tabId, merged.slice(-MAX_VIDEOS_PER_TAB));
}

function videosForTab(tabId) {
  const content = videosByTab.get(tabId) || [];
  const byUrl = new Map();
  for (const item of mergeVideoList(content)) byUrl.set(item.url, item);
  for (const item of mediaByTab.get(tabId)?.values() || []) {
    const existing = byUrl.get(item.url);
    byUrl.set(item.url, existing ? mergeVideos(existing, item) : item);
  }
  return [...byUrl.values()];
}

function findRecord(map, ...urls) {
  for (const url of urls) {
    if (!url) continue;
    const record = map.get(url);
    if (record) return record;
  }
  return undefined;
}

async function cookiesFor(url, storeId) {
  if (!url || !api.cookies?.getAll) return "";
  try {
    const filter = { url };
    if (storeId) filter.storeId = storeId;
    const cookies = await api.cookies.getAll(filter);
    return cookies
      .filter((cookie) => cookie && cookie.name)
      .map((cookie) => `${cookie.name}=${cookie.value ?? ""}`)
      .join("; ");
  } catch {
    return "";
  }
}

async function downloadCookies(finalUrl, url, storeId) {
  const targets = [finalUrl];
  try {
    if (new URL(finalUrl).host === new URL(url).host) targets.push(url);
  } catch {
    targets.push(url);
  }
  const pairs = new Set();
  for (const target of targets) {
    const header = await cookiesFor(target, storeId);
    for (const pair of header.split("; ")) {
      if (pair) pairs.add(pair);
    }
  }
  return [...pairs].join("; ");
}

async function downloadMessage(details) {
  const headers = {};
  const referrer = details.referrer || details.context?.referrer;
  const finalUrl = details.finalUrl || details.url;
  const cookieHeader = await downloadCookies(finalUrl, details.url, details.storeId);
  if (referrer) headers.Referer = referrer;
  if (details.context?.origin) headers.Origin = details.context.origin;
  if (details.context?.accept) headers.Accept = details.context.accept;
  if (cookieHeader) headers.Cookie = cookieHeader;
  return {
    type: "download",
    url: details.url,
    final_url: finalUrl,
    filename: details.filename ? basename(details.filename) : undefined,
    referrer: referrer || undefined,
    cookies: cookieHeader || undefined,
    user_agent: details.context?.userAgent || globalThis.navigator?.userAgent || "",
    headers,
    size: details.size,
    mime: details.mime,
  };
}

async function handleChromeDownload(item, url, context, response, captured, suggest) {
  try {
    await settingsReady;
    if (captured && captureOn("captureFiles")) {
      const message = isTorrent(url, item.filename)
        ? { type: "torrent", source: url }
        : await downloadMessage({
          url,
          finalUrl: item.finalUrl || url,
          filename: item.filename || response?.filename,
          referrer: item.referrer || context?.referrer,
          size: item.fileSize ?? response?.size,
          mime: item.mime || response?.mime,
          storeId: item.storeId,
          context,
        });
      const reply = await handOff(message);
      if (reply.ok) {
        try {
          await api.downloads.cancel(item.id);
        } catch {}
        try {
          await api.downloads.removeFile(item.id);
        } catch {}
        try {
          await api.downloads.erase({ id: item.id });
        } catch {}
      }
    }
  } finally {
    try {
      suggest();
    } catch {}
  }
}

function onDeterminingFilename(item, suggest) {
  const url = item.finalUrl || item.url;
  const context = findRecord(contexts, url, item.url);
  const response = findRecord(responses, url, item.url);
  const extension = fileExtension(url, item.filename);
  const captured = isHttp(url)
    && (!context || context.method === "GET")
    && (Boolean(response?.attachment) || FILE_EXTENSIONS.has(extension));
  handleChromeDownload(item, url, context, response, captured, suggest).catch(() => {});
  return true;
}

function onHeadersReceivedFirefox(details) {
  if (details.method !== "GET") return {};
  if (details.type !== "main_frame") return {};
  if (!isHttp(details.url)) return {};
  const response = recordResponse(details);
  if (!captureOn("captureFiles") || recentlyHandedOff(details.url)) return {};
  const extension = fileExtension(details.url, response?.filename);
  if (!response?.attachment && !FILE_EXTENSIONS.has(extension)) return {};
  remember(handedOff, details.url, Date.now());
  firefoxHandOff(details, response).catch(() => {});
  return { cancel: true };
}

async function browserFallback(url) {
  try {
    await api.downloads.download({ url, saveAs: false });
  } catch {}
}

async function firefoxHandOff(details, response) {
  const context = findRecord(contexts, details.url);
  const filename = response?.filename || basename(details.url.split(/[?#]/)[0]);
  const message = isTorrent(details.url, filename)
    ? { type: "torrent", source: details.url }
    : await downloadMessage({
      url: details.url,
      finalUrl: details.url,
      filename,
      referrer: context?.referrer || details.originUrl || details.documentUrl,
      size: response?.size,
      mime: response?.mime,
      storeId: details.cookieStoreId,
      context,
    });
  const reply = await handOff(message);
  if (!reply.ok) {
    await browserFallback(details.url);
  }
}

function recentlyHandedOff(url) {
  const at = handedOff.get(url);
  if (!at) return false;
  if (Date.now() - at > HANDOFF_TTL_MS) {
    handedOff.delete(url);
    return false;
  }
  return true;
}

async function handleFirefoxCreated(item, url, context, response) {
  const message = isTorrent(url, item.filename)
    ? { type: "torrent", source: url }
    : await downloadMessage({
      url,
      finalUrl: item.finalUrl || url,
      filename: item.filename || response?.filename,
      referrer: item.referrer || context?.referrer,
      size: item.fileSize ?? response?.size,
      mime: item.mime || response?.mime,
      storeId: item.storeId,
      context,
    });
  const reply = await handOff(message);
  if (!reply.ok) {
    await browserFallback(url);
  }
}

async function onCreatedFirefox(item) {
  const url = item.finalUrl || item.url;
  await settingsReady;
  if (!isHttp(url) || recentlyHandedOff(url) || !captureOn("captureFiles")) return;
  const context = findRecord(contexts, url, item.url);
  const response = findRecord(responses, url, item.url);
  if (context && context.method !== "GET") return;
  const extension = fileExtension(url, item.filename);
  if (!response?.attachment && !FILE_EXTENSIONS.has(extension)) return;
  remember(handedOff, url, Date.now());
  api.downloads.cancel(item.id).catch(() => {});
  api.downloads.removeFile(item.id).catch(() => {});
  api.downloads.erase({ id: item.id }).catch(() => {});
  handleFirefoxCreated(item, url, context, response).catch(() => {});
}

async function onMenuClicked(info) {
  if (info.menuItemId !== "dwv-download") return;
  if (!settings.enabled) return;
  const url = info.linkUrl || info.srcUrl;
  if (!url) return;
  if (isTorrent(url)) {
    await handOff({ type: "torrent", source: url });
    return;
  }
  if (!isHttp(url)) return;
  await handOff(await downloadMessage({
    url,
    finalUrl: url,
    referrer: info.pageUrl,
    storeId: info.cookieStoreId,
  }));
}

api.runtime.onInstalled.addListener(() => {
  try {
    api.contextMenus.create({
      id: "dwv-download",
      title: "Download with DwV",
      contexts: ["link", "image"],
    });
  } catch {}
});

function respond(sendResponse, value) {
  try {
    sendResponse(value);
  } catch {}
}

api.runtime.onMessage?.addListener((message, sender, sendResponse) => {
  if (!message || typeof message !== "object") return;

  if (message.type === "videos") {
    const tabId = sender.tab?.id;
    const videos = Array.isArray(message.videos) ? message.videos : [];
    if (tabId != null) {
      settingsReady.then(() => {
        if (captureOn("captureVideo")) ingestContentVideos(tabId, videos);
      }).catch(() => {});
    }
    return;
  }

  if (message.type === "videos-for-tab") {
    settingsReady.then(() => {
      respond(sendResponse, {
        videos: settings.captureVideo ? videosForTab(message.tabId) : [],
      });
    }).catch(() => respond(sendResponse, { videos: [] }));
    return true;
  }

  if (message.type === "handoff") {
    const payload = message.message && typeof message.message === "object"
      ? message.message
      : { type: "ping" };
    settingsReady.then(() => {
      if (!settings.enabled && payload.type !== "ping") {
        respond(sendResponse, { ok: false, error: "Плагин выключен" });
        return;
      }
      handOff(payload)
        .then((reply) => respond(sendResponse, { ok: reply.ok, error: reply.error }))
        .catch(() => respond(sendResponse, { ok: false, error: "DwV не ответил" }));
    }).catch(() => respond(sendResponse, { ok: false, error: "DwV не ответил" }));
    return true;
  }

  if (message.type === "get-status") {
    settingsReady.then(() => respond(sendResponse, { ...settings }))
      .catch(() => respond(sendResponse, { ...DEFAULT_SETTINGS }));
    return true;
  }
});

api.contextMenus.onClicked.addListener((info) => {
  onMenuClicked(info).catch(() => {});
});

api.tabs?.onRemoved?.addListener((tabId) => {
  mediaByTab.delete(tabId);
  videosByTab.delete(tabId);
});

const FILTER = { urls: ["http://*/*", "https://*/*"] };

if (IS_CHROME) {
  api.webRequest.onSendHeaders.addListener(
    onSendHeaders,
    FILTER,
    ["requestHeaders", "extraHeaders"],
  );
  api.webRequest.onHeadersReceived.addListener(
    (details) => {
      recordResponse(details);
      recordMedia(details);
    },
    FILTER,
    ["responseHeaders"],
  );
  api.downloads.onDeterminingFilename.addListener(onDeterminingFilename);
} else {
  api.webRequest.onSendHeaders.addListener(onSendHeaders, FILTER, ["requestHeaders"]);
  api.webRequest.onHeadersReceived.addListener(recordMedia, FILTER, ["responseHeaders"]);
  api.webRequest.onHeadersReceived.addListener(
    onHeadersReceivedFirefox,
    { urls: FILTER.urls, types: ["main_frame"] },
    ["responseHeaders", "blocking"],
  );
  api.downloads.onCreated.addListener(onCreatedFirefox);
}
