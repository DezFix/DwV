<script lang="ts">
  import DownloadItem, { type SelectEvent } from './DownloadItem.svelte';
  import { downloadsStore, type DownloadCategory } from '../stores/downloads.svelte.ts';
  import { visibleDownloads, type SortBy } from '../stores/listView.ts';
  import { t } from '../i18n/index.svelte.ts';
  import type { View } from './Sidebar.svelte';

  interface Props {
    view: View;
    category: DownloadCategory | null;
    sortBy: SortBy;
    sortDir: 'asc' | 'desc';
    query: string;
    selectedId: number | null;
    checkedIds: number[];
    onSelect: (id: number, event: SelectEvent) => void;
    onCheck: (id: number) => void;
    onRemove: (id: number) => void;
    onError: (message: string) => void;
  }

  let { view, category, sortBy, sortDir, query, selectedId, checkedIds, onSelect, onCheck, onRemove, onError }: Props = $props();

  const filtered = $derived(
    visibleDownloads(downloadsStore.items, view, category, query, sortBy, sortDir),
  );
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
