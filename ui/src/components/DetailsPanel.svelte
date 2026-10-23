<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { downloadsStore, type DownloadDetails, type DownloadStatus } from '../stores/downloads.svelte.ts';
  import { locale, t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';

  interface Props {
    id: number;
    onClose: () => void;
  }

  let { id, onClose }: Props = $props();

  type Tab = 'file' | 'video' | 'peers';

  let details = $state<DownloadDetails | null>(null);
  let loadError = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let activeTab = $state<Tab>('file');
  let requestSeq = 0;
  let debounceTimer: number | undefined;

  const liveItem = $derived(downloadsStore.items.find(item => item.id === id));
  const kind = $derived(details?.kind ?? liveItem?.kind ?? 'file');
  const title = $derived(details?.filename ?? liveItem?.filename ?? '');
  const peers = $derived(details?.peers ?? []);
  const hasVideo = $derived(
    kind === 'video' || (details?.category ?? liveItem?.category) === 'video',
  );
  const tabs = $derived<Tab[]>(
    kind === 'torrent' ? ['file', 'peers'] : hasVideo ? ['file', 'video'] : ['file'],
  );
  const percent = $derived(
    details && details.total_size > 0
      ? Math.min(100, (details.downloaded / details.total_size) * 100)
      : 0,
  );

  async function refresh() {
    const target = id;
    const seq = ++requestSeq;
    try {
      const next = await invoke<DownloadDetails>('get_download_details', { id: target });
      if (seq === requestSeq) {
        details = next;
        loadError = null;
      }
    } catch (error) {
      if (seq === requestSeq) {
        loadError = String(error);
      }
    }
  }

  function scheduleRefresh() {
    if (debounceTimer) window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(() => {
      debounceTimer = undefined;
      void refresh();
    }, 350);
  }

  async function openFile(path: string) {
    actionError = null;
    try {
      await invoke('open_file', { path });
    } catch (error) {
      actionError = `${t('details.action_failed')}: ${String(error)}`;
    }
  }

  async function showInFolder(path: string) {
    actionError = null;
    try {
      await invoke('show_in_folder', { path });
    } catch (error) {
      actionError = `${t('details.action_failed')}: ${String(error)}`;
    }
  }

  async function openUrl(url: string) {
    actionError = null;
    try {
      await invoke('open_url', { url });
    } catch (error) {
      actionError = `${t('details.action_failed')}: ${String(error)}`;
    }
  }

  function formatSize(bytes: number | undefined | null): string {
    if (bytes == null) return '—';
    if (bytes <= 0) return '0 B';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function formatDuration(seconds: number | undefined | null): string {
    if (!seconds || seconds <= 0) return '—';
    const total = Math.round(seconds);
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    if (hours > 0) {
      return `${hours}:${String(minutes).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
    }
    return `${minutes}:${String(secs).padStart(2, '0')}`;
  }

  function formatBitrate(value: number | undefined | null): string {
    if (!value || value <= 0) return '—';
    const kbps = value >= 100000 ? value / 1000 : value;
    return `${Math.round(kbps)} ${t('speed.kbps')}`;
  }

  function formatDate(timestamp: number | undefined | null): string {
    if (!timestamp) return '—';
    const formatter = new Intl.DateTimeFormat(locale.current === 'ru' ? 'ru-RU' : 'en-US', {
      dateStyle: 'medium',
      timeStyle: 'short',
    });
    return formatter.format(new Date(timestamp * 1000));
  }

  function formatRatio(value: number | undefined | null): string {
    if (value == null) return '—';
    return value.toFixed(2);
  }

  function statusLabel(status: DownloadStatus): string {
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

  $effect(() => {
    void id;
    details = null;
    loadError = null;
    actionError = null;
    activeTab = 'file';
    void refresh();

    const timer = window.setInterval(() => {
      void refresh();
    }, 2000);

    return () => {
      window.clearInterval(timer);
      if (debounceTimer) window.clearTimeout(debounceTimer);
      requestSeq++;
    };
  });

  $effect(() => {
    const item = liveItem;
    if (!item) return;
    void item.downloaded;
    void item.status;
    scheduleRefresh();
  });

  $effect(() => {
    if (!tabs.includes(activeTab)) {
      activeTab = 'file';
    }
  });
</script>

<div class="details">
  <div class="head">
    <div class="head-main">
      <span class="title" title={title}>{title}</span>
      {#if kind !== 'file'}
        <span class="kind-pill {kind}">{t(`kind.${kind}`)}</span>
      {/if}
    </div>
    <button class="close-btn" onclick={onClose} title={t('actions.close')} aria-label={t('actions.close')}>
      <Icon name="close" size={14} />
    </button>
  </div>

  <div class="tabs" role="tablist">
    {#each tabs as tab (tab)}
      <button
        class="tab"
        class:active={activeTab === tab}
        role="tab"
        aria-selected={activeTab === tab}
        onclick={() => (activeTab = tab)}
      >
        {t(`details.tabs.${tab}`)}
      </button>
    {/each}
  </div>

  <div class="body">
    {#if !details && loadError}
      <p class="state error">{t('details.load_error')}: {loadError}</p>
    {:else if !details}
      <p class="state">{t('details.loading')}</p>
    {:else if activeTab === 'file'}
      <dl class="grid">
        <div>
          <dt>{t('details.filename')}</dt>
          <dd title={details.filename}>{details.filename}</dd>
        </div>
        <div class="wide">
          <dt>{t('details.path')}</dt>
          <dd class="mono break">{details.file_path ?? '—'}</dd>
        </div>
        <div>
          <dt>{t('details.total_size')}</dt>
          <dd>{formatSize(details.total_size)}</dd>
        </div>
        <div>
          <dt>{t('details.downloaded')}</dt>
          <dd>{formatSize(details.downloaded)} ({percent.toFixed(0)}%)</dd>
        </div>
        <div>
          <dt>{t('detail.status')}</dt>
          <dd>{statusLabel(details.status)}</dd>
        </div>
        <div>
          <dt>{t('detail.added')}</dt>
          <dd>{formatDate(details.added_at)}</dd>
        </div>
        <div>
          <dt>{t('details.category')}</dt>
          <dd>{t(`type.${details.category}`)}</dd>
        </div>
        <div>
          <dt>{t('details.connections')}</dt>
          <dd>{details.connections}</dd>
        </div>
        <div class="wide">
          <dt>{t('detail.url')}</dt>
          <dd class="mono break">{details.url}</dd>
        </div>
      </dl>

      {#if details.error}
        {@const errorUrl = details.url}
        <p class="state error">{details.error}</p>
        {#if errorUrl.startsWith('http://') || errorUrl.startsWith('https://')}
          <div class="actions">
            <button class="btn" onclick={() => openUrl(errorUrl)}>
              <Icon name="external" size={14} />
              <span>{t('actions.open_page')}</span>
            </button>
          </div>
          <p class="hint">{t('details.captcha_hint')}</p>
        {/if}
      {/if}

      {#if details.status === 'completed' && details.file_path}
        {@const path = details.file_path}
        <div class="actions">
          <button class="btn" onclick={() => openFile(path)}>
            <Icon name="external" size={14} />
            <span>{t('actions.open')}</span>
          </button>
          <button class="btn" onclick={() => showInFolder(path)}>
            <Icon name="folder" size={14} />
            <span>{t('actions.folder')}</span>
          </button>
        </div>
      {/if}

      {#if actionError}
        <p class="state error">{actionError}</p>
      {/if}
    {:else if activeTab === 'video'}
      {#if details.media}
        <dl class="grid">
          <div>
            <dt>{t('details.resolution')}</dt>
            <dd>{details.media.width} × {details.media.height}</dd>
          </div>
          <div>
            <dt>{t('details.video_codec')}</dt>
            <dd>{details.media.video_codec ?? '—'}</dd>
          </div>
          <div>
            <dt>{t('details.audio_codec')}</dt>
            <dd>{details.media.audio_codec ?? '—'}</dd>
          </div>
          <div>
            <dt>{t('details.duration')}</dt>
            <dd>{formatDuration(details.media.duration_secs)}</dd>
          </div>
          <div>
            <dt>{t('details.bitrate')}</dt>
            <dd>{formatBitrate(details.media.bitrate)}</dd>
          </div>
          <div>
            <dt>{t('details.container')}</dt>
            <dd>{details.media.container ?? '—'}</dd>
          </div>
          <div>
            <dt>{t('details.frame_rate')}</dt>
            <dd>{details.media.frame_rate ?? '—'}</dd>
          </div>
        </dl>
      {:else if details.status === 'completed'}
        <p class="state">{t('details.media_unavailable')}</p>
      {:else}
        <p class="state">{t('details.media_pending')}</p>
      {/if}
    {:else}
      {#if details.torrent}
        <div class="torrent-stats">
          <div class="stat">
            <span class="stat-label">{t('details.downloaded')}</span>
            <span class="stat-value">{formatSize(details.torrent.downloaded)}</span>
          </div>
          <div class="stat">
            <span class="stat-label">{t('details.uploaded')}</span>
            <span class="stat-value">{formatSize(details.torrent.uploaded)}</span>
          </div>
          <div class="stat">
            <span class="stat-label">{t('details.ratio')}</span>
            <span class="stat-value">{formatRatio(details.torrent.ratio)}</span>
          </div>
          <div class="stat">
            <span class="stat-label">{t('details.peers_live')}</span>
            <span class="stat-value">{details.torrent.peers_live}</span>
          </div>
          <div class="stat">
            <span class="stat-label">{t('details.peers_seen')}</span>
            <span class="stat-value">{details.torrent.peers_seen}</span>
          </div>
        </div>
      {/if}

      {#if peers.length === 0}
        <p class="state">{t('details.peers.empty')}</p>
      {:else}
        <div class="peer-scroll">
          <table class="peers">
            <thead>
              <tr>
                <th>{t('details.peers.address')}</th>
                <th>{t('details.peers.state')}</th>
                <th>{t('details.peers.client')}</th>
                <th>{t('details.peers.downloaded')}</th>
                <th>{t('details.peers.uploaded')}</th>
              </tr>
            </thead>
            <tbody>
              {#each peers as peer}
                <tr>
                  <td class="mono">{peer.address}</td>
                  <td>{peer.state}</td>
                  <td>{peer.client ?? '—'}</td>
                  <td>{formatSize(peer.downloaded)}</td>
                  <td>{formatSize(peer.uploaded)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .details {
    height: 280px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-secondary);
    border-top: 1px solid var(--border-color);
    animation: details-in 0.2s ease;
  }

  @keyframes details-in {
    from { opacity: 0; transform: translateY(10px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 40px;
    padding: 0 12px 0 16px;
    border-bottom: 1px solid var(--border-color);
    flex-shrink: 0;
  }

  .head-main {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind-pill {
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--bg-tertiary);
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 600;
    flex-shrink: 0;
  }

  .kind-pill.torrent {
    color: var(--warning);
  }

  .kind-pill.video {
    color: var(--accent);
  }

  .close-btn {
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
    transition: all var(--transition-fast);
    flex-shrink: 0;
  }

  .close-btn:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .tabs {
    display: flex;
    gap: 2px;
    padding: 5px 12px 0;
    border-bottom: 1px solid var(--border-color);
    flex-shrink: 0;
  }

  .tab {
    padding: 6px 12px 7px;
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .tab:hover {
    color: var(--text-primary);
  }

  .tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 16px 16px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 10px 22px;
    margin: 0;
  }

  .grid > div {
    min-width: 0;
  }

  .grid .wide {
    grid-column: 1 / -1;
  }

  dt {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--text-muted);
    margin-bottom: 2px;
  }

  dd {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  dd.break {
    white-space: normal;
    word-break: break-all;
  }

  .mono {
    font-family: ui-monospace, 'Cascadia Code', Consolas, monospace;
    font-size: 11.5px;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 11px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 12px;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .btn:hover {
    background: var(--bg-tertiary);
  }

  .state {
    margin: 0;
    padding: 18px 0;
    text-align: center;
    color: var(--text-muted);
    font-size: 12.5px;
  }

  .hint {
    margin: 6px 0 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--text-muted);
  }

  .state.error {
    color: var(--error);
    text-align: left;
    padding: 8px 0 0;
    word-break: break-word;
  }

  .torrent-stats {
    display: flex;
    flex-wrap: wrap;
    gap: 20px;
    padding-bottom: 12px;
    margin-bottom: 8px;
    border-bottom: 1px solid var(--border-color);
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .stat-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--text-muted);
  }

  .stat-value {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .peer-scroll {
    overflow: auto;
  }

  .peers {
    width: 100%;
    border-collapse: collapse;
    font-size: 11.5px;
  }

  .peers th {
    position: sticky;
    top: 0;
    padding: 5px 10px;
    text-align: left;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--text-muted);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border-color);
    white-space: nowrap;
  }

  .peers td {
    padding: 5px 10px;
    color: var(--text-secondary);
    border-bottom: 1px solid var(--border-color);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
</style>
