<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { UnlistenFn } from '@tauri-apps/api/event';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { open } from '@tauri-apps/plugin-dialog';
  import { check, type Update } from '@tauri-apps/plugin-updater';
  import DownloadList from './components/DownloadList.svelte';
  import Sidebar, { type View } from './components/Sidebar.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Toolbar, { type AddKind, type Mode } from './components/Toolbar.svelte';
  import SettingsPanel from './components/SettingsPanel.svelte';
  import DetailsPanel from './components/DetailsPanel.svelte';
  import ConfirmDeleteDialog from './components/ConfirmDeleteDialog.svelte';
  import DuplicateDialog from './components/DuplicateDialog.svelte';
  import UpdateDialog from './components/UpdateDialog.svelte';
  import KofiBanner from './components/KofiBanner.svelte';
  import Icon from './components/Icon.svelte';

  interface DownloadPreview {
    filename: string;
    suggested: string;
    exists: boolean;
    path: string;
  }
  import { downloadsStore, type DownloadCategory } from './stores/downloads.svelte.ts';
  import { settingsStore } from './stores/settings.svelte.ts';
  import { deletePrefs } from './stores/deletePrefs.svelte.ts';
  import { setLocale, t } from './i18n/index.svelte.ts';
  import type {
    CompletePayload,
    DownloadItem,
    ErrorPayload,
    ProgressPayload,
    StatusPayload,
  } from './stores/downloads.svelte.ts';

  let mode = $state<Mode>('files');
  let query = $state('');
  let view = $state<View>('all');
  let category = $state<DownloadCategory | null>(null);
  let sortBy = $state<'added' | 'name' | 'progress' | 'speed' | 'size'>('added');
  let sortDir = $state<'asc' | 'desc'>('desc');
  let selectedId = $state<number | null>(null);
  let checkedIds = $state<number[]>([]);
  let deleteTargets = $state<number[] | null>(null);
  let deleteFilename = $state<string | undefined>(undefined);
  let duplicate = $state<DownloadPreview | null>(null);
  let duplicateUrl = $state<string | null>(null);
  let showKofi = $state(false);
  let updateInfo = $state<{ version: string; notes?: string } | null>(null);
  let updateBusy = $state(false);
  let updateError = $state<string | null>(null);
  let pendingUpdate: Update | null = null;
  let toastMessage = $state<string | null>(null);
  let toastTimer: number | undefined;

  const FIRST_USE_KEY = 'dwv.first_use';
  const KOFI_LAST_KEY = 'dwv.kofi_last_shown';
  const KOFI_INTERVAL = 14 * 24 * 60 * 60 * 1000;

  function applyTheme(theme: 'system' | 'light' | 'dark') {
    const dark =
      theme === 'dark' ||
      (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  }

  function setMode(next: Mode) {
    if (mode === next) return;
    mode = next;
    query = '';
  }

  function navigate(next: View, nextCategory: DownloadCategory | null) {
    mode = 'files';
    view = next;
    category = nextCategory;
  }

  function toggleSelect(id: number) {
    selectedId = selectedId === id ? null : id;
  }

  function toggleCheck(id: number) {
    checkedIds = checkedIds.includes(id)
      ? checkedIds.filter(item => item !== id)
      : [...checkedIds, id];
  }

  function requestRemove(id: number) {
    requestRemoveMany([id]);
  }

  function requestRemoveMany(ids: number[]) {
    const unique = [...new Set(ids)];
    if (unique.length === 0) return;

    if (deletePrefs.mode !== 'ask') {
      void performRemove(unique, deletePrefs.mode === 'delete');
      return;
    }

    deleteTargets = unique;
    deleteFilename = unique.length === 1
      ? downloadsStore.items.find(item => item.id === unique[0])?.filename
      : undefined;
  }

  async function performRemove(ids: number[], deleteFiles: boolean) {
    const removed: number[] = [];
    let firstError: string | null = null;

    for (const id of ids) {
      try {
        await downloadsStore.remove(id, deleteFiles);
        removed.push(id);
      } catch (error) {
        firstError ??= String(error);
      }
    }

    if (removed.length > 0) {
      checkedIds = checkedIds.filter(id => !removed.includes(id));
      if (selectedId !== null && removed.includes(selectedId)) {
        selectedId = null;
      }
    }

    if (firstError) {
      showToast(`${t('delete.failed')}: ${firstError}`);
    }
  }

  function cancelDelete() {
    deleteTargets = null;
    deleteFilename = undefined;
  }

  function confirmDelete(mode: 'keep' | 'delete', remember: boolean) {
    const ids = deleteTargets ?? [];
    if (remember) deletePrefs.setMode(mode);
    cancelDelete();
    if (ids.length > 0) void performRemove(ids, mode === 'delete');
  }

  function initKofi() {
    try {
      const now = Date.now();
      const firstRaw = window.localStorage.getItem(FIRST_USE_KEY);
      if (!firstRaw) {
        window.localStorage.setItem(FIRST_USE_KEY, String(now));
        return;
      }

      const first = Number(firstRaw);
      if (!Number.isFinite(first) || now - first < KOFI_INTERVAL) return;

      const lastRaw = window.localStorage.getItem(KOFI_LAST_KEY);
      const last = lastRaw ? Number(lastRaw) : 0;
      if (Number.isFinite(last) && now - last < KOFI_INTERVAL) return;

      showKofi = true;
      window.localStorage.setItem(KOFI_LAST_KEY, String(now));
    } catch {
      // Storage unavailable: do not show the banner.
    }
  }

  function dismissKofi() {
    showKofi = false;
    try {
      window.localStorage.setItem(KOFI_LAST_KEY, String(Date.now()));
    } catch {
      // Ignore storage failures.
    }
  }

  function showToast(message: string) {
    toastMessage = message;
    if (toastTimer) window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      toastMessage = null;
      toastTimer = undefined;
    }, 6000);
  }

  function dismissToast() {
    toastMessage = null;
    if (toastTimer) {
      window.clearTimeout(toastTimer);
      toastTimer = undefined;
    }
  }

  async function checkForUpdates(manual: boolean) {
    try {
      const update = await check();
      if (!update) {
        if (manual) showToast(t('update.none'));
        return;
      }
      let skipped: string | null = null;
      try {
        skipped = window.localStorage.getItem('dwv.skipped_update');
      } catch {}
      if (!manual && skipped === update.version) return;
      pendingUpdate = update;
      updateInfo = { version: update.version, notes: update.body ?? '' };
      updateError = null;
    } catch (error) {
      if (manual) showToast(`${t('update.check_failed')}: ${String(error)}`);
    }
  }

  async function installUpdate() {
    if (!pendingUpdate) return;
    updateBusy = true;
    updateError = null;
    try {
      await pendingUpdate.downloadAndInstall();
    } catch (error) {
      updateError = String(error);
      updateBusy = false;
    }
  }

  function skipUpdate() {
    if (updateInfo) {
      try {
        window.localStorage.setItem('dwv.skipped_update', updateInfo.version);
      } catch {}
    }
    updateInfo = null;
    pendingUpdate = null;
    updateBusy = false;
    updateError = null;
  }

  function isTorrentSource(value: string): boolean {
    const lower = value.trim().toLowerCase();
    return lower.startsWith('magnet:') || /\.torrent(?:[?#]|$)/.test(lower);
  }

  async function performAdd(value: string, kind: AddKind, filename?: string) {
    if (filename) {
      await downloadsStore.addAs(value, filename);
    } else if (kind === 'video') {
      await downloadsStore.addVideo(value);
    } else if (kind === 'torrent' || isTorrentSource(value)) {
      await downloadsStore.addTorrent(value);
    } else {
      await downloadsStore.add(value);
    }
    mode = 'files';
    query = '';
  }

  async function handleAdd(url: string, kind: AddKind) {
    const value = url.trim();
    if (!value) return;

    try {
      if (kind === 'file' && !isTorrentSource(value)) {
        try {
          const preview = await invoke<DownloadPreview>('preview_download', { url: value });
          if (preview.exists) {
            duplicateUrl = value;
            duplicate = preview;
            return;
          }
        } catch {
          // Preview is a safety net; a failed HEAD must not block the download.
        }
      }
      await performAdd(value, kind);
    } catch (error) {
      showToast(`${t('add.failed')}: ${String(error)}`);
    }
  }

  function cancelDuplicate() {
    duplicate = null;
    duplicateUrl = null;
  }

  async function renameDuplicate() {
    const url = duplicateUrl;
    const filename = duplicate?.suggested;
    cancelDuplicate();
    if (!url || !filename) return;
    try {
      await performAdd(url, 'file', filename);
    } catch (error) {
      showToast(`${t('add.failed')}: ${String(error)}`);
    }
  }

  async function handleOpenFileDialog() {
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: t('add.torrent'), extensions: ['torrent'] }],
      });
      if (typeof picked === 'string') {
        await handleAdd(picked, 'torrent');
      }
    } catch (error) {
      showToast(`${t('add.failed')}: ${String(error)}`);
    }
  }

  function handleDrop(event: DragEvent) {
    event.preventDefault();
    const files = event.dataTransfer?.files;
    const text = event.dataTransfer?.getData('text')?.trim();

    if (files && files.length > 0) {
      const file = files[0] as File & { path?: string };
      if (file.name.toLowerCase().endsWith('.torrent')) {
        void handleAdd(file.path ?? file.name, 'torrent');
      } else if (text) {
        void handleAdd(text, 'file');
      } else {
        showToast(t('add.unsupported_drop'));
      }
      return;
    }

    if (text) {
      void handleAdd(text, 'file');
    }
  }

  function handleDragOver(event: DragEvent) {
    event.preventDefault();
  }

  $effect(() => {
    setLocale(settingsStore.language);
  });

  $effect(() => {
    document.documentElement.dataset.animations = settingsStore.animations_enabled ? 'on' : 'off';
  });

  $effect(() => {
    document.documentElement.style.zoom = String(settingsStore.ui_scale || 1);
  });

  $effect(() => {
    const theme = settingsStore.theme;
    applyTheme(theme);

    if (theme !== 'system') return;

    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => applyTheme('system');
    media.addEventListener('change', onChange);
    return () => media.removeEventListener('change', onChange);
  });

  $effect(() => {
    if (selectedId !== null && !downloadsStore.items.some(item => item.id === selectedId)) {
      selectedId = null;
    }
  });

  $effect(() => {
    const ids = new Set(downloadsStore.items.map(item => item.id));
    if (checkedIds.some(id => !ids.has(id))) {
      checkedIds = checkedIds.filter(id => ids.has(id));
    }
  });

  onMount(() => {
    initKofi();
    const checkUpdates = () => void checkForUpdates(true);
    window.addEventListener('dwv-check-updates', checkUpdates);
    const unlisteners: UnlistenFn[] = [];
    const appWindow = getCurrentWindow();

    (async () => {
      await Promise.all([downloadsStore.load(), settingsStore.load()]);

      unlisteners.push(
        await listen<ProgressPayload>('download-progress', e => downloadsStore.updateProgress(e.payload)),
        await listen<StatusPayload>('download-status', e => downloadsStore.updateStatus(e.payload)),
        await listen<CompletePayload>('download-complete', e => downloadsStore.onComplete(e.payload)),
        await listen<ErrorPayload>('download-error', e => downloadsStore.onError(e.payload)),
        await listen<DownloadItem>('download-added', e => downloadsStore.addItem(e.payload)),
        await listen<{ id: number; filename: string }>('download-renamed', e =>
          showToast(`${t('duplicate.renamed')}: ${e.payload.filename}`),
        ),
        await listen<{ id: number }>('download-removed', e => downloadsStore.removeItem(e.payload.id)),
        await listen('open-settings', () => setMode('settings')),
        await appWindow.onDragDropEvent(event => {
          if (event.payload.type !== 'drop') return;

          let handled = 0;
          for (const path of event.payload.paths) {
            const lower = path.toLowerCase();
            if (lower.endsWith('.torrent')) {
              void handleAdd(path, 'torrent');
              handled++;
            } else if (lower.startsWith('magnet:') || lower.startsWith('http:') || lower.startsWith('https:')) {
              void handleAdd(path, 'file');
              handled++;
            }
          }
          if (handled === 0) {
            showToast(t('add.unsupported_drop'));
          }
        }),
      );

      void checkForUpdates(false);
    })();

    return () => {
      window.removeEventListener('dwv-check-updates', checkUpdates);
      unlisteners.forEach(unlisten => unlisten());
      if (toastTimer) window.clearTimeout(toastTimer);
    };
  });
