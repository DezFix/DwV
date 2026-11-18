const api = globalThis.browser ?? globalThis.chrome;

function t(key, substitution) {
  try {
    return api.i18n?.getMessage(key, substitution) || key;
  } catch {
    return key;
  }
}

const captureVideo = document.getElementById("capture-video");
const captureFiles = document.getElementById("capture-files");
const minSize = document.getElementById("min-size");
const excludedSites = document.getElementById("excluded-sites");
const videoQuality = document.getElementById("video-quality");
const status = document.getElementById("status");

async function load() {
  const state = await api.storage.local.get({
    captureVideo: true,
    captureFiles: true,
    minSizeKb: 0,
    excludedSites: "",
    videoQuality: "",
  });
  captureVideo.checked = state.captureVideo !== false;
  captureFiles.checked = state.captureFiles !== false;
  minSize.value = String(Number(state.minSizeKb) > 0 ? Number(state.minSizeKb) : 0);
  excludedSites.value = typeof state.excludedSites === "string" ? state.excludedSites : "";
  videoQuality.value = state.videoQuality || "";
}

function save() {
  const size = Number(minSize.value);
  api.storage.local.set({
    captureVideo: captureVideo.checked,
    captureFiles: captureFiles.checked,
    minSizeKb: Number.isFinite(size) && size > 0 ? Math.round(size) : 0,
    excludedSites: excludedSites.value.trim(),
    videoQuality: videoQuality.value,
  });
}

async function check() {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 2500);
  try {
    const response = await fetch("http://127.0.0.1:41763/health", { signal: controller.signal });
    if (response.status !== 200) throw new Error();
    status.textContent = t("connectionOk");
  } catch {
    status.textContent = t("connectionFail");
  } finally {
    clearTimeout(timer);
  }
}

captureVideo.addEventListener("change", save);
captureFiles.addEventListener("change", save);
minSize.addEventListener("change", save);
excludedSites.addEventListener("change", save);
videoQuality.addEventListener("change", save);

load();
check();

function localize() {
  const ui = api.i18n?.getUILanguage?.() || navigator.language || "en";
  document.documentElement.lang = String(ui).split(/[-_]/)[0];
  for (const node of document.querySelectorAll("[data-i18n]")) {
    node.textContent = t(node.dataset.i18n);
  }
}

localize();
