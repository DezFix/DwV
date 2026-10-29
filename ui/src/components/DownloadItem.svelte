<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { type DownloadItem } from '../stores/downloads.svelte.ts';
  import { locale, t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';
  import type { IconName } from '../assets/icons';

  export type SelectEvent = { ctrlKey: boolean; shiftKey: boolean; metaKey: boolean };

  interface Props {
    item: DownloadItem;
    selected: boolean;
    checked: boolean;
    onSelect: (id: number, event: SelectEvent) => void;
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
    if (bytes <= 0) return '—';
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

  function formatAdded(seconds: number): string {
    if (!seconds) return '—';
    const date = new Date(seconds * 1000);
    const today = new Date();
    const sameDay = date.toDateString() === today.toDateString();
    const formatter = new Intl.DateTimeFormat(locale.current === 'ru' ? 'ru-RU' : 'en-US', {
      day: '2-digit',
      month: '2-digit',
      ...(sameDay ? { hour: '2-digit', minute: '2-digit' } : {}),
    });
    return formatter.format(date);
  }

  function hostOf(url: string): string {
    try {
      return new URL(url).hostname.replace(/^www\./i, '');
    } catch {
      return '';
    }
  }

  const metaLine = $derived(
    [
      item.kind === 'torrent' ? 'BitTorrent' : hostOf(item.url),
      t(`type.${item.category}`),
    ]
      .filter(Boolean)
      .join(' · '),
  );

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

  function statusIcon(status: string): IconName {
    switch (status) {
      case 'downloading': return 'download';
      case 'completed': return 'check';
      case 'error': return 'close';
      case 'paused': return 'pause';
      case 'pending': return 'clock';
      default: return 'close';
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
    closeContextMenu();
    await invoke('pause_download', { id: item.id });
  }

  async function handleResume() {
    closeContextMenu();
    await invoke('resume_download', { id: item.id });
  }

  function handleRemove() {
    closeContextMenu();
    onRemove(item.id);
  }

  async function handleOpenFile() {
    if (!item.file_path) return;
    closeContextMenu();
    try {
      await invoke('open_file', { path: item.file_path });
    } catch (error) {
      onError(`${t('details.action_failed')}: ${String(error)}`);
    }
  }

  async function handleShowInFolder() {
    if (!item.file_path) return;
    closeContextMenu();
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
      onSelect(item.id, event);
    }
  }
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
  onclick={(event) => onSelect(item.id, event)}
  onkeydown={handleRowKeydown}
  oncontextmenu={handleContextMenu}
>
  <span class="check-cell" onclick={stop} role="presentation">
    <input
      type="checkbox"
      checked={checked}
      onchange={() => onCheck(item.id)}
      onkeydown={stopKeydown}
      title={t('bulk.select')}
      aria-label={t('bulk.select')}
    />
  </span>

  <div class="name-cell">
    <span class="file-tile cat-{item.category}">{ext}</span>
    <div class="name-copy">
      <span class="name" title={item.filename}>{item.filename}</span>
      <span class="meta" title={item.url}>{metaLine}</span>
    </div>
  </div>

  <span class="cell size cell-size">{formatSize(item.total_size)}</span>

  <span class="cell status cell-status" style="color: {statusColor(item.status)}">
    <Icon name={statusIcon(item.status)} size={13} />
    <span class="status-text" title={item.error ?? statusLabel(item.status)}>{statusLabel(item.status)}</span>
  </span>

  <div class="cell progress-cell">
    <div class="progress-head">
      <span class="pct">{progress.toFixed(0)}%</span>
      <span class="sizes">{formatSize(item.downloaded)} / {formatSize(item.total_size)}</span>
    </div>
    <div class="progress">
      <div class="progress-fill" style="width: {progress}%; background: {statusColor(item.status)}"></div>
    </div>
  </div>

  <span class="cell speed cell-speed">{formatSpeed(item.speed)}</span>
  <span class="cell eta cell-eta">{item.status === 'downloading' ? formatEta(item.eta) : '—'}</span>
  <span class="cell added cell-added">{formatAdded(item.added_at)}</span>

  <div class="row-actions" onclick={stop} role="presentation">
    {#if item.id > 0 && item.kind !== 'video' && item.status === 'downloading'}
      <button class="act" onclick={handlePause} title={t('actions.pause')} aria-label={t('actions.pause')}>
        <Icon name="pause" size={14} />
      </button>
    {:else if item.id > 0 && item.kind !== 'video' && item.status === 'paused'}
      <button class="act" onclick={handleResume} title={t('actions.resume')} aria-label={t('actions.resume')}>
        <Icon name="play" size={14} />
      </button>
    {/if}

    {#if item.status === 'completed'}
      <button class="act" onclick={handleOpenFile} title={t('actions.open')} aria-label={t('actions.open')}>
        <Icon name="external" size={14} />
      </button>
      <button class="act" onclick={handleShowInFolder} title={t('actions.folder')} aria-label={t('actions.folder')}>
        <Icon name="folder" size={14} />
      </button>
    {/if}

    <button class="act danger" onclick={handleRemove} title={t('actions.remove')} aria-label={t('actions.remove')}>
      <Icon name="close" size={14} />
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
      {#if item.status === 'completed'}
        <button class="ctx" onclick={handleOpenFile}>
          <Icon name="external" size={14} />
          {t('actions.open')}
        </button>
        <button class="ctx" onclick={handleShowInFolder}>
          <Icon name="folder" size={14} />
          {t('actions.folder')}
        </button>
        <hr />
      {/if}
      {#if item.id > 0 && item.kind !== 'video' && item.status === 'downloading'}
        <button class="ctx" onclick={handlePause}>
          <Icon name="pause" size={14} />
          {t('actions.pause')}
        </button>
      {:else if item.id > 0 && item.kind !== 'video' && item.status === 'paused'}
        <button class="ctx" onclick={handleResume}>
          <Icon name="play" size={14} />
          {t('actions.resume')}
        </button>
      {/if}
      <button class="ctx danger" onclick={handleRemove}>
        <Icon name="close" size={14} />
        {t('actions.remove')}
      </button>
    </div>
  {/if}
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: var(--row-grid);
    align-items: center;
    gap: 10px;
    min-height: 52px;
    padding: 8px 14px;
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
    background: color-mix(in srgb, var(--warning) 6%, var(--bg-elevated));
    box-shadow: inset 3px 0 var(--warning);
  }

  .row.selected {
    background: var(--accent-soft);
    box-shadow: inset 3px 0 var(--accent);
  }

  .check-cell {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .check-cell input {
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .name-cell {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .name-copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .file-tile {
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    color: #fff;
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: 0.3px;
    overflow: hidden;
  }

  .cat-video { background: linear-gradient(135deg, #f0883e, #db6d28); }
  .cat-audio { background: linear-gradient(135deg, #a371f7, #8957e5); }
  .cat-document { background: linear-gradient(135deg, #3fb950, #2f9e42); }
  .cat-archive { background: linear-gradient(135deg, #d29922, #bb8009); }
  .cat-program { background: linear-gradient(135deg, #58a6ff, #388bfd); }
  .cat-image { background: linear-gradient(135deg, #db61a2, #bf4b8f); }
  .cat-other { background: linear-gradient(135deg, #8b949e, #6e7681); }

  .name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    font-size: 11px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cell {
    font-size: 11.5px;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }

  .size,
  .speed,
  .eta,
  .added {
    text-align: right;
  }

  .speed {
    color: var(--accent);
    font-weight: 600;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 5px;
    font-weight: 500;
  }

  .status-text {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .progress-cell {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .progress-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }

  .pct {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .sizes {
    font-size: 10.5px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .progress {
    height: 4px;
    background: var(--bg-tertiary);
    border-radius: 2px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 0.3s ease;
  }

  .row:hover .progress-fill {
    animation: pulse-bar 0.9s linear infinite;
  }

  .row-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 1px;
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
    width: 26px;
    height: 26px;
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

  hr {
    border: none;
    border-top: 1px solid var(--border-color);
    margin: 4px 0;
  }

  @media (max-width: 1180px) {
    .cell-added {
      display: none;
    }
  }

  @media (max-width: 1040px) {
    .cell-eta {
      display: none;
    }
  }

  @media (max-width: 920px) {
    .cell-size {
      display: none;
    }
  }

  @media (max-width: 820px) {
    .cell-status {
      display: none;
    }
  }
</style>
