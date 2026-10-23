<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { tick } from 'svelte';
  import { t } from '../i18n/index.svelte.ts';
  import { downloadsStore } from '../stores/downloads.svelte.ts';
  import { settingsStore } from '../stores/settings.svelte.ts';
  import Icon from './Icon.svelte';

  export type Mode = 'files' | 'settings';
  export type AddKind = 'file' | 'video' | 'torrent';

  interface Props {
    mode: Mode;
    query: string;
    onQueryChange: (value: string) => void;
    onModeChange: (mode: Mode) => void;
    onAdd: (url: string, kind: AddKind) => void;
    onSettingsClick: () => void;
    onOpenFileDialog: () => void;
    onError: (message: string) => void;
  }

  let { mode, query, onQueryChange, onModeChange, onAdd, onSettingsClick, onOpenFileDialog, onError }: Props = $props();

  let menuOpen = $state(false);
  let addKind = $state<AddKind>('file');
  let addUrl = $state('');
  let urlInput = $state<HTMLInputElement | null>(null);

  const appWindow = getCurrentWindow();

  async function toggleMenu() {
    menuOpen = !menuOpen;
    if (menuOpen) {
      addUrl = '';
      await tick();
      urlInput?.focus();
    }
  }

  function submit() {
    const url = addUrl.trim();
    if (!url) return;
    onAdd(url, addKind);
    menuOpen = false;
    addUrl = '';
  }

  function pickFile() {
    menuOpen = false;
    onOpenFileDialog();
  }

  async function pasteFromClipboard() {
    try {
      const text = await invoke<string>('plugin:clipboard-manager|read_text');
      const value = (text ?? '').trim();
      if (!value) {
        onError(t('add.clipboard_empty'));
        return;
      }
      addUrl = value;
      await tick();
      urlInput?.focus();
    } catch (error) {
      onError(`${t('add.clipboard_failed')}: ${String(error)}`);
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.code === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      submit();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      submit();
    } else if (event.key === 'Escape') {
      menuOpen = false;
    }
  }

  async function minimizeWindow() {
    if (settingsStore.minimize_to_tray) {
      await appWindow.hide();
    } else {
      await appWindow.minimize();
    }
  }

  async function closeWindow() {
    if (settingsStore.close_to_tray) {
      await appWindow.hide();
    } else {
      await appWindow.close();
    }
  }
</script>

