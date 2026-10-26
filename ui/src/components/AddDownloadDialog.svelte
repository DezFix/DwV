<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';

  interface Props {
    open: boolean;
    filename: string;
    url: string;
    dir: string;
    onCancel: () => void;
    onConfirm: (dir: string, hide: boolean) => void;
  }

  let { open: isOpen, filename, url, dir, onCancel, onConfirm }: Props = $props();

  let folder = $state('');
  let hide = $state(false);

  $effect(() => {
    if (isOpen) {
      folder = dir;
      hide = false;
    }
  });

  async function browse() {
    const picked = await open({
      directory: true,
      defaultPath: folder || undefined,
      title: t('adddialog.folder'),
    });
    if (typeof picked === 'string') folder = picked;
  }

  function confirm() {
    const value = folder.trim();
    if (!value) return;
    onConfirm(value, hide);
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!isOpen) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      onCancel();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      confirm();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if isOpen}
  <div class="overlay" role="presentation" onclick={onCancel}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="add-dialog-title"
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
    >
      <button class="close" onclick={onCancel} title={t('actions.close')} aria-label={t('actions.close')}>
        <Icon name="close" size={14} />
      </button>

      <div class="badge">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
          <polyline points="7 10 12 15 17 10" />
          <line x1="12" y1="15" x2="12" y2="3" />
        </svg>
      </div>

      <h2 id="add-dialog-title">{t('adddialog.title')}</h2>

      <p class="filename" title={filename || url}>{filename || url}</p>
      {#if filename && filename !== url}
        <p class="url" title={url}>{url}</p>
      {/if}

      <span class="label">{t('adddialog.folder')}</span>
      <div class="folder">
        <input type="text" class="input" bind:value={folder} placeholder="—" />
        <button class="btn" onclick={browse}>
          <Icon name="folder" size={15} />
          <span>{t('settings.browse')}</span>
        </button>
      </div>

      <div class="footer">
        <label class="remember">
          <input type="checkbox" bind:checked={hide} />
          <span>{t('adddialog.dont_show')}</span>
        </label>
        <div class="buttons">
          <button class="btn ghost" onclick={onCancel}>{t('adddialog.cancel')}</button>
          <button class="btn primary" onclick={confirm} disabled={!folder.trim()}>
            {t('adddialog.download')}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 1200;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.45);
    animation: overlay-in 140ms ease;
  }

  @keyframes overlay-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .dialog {
    position: relative;
    width: min(470px, 100%);
    padding: 24px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    animation: dialog-pop 170ms ease;
  }

  @keyframes dialog-pop {
    from { opacity: 0; transform: translateY(8px) scale(0.97); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  .close {
    position: absolute;
    top: 10px;
    right: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .close:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .badge {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    margin-bottom: 14px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }

  .badge svg {
    width: 19px;
    height: 19px;
  }

  h2 {
    margin: 0 0 10px;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    padding-right: 18px;
  }

  .filename {
    margin: 0 0 4px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .url {
    margin: 0 0 14px;
    font-size: 11.5px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .label {
    display: block;
    margin: 12px 0 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .folder {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .input {
    flex: 1;
    min-width: 0;
    padding: 7px 10px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 12.5px;
    outline: none;
  }

  .input:focus {
    border-color: var(--accent);
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 18px;
  }

  .remember {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--text-secondary);
    cursor: pointer;
    user-select: none;
  }

  .remember input {
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .buttons {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 12.5px;
    cursor: pointer;
    transition: background var(--transition-fast), border-color var(--transition-fast);
  }

  .btn:hover {
    background: var(--bg-tertiary);
  }

  .btn.primary {
    border-color: var(--accent);
    background: var(--accent);
    color: #fff;
    font-weight: 600;
  }

  .btn.primary:hover {
    filter: brightness(1.08);
  }

  .btn.primary:disabled {
    opacity: 0.5;
    cursor: default;
    filter: none;
  }
</style>
