<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { type DownloadItem } from '../stores/downloads.svelte.ts';
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';

  interface Props {
    item: DownloadItem;
    selected: boolean;
    checked: boolean;
    onSelect: (id: number) => void;
    onCheck: (id: number) => void;
    onRemove: (id: number) => void;
    onError: (message: string) => void;
  }

  let { item, selected, checked, onSelect, onCheck, onRemove, onError }: Props = $props();

  let showContextMenu = $state(false);
  let contextMenuX = $state(0);
  let contextMenuY = $state(0);

  const progress = $derived(item.total_size > 0 ? (item.downloaded / item.total_size) * 100 : 0);

  function formatSpeed(bytes: number): string {
    if (bytes <= 0) return '—';
    if (bytes < 1024) return `${bytes.toFixed(0)} B/s`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB/s`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB/s`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB/s`;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function formatEta(seconds: number): string {
    if (seconds <= 0) return '—';
    if (seconds < 60) return `${Math.ceil(seconds)} с`;
    if (seconds < 3600) return `${Math.floor(seconds / 60)} мин ${Math.ceil((seconds % 60) / 60)} с`;
    return `${Math.floor(seconds / 3600)} ч ${Math.floor((seconds % 3600) / 60)} мин`;
  }

  function statusLabel(status: string): string {
    switch (status) {
      case 'downloading': return t('status.downloading');
      case 'completed': return t('status.completed');
      case 'error': return t('status.error');
      case 'paused': return t('status.paused');
      case 'pending': return t('status.queued');
      case 'cancelled': return t('status.cancelled');
      default: return status;
    }
  }

  function statusColor(status: string): string {
    switch (status) {
      case 'downloading': return 'var(--accent)';
      case 'completed': return 'var(--success)';
      case 'error': return 'var(--error)';
      case 'paused': return 'var(--warning)';
      default: return 'var(--text-muted)';
    }
  }

  const ext = $derived(item.filename.split('.').pop()?.toUpperCase() ?? '');

  async function handlePause() {
    await invoke('pause_download', { id: item.id });
  }

  async function handleResume() {
    await invoke('resume_download', { id: item.id });
  }

  async function handleRemove() {
    closeContextMenu();
    onRemove(item.id);
  }

  async function handleOpenFile() {
    if (!item.file_path) return;
    try {
      await invoke('open_file', { path: item.file_path });
    } catch (error) {
      onError(`${t('details.action_failed')}: ${String(error)}`);
    }
  }

  async function handleShowInFolder() {
    if (!item.file_path) return;
    try {
      await invoke('show_in_folder', { path: item.file_path });
    } catch (error) {
      onError(`${t('details.action_failed')}: ${String(error)}`);
    }
  }

  function handleContextMenu(event: MouseEvent) {
    event.preventDefault();
    contextMenuX = event.clientX;
    contextMenuY = event.clientY;
    showContextMenu = true;
  }

  function closeContextMenu() {
    showContextMenu = false;
  }

  function stop(event: MouseEvent) {
    event.stopPropagation();
  }

  function stopKeydown(event: KeyboardEvent) {
    event.stopPropagation();
  }

  function handleRowKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onSelect(item.id);
    }
  }

  const kindLabel = $derived(item.kind === 'file' ? null : t(`kind.${item.kind}`));
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="row"
  class:selected
  class:checked
  role="listitem"
  tabindex="0"
  onclick={() => onSelect(item.id)}
  onkeydown={handleRowKeydown}
  oncontextmenu={handleContextMenu}
>
  <button
    type="button"
    class="file-tile cat-{item.category}"
    class:checked
    onclick={(event) => {
      stop(event);
      onCheck(item.id);
    }}
    onkeydown={stopKeydown}
    title={t('bulk.select')}
    aria-label={t('bulk.select')}
    aria-pressed={checked}
  >
    {#if checked}
      <Icon name="check" size={16} />
    {:else}
      <span class="ext">{ext}</span>
    {/if}
  </button>

  <div class="body">
    <div class="name-line">
      <span class="name" title={item.filename}>{item.filename}</span>
      {#if kindLabel}
        <span class="kind-badge {item.kind}" title={kindLabel}>
          <Icon name={item.kind === 'torrent' ? 'archive' : 'video'} size={11} />
          <span>{kindLabel}</span>
        </span>
      {/if}
      <span class="status" style="color: {statusColor(item.status)}">{statusLabel(item.status)}</span>
    </div>

    <div class="progress-line">
      <div class="progress">
        <div class="progress-fill" style="width: {progress}%; background: {statusColor(item.status)}"></div>
      </div>
      <span class="pct">{progress.toFixed(0)}%</span>
      <span class="sizes">{formatSize(item.downloaded)} {t('detail.of')} {formatSize(item.total_size)}</span>
    </div>
  </div>

    <div class="metrics">
      <div class="metric">
        <span class="metric-label">{t('header.speed')}</span>
        <span class="metric-value speed">{formatSpeed(item.speed)}</span>
      </div>
      <div class="metric eta">
        <span class="metric-label">{t('header.eta')}</span>
        <span class="metric-value">{item.status === 'downloading' ? formatEta(item.eta) : '—'}</span>
      </div>
    </div>

  <div class="row-actions" onclick={stop} role="presentation">
    {#if item.id > 0 && item.kind !== 'video' && item.status === 'downloading'}
      <button class="act" onclick={handlePause} title={t('actions.pause')} aria-label={t('actions.pause')}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/></svg>
      </button>
    {:else if item.id > 0 && item.kind !== 'video' && item.status === 'paused'}
      <button class="act" onclick={handleResume} title={t('actions.resume')} aria-label={t('actions.resume')}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="6 3 20 12 6 21 6 3" fill="currentColor" stroke="none"/></svg>
      </button>
    {/if}

    {#if item.status === 'completed'}
      <button class="act" onclick={handleOpenFile} title={t('actions.open')} aria-label={t('actions.open')}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
      </button>
      <button class="act" onclick={handleShowInFolder} title={t('actions.folder')} aria-label={t('actions.folder')}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
      </button>
    {/if}

    <button class="act danger" onclick={handleRemove} title={t('actions.remove')} aria-label={t('actions.remove')}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
    </button>
  </div>

  {#if showContextMenu}
    <div class="menu-backdrop" onclick={(event) => { stop(event); closeContextMenu(); }} role="presentation"></div>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="context-menu"
      style="left: {contextMenuX}px; top: {contextMenuY}px"
      onclick={stop}
      role="menu"
      tabindex="-1"
    >
      <button class="ctx" onclick={handleOpenFile}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
        {t('actions.open')}
      </button>
      <button class="ctx" onclick={handleShowInFolder}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
        {t('actions.folder')}
      </button>
      <hr />
      {#if item.id > 0 && item.kind !== 'video' && item.status === 'downloading'}
        <button class="ctx" onclick={handlePause}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>
          {t('actions.pause')}
        </button>
      {:else if item.id > 0 && item.kind !== 'video' && item.status === 'paused'}
        <button class="ctx" onclick={handleResume}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="6 3 20 12 6 21 6 3"/></svg>
          {t('actions.resume')}
        </button>
      {/if}
      <button class="ctx danger" onclick={handleRemove}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
        {t('actions.remove')}
      </button>
    </div>
  {/if}
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border-color);
    background: var(--bg-elevated);
    transition: background var(--transition-fast), box-shadow var(--transition-fast);
    overflow: hidden;
    animation: rise-in 0.2s ease;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.checked {
    background: color-mix(in srgb, var(--warning) 7%, var(--bg-elevated));
    box-shadow: inset 3px 0 var(--warning);
  }

  .row.selected {
    background: var(--accent-soft);
    box-shadow: inset 3px 0 var(--accent);
  }

  .file-tile {
    width: 36px;
    height: 36px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    color: #fff;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.3px;
    overflow: hidden;
    cursor: pointer;
    transition: box-shadow var(--transition-fast), transform var(--transition-fast), filter var(--transition-fast);
  }

  .file-tile:hover {
    filter: brightness(1.08);
    transform: translateY(-1px);
  }

  .file-tile.checked {
    box-shadow: 0 0 0 2px var(--bg-elevated), 0 0 0 4px var(--accent);
  }

  .cat-video { background: linear-gradient(135deg, #f0883e, #db6d28); }
  .cat-audio { background: linear-gradient(135deg, #a371f7, #8957e5); }
  .cat-document { background: linear-gradient(135deg, #3fb950, #2f9e42); }
  .cat-archive { background: linear-gradient(135deg, #d29922, #bb8009); }
  .cat-program { background: linear-gradient(135deg, #58a6ff, #388bfd); }
  .cat-image { background: linear-gradient(135deg, #db61a2, #bf4b8f); }
  .cat-other { background: linear-gradient(135deg, #8b949e, #6e7681); }

  .body {
    min-width: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .name-line {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status {
    font-size: 11px;
    font-weight: 500;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .kind-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--bg-tertiary);
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 600;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .kind-badge.torrent {
    color: var(--warning);
  }

  .kind-badge.video {
    color: var(--accent);
  }

  .progress-line {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .progress {
    flex: 1;
    height: 4px;
    background: var(--bg-tertiary);
    border-radius: 2px;
    overflow: hidden;
    min-width: 40px;
  }

  .progress-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 0.3s ease;
    background-size: 28px 100%;
  }

  .row:hover .progress-fill {
    animation: pulse-bar 0.9s linear infinite;
  }

  .pct {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    min-width: 34px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .sizes {
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }

  .metrics {
    display: flex;
    align-items: center;
    gap: 18px;
  }

  .metric {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 72px;
  }

  /* Narrow windows: drop the ETA column before the name gets squeezed out. */
  @media (max-width: 1000px) {
    .metric.eta {
      display: none;
    }
  }

  .metric-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--text-muted);
  }

  .metric-value {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .metric-value.speed {
    color: var(--accent);
  }

  .row-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    opacity: 0;
    transition: opacity var(--transition-fast);
  }

  .row:hover .row-actions,
  .row:focus-within .row-actions {
    opacity: 1;
  }

  .act {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--transition-fast), color var(--transition-fast);
  }

  .act:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .act.danger:hover {
    background: var(--error);
    color: #fff;
  }

  .act svg {
    width: 15px;
    height: 15px;
  }

  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 900;
  }

  .context-menu {
    position: fixed;
    z-index: 901;
    background: var(--bg-elevated);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    min-width: 180px;
    padding: 4px;
  }

  .ctx {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    border: none;
    background: transparent;
    color: var(--text-primary);
    font-size: 13px;
    cursor: pointer;
    border-radius: var(--radius-sm);
    text-align: left;
  }

  .ctx:hover {
    background: var(--bg-tertiary);
  }

  .ctx.danger {
    color: var(--error);
  }

  .ctx svg {
    width: 15px;
    height: 15px;
    flex-shrink: 0;
  }

  hr {
    border: none;
    border-top: 1px solid var(--border-color);
    margin: 4px 0;
  }
</style>
