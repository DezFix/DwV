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
const status = document.getElementById("status");

async function load() {
  const state = await api.storage.local.get({ captureVideo: true, captureFiles: true });
  captureVideo.checked = state.captureVideo !== false;
  captureFiles.checked = state.captureFiles !== false;
}

function save() {
  api.storage.local.set({
    captureVideo: captureVideo.checked,
    captureFiles: captureFiles.checked,
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

load();
check();

function localize() {
  const ui = api.i18n?.getUILanguage?.() || navigator.language || "en";
  document.documentElement.lang = String(ui).split(/[-_]/)[0];
  for (const node of document.querySelectorAll("[data-i18n]")) {
    node.textContent = t(node.dataset.i18n);
  }
  for (const node of document.querySelectorAll("[data-i18n-html]")) {
    node.innerHTML = t(node.dataset.i18nHtml);
  }
}

localize();
