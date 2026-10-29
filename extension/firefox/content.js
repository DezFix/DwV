/**
 * Runs on every page. Finds media the user might want to hand over to DwV
 * and can bring a chosen video back into view.
 */
const api = globalThis.browser ?? globalThis.chrome;

function t(key, substitution) {
  try {
    return api.i18n?.getMessage(key, substitution) || key;
  } catch {
    return key;
  }
}

const MEDIA_EXTENSIONS = /\.(m3u8|mpd|mp4|m4v|webm|mkv|mov|avi|flv)(?:[?#]|$)/i;
const STREAM_EXTENSIONS = /\.(m3u8|mpd)(?:[?#]|$)/i;
const MAX_VIDEOS = 50;

function absoluteUrl(value) {
  if (!value) return "";
  try {
    const url = new URL(value, location.href);
    return /^https?:$/.test(url.protocol) ? url.href : "";
  } catch {
    return "";
  }
}

function mediaTitle(url) {
  try {
    const name = decodeURIComponent(new URL(url).pathname.split("/").pop() || "");
    return name || document.title || t("videosOnPage");
  } catch {
    return document.title || t("videosOnPage");
  }
}

function makeItem(url, overrides = {}) {
  return {
    url,
    kind: STREAM_EXTENSIONS.test(url) ? "stream" : "file",
    title: document.title || t("videosOnPage"),
    poster: "",
    width: 0,
    height: 0,
    duration: 0,
    page: location.href,
    source: "page",
    ...overrides,
  };
}

function collectVideos() {
  const found = new Map();
  const add = (url, overrides) => {
    if (!url || found.has(url)) return;
    found.set(url, makeItem(url, overrides));
  };

  for (const video of document.querySelectorAll("video")) {
    const source = absoluteUrl(video.currentSrc || video.src || "");
    if (source) {
      add(source, {
        title: video.title || video.getAttribute("aria-label") || document.title || t("videosOnPage"),
        poster: absoluteUrl(video.poster || ""),
        width: video.videoWidth || Math.round(video.getBoundingClientRect().width) || 0,
        height: video.videoHeight || Math.round(video.getBoundingClientRect().height) || 0,
        duration: Number.isFinite(video.duration) ? Math.round(video.duration) : 0,
      });
    }

    for (const sourceEl of video.querySelectorAll("source[src]")) {
      const url = absoluteUrl(sourceEl.getAttribute("src") || "");
      if (!url || !MEDIA_EXTENSIONS.test(url)) continue;
      add(url, {
        title: video.title || document.title || t("videosOnPage"),
        poster: absoluteUrl(video.poster || ""),
        width: video.videoWidth || 0,
        height: video.videoHeight || 0,
        duration: Number.isFinite(video.duration) ? Math.round(video.duration) : 0,
      });
    }
  }

  for (const sourceEl of document.querySelectorAll("source[src]")) {
    if (sourceEl.closest("video")) continue;
    const url = absoluteUrl(sourceEl.getAttribute("src") || "");
    if (!url || !MEDIA_EXTENSIONS.test(url)) continue;
    add(url);
  }

  for (const link of document.querySelectorAll("a[href]")) {
    const url = absoluteUrl(link.getAttribute("href") || "");
    if (!url || !MEDIA_EXTENSIONS.test(url)) continue;
    const text = (link.textContent || "").trim().replace(/\s+/g, " ").slice(0, 160);
    add(url, {
      title: text || mediaTitle(url),
      source: "link",
    });
  }

  return [...found.values()].slice(0, MAX_VIDEOS);
}

function pageMeta() {
  const meta = (name) => {
    const el = document.querySelector(`meta[property="${name}"], meta[name="${name}"]`);
    return el?.content?.trim() || "";
  };
  return {
    url: location.href,
    title: meta("og:title") || meta("twitter:title") || document.title || "",
    image: meta("og:image") || meta("og:image:secure_url") || meta("twitter:image") || "",
    video: meta("og:video") || meta("og:video:url") || meta("og:video:secure_url") || meta("twitter:player:stream") || "",
    site: meta("og:site_name") || location.hostname,
  };
}

function notifyBackground() {
  const videos = collectVideos();
  try {
    const pending = api.runtime.sendMessage({ type: "videos", videos });
    if (pending && typeof pending.catch === "function") pending.catch(() => {});
  } catch {}
}

let scheduled = false;
function scheduleScan() {
  if (scheduled) return;
  scheduled = true;
  setTimeout(() => {
    scheduled = false;
    notifyBackground();
  }, 400);
}

const observer = new MutationObserver(scheduleScan);
observer.observe(document.documentElement, { childList: true, subtree: true });

api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || typeof message !== "object") return;

  if (message.type === "scan-videos") {
    sendResponse({ videos: collectVideos() });
    return;
  }

  if (message.type === "page-meta") {
    sendResponse(pageMeta());
    return;
  }

  if (message.type === "watch-on-page") {
    const target = document.querySelector("video");
    if (!target) {
      sendResponse({ ok: false, error: t("noVideoOnPage") });
      return;
    }
    target.scrollIntoView({ behavior: "smooth", block: "center" });
    target.play().catch(() => {});
    sendResponse({ ok: true });
    return;
  }
});

scheduleScan();
