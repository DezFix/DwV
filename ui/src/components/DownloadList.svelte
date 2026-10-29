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
    allChecked: boolean;
    someChecked: boolean;
    onSort: (key: SortBy) => void;
    onToggleAll: () => void;
    onSelect: (id: number, event: SelectEvent) => void;
    onCheck: (id: number) => void;
    onRemove: (id: number) => void;
    onError: (message: string) => void;
  }

  let {
    view,
    category,
    sortBy,
    sortDir,
    query,
    selectedId,
    checkedIds,
    allChecked,
    someChecked,
    onSort,
    onToggleAll,
    onSelect,
    onCheck,
    onRemove,
    onError,
  }: Props = $props();

  const filtered = $derived(
    visibleDownloads(downloadsStore.items, view, category, query, sortBy, sortDir),
  );

  function mark(key: SortBy): string {
    if (sortBy !== key) return '';
    return sortDir === 'asc' ? ' ↑' : ' ↓';
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
    <div class="head" role="presentation">
      <span class="head-check">
        <input
          type="checkbox"
          checked={allChecked}
          indeterminate={someChecked && !allChecked}
          onchange={onToggleAll}
          title={t('bulk.select_all')}
          aria-label={t('bulk.select_all')}
        />
      </span>
      <button class="head-cell cell-name" onclick={() => onSort('name')}>{t('header.name')}{mark('name')}</button>
      <button class="head-cell right cell-size" onclick={() => onSort('size')}>{t('header.size')}{mark('size')}</button>
      <span class="head-cell cell-status">{t('header.status')}</span>
      <button class="head-cell cell-progress" onclick={() => onSort('progress')}>{t('header.progress')}{mark('progress')}</button>
      <button class="head-cell right cell-speed" onclick={() => onSort('speed')}>{t('header.speed')}{mark('speed')}</button>
      <span class="head-cell right cell-eta">{t('header.eta')}</span>
      <button class="head-cell right cell-added" onclick={() => onSort('added')}>{t('header.added')}{mark('added')}</button>
      <span></span>
    </div>

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
    --row-grid: 30px minmax(0, 1fr) 78px 112px 186px 78px 66px 112px 84px;
    flex: 1;
    overflow-y: auto;
    background: var(--bg-elevated);
  }

  .head {
    position: sticky;
    top: 0;
    z-index: 2;
    display: grid;
    grid-template-columns: var(--row-grid);
    align-items: center;
    gap: 10px;
    height: 30px;
    padding: 0 14px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-color);
  }

  .head-check {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .head-check input {
    width: 13px;
    height: 13px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .head-cell {
    padding: 0;
    border: none;
    background: transparent;
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--text-muted);
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  button.head-cell {
    cursor: pointer;
    transition: color var(--transition-fast);
  }

  button.head-cell:hover {
    color: var(--text-primary);
  }

  .right {
    text-align: right;
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

  @media (max-width: 1180px) {
    .content {
      --row-grid: 30px minmax(0, 1fr) 78px 112px 186px 78px 66px 84px;
    }
    .cell-added {
      display: none;
    }
  }

  @media (max-width: 1040px) {
    .content {
      --row-grid: 30px minmax(0, 1fr) 78px 112px 186px 78px 84px;
    }
    .cell-eta {
      display: none;
    }
  }

  @media (max-width: 920px) {
    .content {
      --row-grid: 30px minmax(0, 1fr) 112px 186px 78px 84px;
    }
    .cell-size {
      display: none;
    }
  }

  @media (max-width: 820px) {
    .content {
      --row-grid: 30px minmax(0, 1fr) 186px 78px 84px;
    }
    .cell-status {
      display: none;
    }
  }
</style>
