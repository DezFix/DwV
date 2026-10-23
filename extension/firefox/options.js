const api = globalThis.browser ?? globalThis.chrome;

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
    status.textContent = "Соединение установлено — DwV запущен.";
  } catch {
    status.textContent = "Нет соединения — запустите приложение DwV.";
  } finally {
    clearTimeout(timer);
  }
}

captureVideo.addEventListener("change", save);
captureFiles.addEventListener("change", save);

load();
check();
