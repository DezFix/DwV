import { mount } from 'svelte';
import App from './App.svelte';

const target = document.getElementById('app')!;

function showFatal(message: string, detail?: string) {
  const overlay = document.createElement('div');
  overlay.style.cssText = [
    'position:fixed',
    'inset:0',
    'z-index:99999',
    'background:#0f1117',
    'color:#f85149',
    'font-family:ui-monospace,Menlo,Consolas,monospace',
    'font-size:12px',
    'padding:28px',
    'overflow:auto',
  ].join(';');

  overlay.innerHTML = `
    <h2 style="margin:0 0 10px;font-size:15px">${message}</h2>
    ${detail ? `<pre style="white-space:pre-wrap;color:#a8b0bd;margin:0 0 18px">${detail}</pre>` : ''}
    <button id="fatal-close" style="padding:8px 16px;border:1px solid #3b424f;border-radius:6px;background:#1f242c;color:#e6e9ef;cursor:pointer;font-size:13px">
      Закрыть приложение
    </button>
  `;

  document.body.appendChild(overlay);
  overlay.querySelector('#fatal-close')?.addEventListener('click', () => {
    import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) => getCurrentWindow().close())
      .catch(() => window.close());
  });
}

let appReady = false;

window.addEventListener('error', event => {
  if (appReady) return;
  showFatal('Ошибка при загрузке интерфейса', event.error?.stack ?? event.message);
});

window.addEventListener('unhandledrejection', event => {
  console.error('Unhandled promise rejection:', event.reason);
});

try {
  mount(App, { target });
  appReady = true;
} catch (error) {
  showFatal('Не удалось запустить приложение', error instanceof Error ? error.stack : String(error));
}
