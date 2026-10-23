const api = globalThis.browser ?? globalThis.chrome;

const dot = document.getElementById("dot");
const statusText = document.getElementById("status");
const enabled = document.getElementById("enabled");
const captureFiles = document.getElementById("capture-files");
const captureVideo = document.getElementById("capture-video");
const videosBox = document.getElementById("videos");
const refreshButton = document.getElementById("refresh");
const openApp = document.getElementById("open-app");
const pageVideo = document.getElementById("page-video");
const pageNotice = document.getElementById("page-notice");
const notice = document.getElementById("notice");

const APP_URL = "http://127.0.0.1:41763/";

async function activeTab() {
  try {
    const [tab] = await api.tabs.query({ active: true, currentWindow: true });
    return tab || null;
  } catch {
    return null;
  }
}

async function refreshStatus() {
  dot.className = "dot";
  statusText.textContent = "проверка…";
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 2500);
  try {
    const response = await fetch(`${APP_URL}health`, {
      signal: controller.signal,
      cache: "no-store",
    });
    if (response.status !== 200) throw new Error();
    dot.className = "dot online";
    statusText.textContent = "DwV запущен";
    return true;
  } catch {
    dot.className = "dot offline";
    statusText.textContent = "DwV не запущен";
    return false;
  } finally {
    clearTimeout(timer);
  }
}

async function loadSettings() {
  try {
    const state = await api.storage.local.get({
      enabled: true,
      captureFiles: true,
      captureVideo: true,
    });
    enabled.checked = state.enabled !== false;
    captureFiles.checked = state.captureFiles !== false;
    captureVideo.checked = state.captureVideo !== false;
  } catch {}
}

async function saveSetting(key, value) {
  try {
    await api.storage.local.set({ [key]: value });
  } catch {}
}

function setNotice(text, tone) {
  notice.textContent = text || "";
  notice.className = "notice" + (tone ? ` ${tone}` : "");
}

function renderEmpty(text) {
  videosBox.innerHTML = "";
  const empty = document.createElement("div");
  empty.className = "empty";
  empty.textContent = text;
  videosBox.append(empty);
}

function mergeVideos(groups) {
  const byUrl = new Map();
  for (const group of groups) {
    if (!Array.isArray(group)) continue;
    for (const item of group) {
      if (!item || typeof item.url !== "string") continue;
      const existing = byUrl.get(item.url);
      if (!existing) {
        byUrl.set(item.url, { ...item });
        continue;
      }
      const merged = { ...item, ...existing };
      for (const key of Object.keys(item)) {
        const value = merged[key];
        if (value === undefined || value === null || value === "" || value === 0) {
          merged[key] = item[key];
        }
      }
      byUrl.set(item.url, merged);
    }
  }
  return [...byUrl.values()];
}

