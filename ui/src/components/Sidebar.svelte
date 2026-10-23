<script lang="ts">
  import { downloadsStore, type DownloadCategory } from '../stores/downloads.svelte.ts';
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';
  import type { IconName } from '../assets/icons';

  export type View =
    | 'all'
    | 'downloading'
    | 'completed'
    | 'error'
    | 'pending'
    | 'paused'
    | 'torrent'
    | 'video';

  interface Props {
    view: View;
    category: DownloadCategory | null;
    onNavigate: (view: View, category: DownloadCategory | null) => void;
  }

  let { view, category, onNavigate }: Props = $props();

  const sections = $derived([
    { id: 'all' as View, label: 'filter.all', count: downloadsStore.items.length, icon: 'download' as IconName },
    { id: 'downloading' as View, label: 'filter.downloading', count: downloadsStore.activeCount, icon: 'download' as IconName },
    { id: 'pending' as View, label: 'filter.pending', count: downloadsStore.pending.length, icon: 'clock' as IconName },
    { id: 'paused' as View, label: 'filter.paused', count: downloadsStore.paused.length, icon: 'pause' as IconName },
    { id: 'completed' as View, label: 'filter.completed', count: downloadsStore.completed.length, icon: 'check' as IconName },
    { id: 'error' as View, label: 'filter.error', count: downloadsStore.errored.length, icon: 'close' as IconName },
  ]);

  const categories = $derived(
    (Object.keys(downloadsStore.categoryCounts) as DownloadCategory[]).map(cat => ({
      id: cat,
      label: `type.${cat}`,
      count: downloadsStore.categoryCounts[cat],
      icon: cat as IconName,
    })),
  );
</script>

<aside class="sidebar">
  <nav class="nav">
    <div class="nav-section">
      {#each sections as section (section.id)}
        <button
          class="nav-item"
          class:active={view === section.id && category === null}
          onclick={() => onNavigate(section.id, null)}
        >
          <span class="nav-icon"><Icon name={section.icon} size={16} /></span>
          <span class="nav-label">{t(section.label)}</span>
          {#if section.count > 0}
            <span class="nav-count">{section.count}</span>
          {/if}
        </button>
      {/each}
    </div>

    <div class="nav-section">
      <div class="nav-title">{t('sidebar.type')}</div>
      {#each categories as cat (cat.id)}
        <button
          class="nav-item"
          class:active={category === cat.id}
          onclick={() => onNavigate('all', cat.id)}
        >
          <span class="nav-icon"><Icon name={cat.icon} size={16} /></span>
          <span class="nav-label">{t(cat.label)}</span>
          {#if cat.count > 0}
            <span class="nav-count">{cat.count}</span>
          {/if}
        </button>
      {/each}
      <button
        class="nav-item"
        class:active={view === 'torrent' && category === null}
        onclick={() => onNavigate('torrent', null)}
      >
        <span class="nav-icon"><Icon name="archive" size={16} /></span>
        <span class="nav-label">{t('sidebar.torrents')}</span>
        {#if downloadsStore.torrentCount > 0}
          <span class="nav-count">{downloadsStore.torrentCount}</span>
        {/if}
      </button>
    </div>
  </nav>
</aside>

<style>
  .sidebar {
    width: 200px;
    flex-shrink: 0;
    background: var(--bg-secondary);
    border-right: 1px solid var(--border-color);
    overflow-y: auto;
    padding: 8px 0;
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .nav-section {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 4px 8px;
  }

  .nav-section + .nav-section {
    margin-top: 8px;
    padding-top: 12px;
    border-top: 1px solid var(--border-color);
  }

  .nav-title {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    color: var(--text-muted);
    padding: 4px 10px 6px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    text-align: left;
    width: 100%;
    transition: all var(--transition-fast);
    animation: slide-in-left 0.22s ease;
  }

  .nav-item:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .nav-item.active {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .nav-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    color: currentColor;
  }

  .nav-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .nav-count {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .nav-item.active .nav-count {
    color: var(--accent);
  }
</style>