</script>

<div
  class="app"
  role="application"
  ondragenter={handleDragOver}
  ondragover={handleDragOver}
  ondrop={handleDrop}
>
  <Toolbar
    {mode}
    {query}
    onQueryChange={(value) => (query = value)}
    onModeChange={setMode}
    onAdd={handleAdd}
    onSettingsClick={() => setMode('settings')}
    onOpenFileDialog={handleOpenFileDialog}
    onError={showToast}
  />

  <div class="workspace">
    {#if mode === 'files'}
      <Sidebar {view} {category} onNavigate={navigate} />
    {/if}

    <main class="main">
      {#if mode === 'files'}
        <div class="files-view">
          <div class="list-toolbar">
            <div class="crumbs">
              <span class="crumb-title">
                {category ? t(`type.${category}`) : t(`filter.${view}`)}
              </span>
              <span class="crumb-count">{downloadsStore.items.length}</span>
            </div>

            <div class="sort-group">
              <select bind:value={sortBy} class="sort-select" aria-label="Sort">
                <option value="added">{t('detail.added')}</option>
                <option value="name">{t('header.name')}</option>
                <option value="progress">%</option>
                <option value="speed">{t('header.speed')}</option>
                <option value="size">{t('header.size')}</option>
              </select>
              <button
                class="sort-dir"
                onclick={() => (sortDir = sortDir === 'asc' ? 'desc' : 'asc')}
                title={sortDir === 'asc' ? '↑' : '↓'}
                aria-label="Sort direction"
              >
                {#if sortDir === 'asc'}
                  <Icon name="chevronUp" size={14} />
                {:else}
                  <Icon name="chevronDown" size={14} />
                {/if}
              </button>
            </div>
          </div>

          {#if checkedIds.length > 0}
            <div class="bulk-bar">
              <span class="bulk-count">{t('bulk.selected')}: {checkedIds.length}</span>
              <div class="bulk-actions">
                <button class="bulk-btn danger" onclick={() => requestRemoveMany(checkedIds)}>
                  {t('bulk.delete')}
                </button>
                <button class="bulk-btn" onclick={() => (checkedIds = [])}>
                  {t('bulk.clear')}
                </button>
              </div>
            </div>
          {/if}

          {#if showKofi}
            <KofiBanner onClose={dismissKofi} onError={showToast} />
          {/if}

          <DownloadList
            {view}
            {category}
            {sortBy}
            {sortDir}
            {query}
            {selectedId}
            {checkedIds}
            onSelect={toggleSelect}
            onCheck={toggleCheck}
            onRemove={requestRemove}
            onError={showToast}
          />

          {#if selectedId !== null}
            <DetailsPanel id={selectedId} onClose={() => (selectedId = null)} />
          {/if}
        </div>
      {:else}
        <SettingsPanel {query} onClose={() => setMode('files')} />
      {/if}
    </main>
  </div>

  <StatusBar />

  <ConfirmDeleteDialog
    open={deleteTargets !== null}
    count={deleteTargets?.length ?? 0}
    filename={deleteFilename}
    onCancel={cancelDelete}
    onConfirm={confirmDelete}
  />

  <DuplicateDialog
    open={duplicate !== null}
    filename={duplicate?.filename ?? ''}
    suggested={duplicate?.suggested ?? ''}
    onCancel={cancelDuplicate}
    onRename={renameDuplicate}
  />

  <UpdateDialog
    open={updateInfo !== null}
    version={updateInfo?.version ?? ''}
    notes={updateInfo?.notes}
    busy={updateBusy}
    error={updateError}
    onUpdate={installUpdate}
    onSkip={skipUpdate}
  />

  {#if toastMessage}
    <div class="toast" role="status">
      <span class="toast-text">{toastMessage}</span>
      <button class="toast-close" onclick={dismissToast} aria-label={t('error.dismiss')}>
        <Icon name="close" size={13} />
      </button>
    </div>
  {/if}
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 13px;
    overflow: hidden;
  }

  .workspace {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--bg-elevated);
  }

  .files-view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    animation: fade-in 0.16s ease;
  }

  .list-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 38px;
    padding: 0 12px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-color);
    flex-shrink: 0;
  }

  .crumbs {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .bulk-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 12px;
    background: var(--accent-soft);
    border-bottom: 1px solid var(--border-color);
    flex-shrink: 0;
    animation: bulk-in 0.16s ease;
  }

  @keyframes bulk-in {
    from { opacity: 0; transform: translateY(-6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .bulk-count {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }

  .bulk-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .bulk-btn {
    padding: 4px 10px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background var(--transition-fast), color var(--transition-fast), border-color var(--transition-fast);
  }

  .bulk-btn:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .bulk-btn.danger {
    color: var(--error);
    border-color: color-mix(in srgb, var(--error) 35%, var(--border-color));
  }

  .bulk-btn.danger:hover {
    background: color-mix(in srgb, var(--error) 10%, transparent);
    color: var(--error);
  }

  .crumb-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .crumb-count {
    font-size: 11px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .sort-group {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .sort-select {
    padding: 4px 8px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    outline: none;
  }

  .sort-dir {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    color: var(--text-secondary);
    cursor: pointer;
  }

  .sort-dir:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .toast {
    position: fixed;
    right: 14px;
    bottom: 42px;
    z-index: 1000;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    max-width: 420px;
    padding: 10px 12px;
    background: var(--bg-elevated);
    border: 1px solid color-mix(in srgb, var(--error) 45%, var(--border-color));
    border-left: 3px solid var(--error);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    animation: rise-in 0.18s ease;
  }

  .toast-text {
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--text-primary);
    word-break: break-word;
  }

  .toast-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--text-muted);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .toast-close:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }
</style>