function formatSize(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const units = ["Б", "КБ", "МБ", "ГБ"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const text = value >= 10 || unit === 0
    ? String(Math.round(value))
    : value.toFixed(1).replace(".", ",");
  return `${text} ${units[unit]}`;
}

function formatTime(seconds) {
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`;
  }
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

async function scanVideos() {
  if (!enabled.checked) {
    renderEmpty("Плагин выключен");
    return;
  }
  if (!captureVideo.checked) {
    renderEmpty("Перехват видео выключен");
    return;
  }

  const tab = await activeTab();
  if (!tab?.id) {
    renderEmpty("Нет активной вкладки");
    return;
  }

  renderEmpty("Сканирую страницу…");

  let pageVideos = [];
  try {
    const reply = await api.tabs.sendMessage(tab.id, { type: "scan-videos" });
    pageVideos = Array.isArray(reply) ? reply : reply?.videos || [];
  } catch {}

  let networkVideos = [];
  try {
    const reply = await api.runtime.sendMessage({ type: "videos-for-tab", tabId: tab.id });
    networkVideos = Array.isArray(reply) ? reply : reply?.videos || [];
  } catch {}

  const videos = mergeVideos([pageVideos, networkVideos]);
  if (videos.length === 0) {
    renderEmpty("Видео не найдено");
    return;
  }
  renderVideos(videos, tab.id);
}

function renderVideos(videos, tabId) {
  videosBox.innerHTML = "";
  for (const video of videos) {
    const card = document.createElement("div");
    card.className = "video";

    const name = document.createElement("div");
    name.className = "video-name";
    name.textContent = video.title || "Видео";
    name.title = video.url;

    const meta = document.createElement("div");
    meta.className = "video-meta";
    const resolution = video.width && video.height ? `${video.width}×${video.height}` : "";
    const length = video.duration ? formatTime(video.duration) : "";
    const size = formatSize(video.size);
    meta.textContent = [
      video.kind === "stream" ? "поток" : "файл",
      size,
      length,
      resolution,
    ].filter(Boolean).join(" · ");

    const actions = document.createElement("div");
    actions.className = "video-actions";

    const status = document.createElement("div");
    status.className = "video-status";

    const download = document.createElement("button");
    download.type = "button";
    download.textContent = "Скачать";
    download.disabled = !enabled.checked;
    download.addEventListener("click", () => downloadVideo(video, download, status));

    const watch = document.createElement("button");
    watch.type = "button";
    watch.textContent = "Смотреть";
    watch.addEventListener("click", () => watchOnPage(tabId, watch, status));

    actions.append(download, watch);
    card.append(name, meta, actions, status);
    videosBox.append(card);
  }
}

async function sendVideoUrl(url, button, status) {
  const label = button.textContent;
  button.disabled = true;
  button.textContent = "Отправка...";
  status.textContent = "";
  let reply;
  try {
    reply = await api.runtime.sendMessage({
      type: "handoff",
      message: { type: "video", url },
    });
  } catch (error) {
    reply = { ok: false, error: error?.message || "Не удалось отправить в DwV" };
  }
  if (reply?.ok) {
    button.textContent = "Отправлено";
    return;
  }
  button.disabled = false;
  button.textContent = label;
  status.textContent = "Не получилось: " + (reply?.error || "DwV не ответил");
}

function downloadVideo(video, button, status) {
  return sendVideoUrl(video.url, button, status);
}

async function watchOnPage(tabId, button, status) {
  button.disabled = true;
  status.textContent = "";
  try {
    const reply = await api.tabs.sendMessage(tabId, { type: "watch-on-page" });
    if (reply?.ok) {
      window.close();
      return;
    }
    status.textContent = reply?.error || "Видео не найдено на странице";
  } catch {
    status.textContent = "Скрипт страницы недоступен";
  }
  button.disabled = false;
}

function setPageNotice(text, tone) {
  pageNotice.textContent = text || "";
  pageNotice.className = "notice" + (tone ? ` ${tone}` : "");
}

pageVideo.addEventListener("click", async () => {
  const tab = await activeTab();
  if (!tab?.url || !/^https?:/i.test(tab.url)) {
    setPageNotice("Откройте обычную страницу сайта", "bad");
    return;
  }
  await sendVideoUrl(tab.url, pageVideo, pageNotice);
});

enabled.addEventListener("change", async () => {
  await saveSetting("enabled", enabled.checked);
  await scanVideos();
});

captureFiles.addEventListener("change", () => {
  saveSetting("captureFiles", captureFiles.checked);
});

captureVideo.addEventListener("change", async () => {
  await saveSetting("captureVideo", captureVideo.checked);
  await scanVideos();
});

refreshButton.addEventListener("click", scanVideos);

openApp.addEventListener("click", async () => {
  openApp.disabled = true;
  openApp.textContent = "Открываю…";
  setNotice("");
  let reply;
  try {
    reply = await api.runtime.sendMessage({
      type: "handoff",
      message: { type: "ping" },
    });
  } catch (error) {
    reply = { ok: false, error: error?.message || "нет связи с расширением" };
  }
  if (reply?.ok) {
    setNotice("DwV запущен или уже открыт", "good");
    refreshStatus();
  } else {
    setNotice("Не удалось открыть DwV" + (reply?.error ? `: ${reply.error}` : ""), "bad");
  }
  openApp.disabled = false;
  openApp.textContent = "Открыть DwV";
});

(async () => {
  await loadSettings();
  await Promise.all([refreshStatus(), scanVideos()]);
})();
