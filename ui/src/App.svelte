<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { UnlistenFn } from '@tauri-apps/api/event';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { open } from '@tauri-apps/plugin-dialog';
  import DownloadList from './components/DownloadList.svelte';
  import Sidebar, { type View } from './components/Sidebar.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Toolbar, { type AddKind, type Mode } from './components/Toolbar.svelte';
  import SettingsPanel from './components/SettingsPanel.svelte';
  import DetailsPanel from './components/DetailsPanel.svelte';
  import ConfirmDeleteDialog from './components/ConfirmDeleteDialog.svelte';
  import DuplicateDialog from './components/DuplicateDialog.svelte';
  import AddDownloadDialog from './components/AddDownloadDialog.svelte';
  import UpdateDialog from './components/UpdateDialog.svelte';
  import KofiBanner from './components/KofiBanner.svelte';
  import Icon from './components/Icon.svelte';

  interface DownloadPreview {
    filename: string;
    suggested: string;
    exists: boolean;
    path: string;
  }

  interface AddRequest {
    key: string;
    kind: AddKind;
    url: string;
    filename: string;
    dir: string;
    pendingId?: number;
  }

  interface PendingDownload {
    id: number;
    kind: string;
    url: string;
    filename: string;
    dir: string;
  }
  import { downloadsStore, type DownloadCategory } from './stores/downloads.svelte.ts';
  import { settingsStore } from './stores/settings.svelte.ts';
  import { visibleDownloads, type SortBy } from './stores/listView.ts';
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
  let sortBy = $state<SortBy>('added');
  let sortDir = $state<'asc' | 'desc'>('desc');
  let selectedId = $state<number | null>(null);
  let checkedIds = $state<number[]>([]);
  let anchorId = $state<number | null>(null);
  let deleteTargets = $state<number[] | null>(null);
  let deleteFilename = $state<string | undefined>(undefined);
  let duplicate = $state<DownloadPreview | null>(null);
  let duplicateUrl = $state<string | null>(null);
  let duplicateDir = $state<string | undefined>(undefined);
  let addQueue = $state<AddRequest[]>([]);
  let addSeq = 0;
  let defaultDir = $state('');
  let showKofi = $state(false);
  let updateInfo = $state<{
    available: boolean;
    version: string;
    notes?: string;
    url: string;
  } | null>(null);
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

  const visible = $derived(
    visibleDownloads(downloadsStore.items, view, category, query, sortBy, sortDir),
  );

  const allVisibleChecked = $derived(
    visible.length > 0 && visible.every(item => checkedIds.includes(item.id)),
  );

  function toggleCheck(id: number) {
    checkedIds = checkedIds.includes(id)
      ? checkedIds.filter(item => item !== id)
      : [...checkedIds, id];
  }

  function handleRowClick(
    id: number,
    event: { ctrlKey: boolean; shiftKey: boolean; metaKey: boolean },
  ) {
    if (event.ctrlKey || event.metaKey) {
      toggleCheck(id);
    } else if (event.shiftKey) {
      const ids = visible.map(item => item.id);
      const to = ids.indexOf(id);
      const from = anchorId !== null ? ids.indexOf(anchorId) : to;
      if (to >= 0 && from >= 0) {
        const range = ids.slice(Math.min(from, to), Math.max(from, to) + 1);
        checkedIds = [...new Set([...checkedIds, ...range])];
      }
    } else {
      toggleSelect(id);
    }
    anchorId = id;
  }

  function toggleSelectAll() {
    checkedIds = allVisibleChecked ? [] : visible.map(item => item.id);
  }

  function handleGlobalKeydown(event: KeyboardEvent) {
    if (mode !== 'files') return;
    if (addQueue.length > 0 || duplicate !== null || deleteTargets !== null) return;
    if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey) return;
    if (event.code !== 'KeyA') return;
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable="true"]')) return;
    if (visible.length === 0) return;
    event.preventDefault();
    checkedIds = visible.map(item => item.id);
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

    // Closing the details panel stops the media probe, which can hold the file.
    if (selectedId !== null && ids.includes(selectedId)) selectedId = null;

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
      // The backend may have removed the row but failed on the file.
      await downloadsStore.load();
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
      const update = await invoke<{
        available: boolean;
        version: string;
        notes?: string;
        url: string;
      } | null>('check_updates');
      if (!update) {
        if (manual) showToast(t('update.none'));
        return;
      }
      let skipped: string | null = null;
      try {
        skipped = window.localStorage.getItem('dwv.skipped_update');
      } catch {}
      if (!manual && skipped === update.version) return;
      updateInfo = update;
    } catch (error) {
      if (manual) showToast(`${t('update.check_failed')}: ${String(error)}`);
    }
  }

  async function openUpdate() {
    const update = updateInfo;
    if (!update) return;
    try {
      await invoke('open_url', { url: update.url });
      skipUpdate();
    } catch (error) {
      showToast(`${t('update.open_failed')}: ${String(error)}`);
    }
  }

  function skipUpdate() {
    if (updateInfo) {
      try {
        window.localStorage.setItem('dwv.skipped_update', updateInfo.version);
      } catch {}
    }
    updateInfo = null;
  }

  function isTorrentSource(value: string): boolean {
    const lower = value.trim().toLowerCase();
    return lower.startsWith('magnet:') || /\.torrent(?:[?#]|$)/.test(lower);
  }

  async function performAdd(value: string, kind: AddKind, filename?: string, dir?: string, quality?: string) {
    if (filename) {
      await downloadsStore.addAs(value, filename, dir);
    } else if (kind === 'video') {
      await downloadsStore.addVideo(value, dir, quality);
    } else if (kind === 'torrent' || isTorrentSource(value)) {
      await downloadsStore.addTorrent(value, dir);
    } else {
      await downloadsStore.add(value, dir);
    }
    mode = 'files';
    query = '';
  }

  async function startAdd(value: string, kind: AddKind, dir?: string, quality?: string) {
    try {
      if (kind === 'file' && !isTorrentSource(value)) {
        try {
          const preview = await invoke<DownloadPreview>('preview_download', { url: value, dir });
          if (preview.exists) {
            duplicateUrl = value;
            duplicateDir = dir;
            duplicate = preview;
            return;
          }
        } catch {
          // Preview is a safety net; a failed HEAD must not block the download.
        }
      }
      await performAdd(value, kind, undefined, dir, quality);
    } catch (error) {
      showToast(`${t('add.failed')}: ${String(error)}`);
    }
  }

  function suggestedName(url: string) {
    const path = url.split(/[?#]/)[0];
    const tail = path.split('/').filter(Boolean).pop() ?? '';
    try {
      return decodeURIComponent(tail);
    } catch {
      return tail;
    }
  }

  async function handleAdd(url: string, kind: AddKind) {
    const value = url.trim();
    if (!value) return;

    if (settingsStore.ask_save_path) {
      addQueue = [
        ...addQueue,
        {
          key: `local-${addSeq++}`,
          kind,
          url: value,
          filename: suggestedName(value),
          dir: settingsStore.download_dir || defaultDir,
        },
      ];
      return;
    }

    await startAdd(value, kind);
  }

  async function confirmAdd(dir: string, hide: boolean, quality: string) {
    const request = addQueue[0];
    if (!request) return;
    addQueue = addQueue.slice(1);
    if (hide) await settingsStore.save({ ask_save_path: false });

    if (request.pendingId !== undefined) {
      try {
        await invoke('resolve_pending', { id: request.pendingId, dir, quality });
        mode = 'files';
        query = '';
      } catch (error) {
        showToast(`${t('add.failed')}: ${String(error)}`);
      }
      return;
    }

    await startAdd(request.url, request.kind, dir, quality);
  }

  async function cancelAdd() {
    const request = addQueue[0];
    if (!request) return;
    addQueue = addQueue.slice(1);
    if (request.pendingId !== undefined) {
      try {
        await invoke('cancel_pending', { id: request.pendingId });
      } catch {
        // The request may already be gone.
      }
    }
  }

  function enqueuePending(item: PendingDownload) {
    if (addQueue.some(request => request.pendingId === item.id)) return;
    const kind: AddKind = item.kind === 'torrent'
      ? 'torrent'
      : item.kind === 'video'
        ? 'video'
        : 'file';
    addQueue = [
      ...addQueue,
      {
        key: `pending-${item.id}`,
        kind,
        url: item.url,
        filename: item.filename,
        dir: item.dir,
        pendingId: item.id,
      },
    ];
  }

  function cancelDuplicate() {
    duplicate = null;
    duplicateUrl = null;
    duplicateDir = undefined;
  }

  async function renameDuplicate() {
    const url = duplicateUrl;
    const filename = duplicate?.suggested;
    const dir = duplicateDir;
    cancelDuplicate();
    if (!url || !filename) return;
    try {
      await performAdd(url, 'file', filename, dir);
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
      try {
        defaultDir = await invoke<string>('default_download_dir');
      } catch {
        defaultDir = '';
      }

      unlisteners.push(
        await listen<ProgressPayload>('download-progress', e => downloadsStore.updateProgress(e.payload)),
        await listen<StatusPayload>('download-status', e => downloadsStore.updateStatus(e.payload)),
        await listen<CompletePayload>('download-complete', e => downloadsStore.onComplete(e.payload)),
        await listen<ErrorPayload>('download-error', e => downloadsStore.onError(e.payload)),
        await listen<DownloadItem>('download-added', e => downloadsStore.addItem(e.payload)),
        await listen<PendingDownload>('pending-download', e => enqueuePending(e.payload)),
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

      try {
        const waiting = await invoke<PendingDownload[]>('pending_downloads');
        for (const item of waiting) enqueuePending(item);
      } catch {
        // The queue is rebuilt on the next event.
      }

      void checkForUpdates(false);
    })();

    return () => {
      window.removeEventListener('dwv-check-updates', checkUpdates);
      unlisteners.forEach(unlisten => unlisten());
      if (toastTimer) window.clearTimeout(toastTimer);
    };
  });
</script>

<svelte:window onkeydown={handleGlobalKeydown} />

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
              {#if visible.length > 0}
                <button class="select-all" onclick={toggleSelectAll} title={t('bulk.select_all_hint')}>
                  {allVisibleChecked ? t('bulk.clear') : t('bulk.select_all')}
                </button>
              {/if}
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
            onSelect={handleRowClick}
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

  {#key addQueue[0]?.key}
    <AddDownloadDialog
      open={addQueue.length > 0 && duplicate === null}
      filename={addQueue[0]?.filename ?? ''}
      url={addQueue[0]?.url ?? ''}
      kind={addQueue[0]?.kind ?? 'file'}
      dir={addQueue[0]?.dir ?? ''}
      onCancel={cancelAdd}
      onConfirm={confirmAdd}
    />
  {/key}

  <UpdateDialog
    open={updateInfo !== null}
    version={updateInfo?.version ?? ''}
    notes={updateInfo?.notes}
    onUpdate={openUpdate}
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

  .select-all {
    margin-left: 6px;
    padding: 2px 8px;
    border: 1px solid var(--border-color);
    border-radius: 999px;
    background: transparent;
    color: var(--text-secondary);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: background var(--transition-fast), color var(--transition-fast), border-color var(--transition-fast);
  }

  .select-all:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
    border-color: var(--accent);
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