<div class="toolbar" data-tauri-drag-region>
  <div class="left" data-tauri-drag-region>
    <div class="logo" data-tauri-drag-region>
      <img class="logo-img" src="/dwv-icon.png" alt="" draggable="false" />
      <span class="app-name" data-tauri-drag-region>DwV</span>
    </div>
  </div>

  <div class="center">
    <div class="search" class:focused={menuOpen === false}>
      <Icon name="search" size={15} />
      <input
        type="text"
        value={query}
        oninput={(e) => onQueryChange(e.currentTarget.value)}
        placeholder={mode === 'files' ? t('search.placeholder') : t('settings.search')}
        aria-label={t('search.placeholder')}
      />
      <button
        class="mode-switch"
        onclick={() => onModeChange(mode === 'files' ? 'settings' : 'files')}
        title={mode === 'files' ? t('settings.title') : t('filter.all')}
        aria-label={mode === 'files' ? t('settings.title') : t('filter.all')}
      >
        <Icon name={mode === 'files' ? 'sliders' : 'download'} size={14} />
      </button>
    </div>
  </div>

  <div class="right">
    <div class="add-wrap">
      <button class="btn-primary" onclick={toggleMenu} aria-expanded={menuOpen}>
        <Icon name="plus" size={15} />
        <span>{t('search.add')}</span>
      </button>

      {#if menuOpen}
        <div class="menu-backdrop" onclick={() => (menuOpen = false)} role="presentation"></div>
        <div class="menu" role="dialog" aria-label={t('search.add')}>
          <div class="kinds">
            <button class="kind" class:active={addKind === 'file'} onclick={() => (addKind = 'file')}>
              <Icon name="download" size={14} />
              <span>{t('add.file')}</span>
            </button>
            <button class="kind" class:active={addKind === 'video'} onclick={() => (addKind = 'video')}>
              <Icon name="video" size={14} />
              <span>{t('add.video')}</span>
            </button>
            <button class="kind" class:active={addKind === 'torrent'} onclick={() => (addKind = 'torrent')}>
              <Icon name="archive" size={14} />
              <span>{t('add.torrent')}</span>
            </button>
          </div>

          <div class="menu-input-row">
            <input
              type="text"
              class="menu-input"
              bind:value={addUrl}
              bind:this={urlInput}
              onkeydown={handleKeydown}
              placeholder={addKind === 'torrent' ? t('add.magnet') : t('add.link')}
            />
            <button class="btn-ghost paste-btn" onclick={pasteFromClipboard}>
              <span>{t('add.paste')}</span>
            </button>
          </div>

          <div class="menu-actions">
            <button class="btn-ghost" onclick={pickFile}>
              <Icon name="folder" size={14} />
              <span>{t('add.pick_torrent')}</span>
            </button>
            <button class="btn-primary" onclick={submit} disabled={!addUrl.trim()}>
              <span>{t('search.add')}</span>
            </button>
          </div>

          {#if addKind === 'file'}
            <p class="menu-hint">{t('add.auto_hint')}</p>
          {/if}
          <p class="menu-hint">{t('add.drag_hint')}</p>
        </div>
      {/if}
    </div>

    <span class="sep"></span>

    <div class="bulk">
      <button class="icon-btn" onclick={() => downloadsStore.pauseAll()} title={t('tray.pause_all')} aria-label={t('tray.pause_all')}>
        <Icon name="pause" size={15} />
      </button>
      <button class="icon-btn" onclick={() => downloadsStore.resumeAll()} title={t('tray.resume_all')} aria-label={t('tray.resume_all')}>
        <Icon name="play" size={15} />
      </button>
    </div>

    <span class="sep"></span>

    <button class="icon-btn" onclick={onSettingsClick} title={t('actions.settings')} aria-label={t('actions.settings')}>
      <Icon name="settings" size={16} />
    </button>
    <button
      class="icon-btn"
      onclick={minimizeWindow}
      title={settingsStore.minimize_to_tray ? t('settings.minimize_tray') : t('actions.minimize')}
      aria-label={t('actions.minimize')}
    >
      <Icon name="minus" size={16} />
    </button>
    <button
      class="icon-btn close"
      onclick={closeWindow}
      title={settingsStore.close_to_tray ? t('settings.close_tray') : t('actions.close')}
      aria-label={t('actions.close')}
    >
      <Icon name="close" size={16} />
    </button>
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 52px;
    padding: 0 10px 0 14px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-color);
    flex-shrink: 0;
    user-select: none;
    position: relative;
    z-index: 30;
  }

  .left {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }

  .logo {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--accent);
  }

  .logo-img {
    width: 24px;
    height: 24px;
    border-radius: 6px;
    display: block;
    flex-shrink: 0;
  }

  .app-name {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.3px;
  }

  .center {
    flex: 1;
    display: flex;
    justify-content: center;
    min-width: 0;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    max-width: 420px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    padding: 0 6px 0 10px;
    transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
  }

  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }

  .search :global(svg) {
    color: var(--text-muted);
  }

  .search input {
    flex: 1;
    min-width: 0;
    padding: 7px 0;
    border: none;
    background: transparent;
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
  }

  .search input::placeholder {
    color: var(--text-muted);
  }

  .mode-switch {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .mode-switch:hover {
    background: var(--bg-tertiary);
    color: var(--accent);
  }

  .right {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
  }

  .add-wrap {
    position: relative;
    margin-right: 6px;
  }

  .btn-primary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 13px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--transition-fast), transform var(--transition-fast);
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-primary:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .bulk {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-right: 6px;
    margin-right: 3px;
    border-right: 1px solid var(--border-color);
  }

  .sep {
    width: 1px;
    height: 20px;
    background: var(--border-color);
    margin: 0 5px;
  }

  .icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--transition-fast), color var(--transition-fast);
  }

  .icon-btn:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .icon-btn.close:hover {
    background: var(--error);
    color: #fff;
  }

  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
  }

  .menu {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 41;
    width: 340px;
    padding: 12px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    animation: menuIn 0.14s ease;
  }

  @keyframes menuIn {
    from { opacity: 0; transform: translateY(-6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .kinds {
    display: flex;
    gap: 4px;
    margin-bottom: 10px;
    background: var(--bg-primary);
    padding: 3px;
    border-radius: var(--radius-sm);
  }

  .kind {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: 6px 4px;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    border-radius: 5px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .kind:hover {
    color: var(--text-primary);
  }

  .kind.active {
    background: var(--accent);
    color: #fff;
  }

  .menu-input-row {
    display: flex;
    align-items: stretch;
    gap: 8px;
  }

  .menu-input {
    flex: 1;
    min-width: 0;
    padding: 9px 11px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
  }

  .menu-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }

  .paste-btn {
    flex-shrink: 0;
  }

  .menu-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 10px;
  }

  .btn-ghost {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 11px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    font-size: 12.5px;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .btn-ghost:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .menu-hint {
    margin-top: 10px;
    font-size: 11px;
    color: var(--text-muted);
    text-align: center;
  }
</style>
