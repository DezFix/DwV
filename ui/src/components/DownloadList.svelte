<script lang="ts">
  import DownloadItem from './DownloadItem.svelte';
  import { downloadsStore, type DownloadItem as DownloadItemType, type DownloadCategory } from '../stores/downloads.svelte.ts';
  import { t } from '../i18n/index.svelte.ts';
  import type { View } from './Sidebar.svelte';

  interface Props {
    view: View;
    category: DownloadCategory | null;
    sortBy: 'added' | 'name' | 'progress' | 'speed' | 'size';
    sortDir: 'asc' | 'desc';
    query: string;
    selectedId: number | null;
    checkedIds: number[];
    onSelect: (id: number) => void;
    onCheck: (id: number) => void;
    onRemove: (id: number) => void;
    onError: (message: string) => void;
  }

  let { view, category, sortBy, sortDir, query, selectedId, checkedIds, onSelect, onCheck, onRemove, onError }: Props = $props();

  const filtered = $derived(sortItems(searchItems(filterByStatus(filterByCategory(downloadsStore.items)))));

  function searchItems(items: DownloadItemType[]) {
    const needle = query.trim().toLowerCase();
    if (!needle) return items;
    return items.filter(
      item =>
        item.filename.toLowerCase().includes(needle) || item.url.toLowerCase().includes(needle),
    );
  }

  function filterByCategory(items: DownloadItemType[]) {
    if (!category) return items;
    return items.filter(item => item.category === category);
  }

  function filterByStatus(items: DownloadItemType[]) {
    if (view === 'all') return items;
    if (view === 'torrent') return items.filter(item => item.kind === 'torrent');
    if (view === 'video') {
      return items.filter(item => item.kind === 'video' || item.category === 'video');
    }
    return items.filter(item => item.status === view);
  }

  function sortItems(items: DownloadItemType[]) {
    const value = (item: DownloadItemType): string | number => {
      switch (sortBy) {
        case 'name': return item.filename.toLowerCase();
        case 'progress': return item.total_size > 0 ? item.downloaded / item.total_size : 0;
        case 'speed': return item.speed;
        case 'size': return item.total_size;
        case 'added': return item.added_at;
      }
    };

    return [...items].sort((a, b) => {
      const av = value(a);
      const bv = value(b);
      if (av < bv) return sortDir === 'asc' ? -1 : 1;
      if (av > bv) return sortDir === 'asc' ? 1 : -1;
      return 0;
    });
  }
</script>

<div class="content">
  {#if filtered.length === 0}
    <div class="empty">
      <div class="empty-art">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
          <polyline points="7 10 12 15 17 10"/>
          <line x1="12" y1="15" x2="12" y2="3"/>
        </svg>
      </div>
      <p class="empty-title">{t('empty.title')}</p>
      <p class="empty-sub">{t('empty.subtitle')}</p>
    </div>
  {:else}
    <div class="list" role="list">
      {#each filtered as item (item.id)}
        <DownloadItem
          {item}
          selected={item.id === selectedId}
          checked={checkedIds.includes(item.id)}
          {onSelect}
          {onCheck}
          {onRemove}
          {onError}
        />
      {/each}
    </div>
  {/if}
</div>

<style>
  .content {
    flex: 1;
    overflow-y: auto;
    background: var(--bg-elevated);
  }

  .list {
    display: flex;
    flex-direction: column;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 40px;
    text-align: center;
  }

  .empty-art {
    width: 72px;
    height: 72px;
    margin-bottom: 18px;
    color: var(--text-muted);
    opacity: 0.45;
  }

  .empty-art svg {
    width: 100%;
    height: 100%;
  }

  .empty-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: 6px;
  }

  .empty-sub {
    font-size: 12.5px;
    color: var(--text-muted);
    max-width: 340px;
  }
</style>
