<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';
  import type { TorrentMeta } from '../stores/downloads.svelte.ts';

  interface Props {
    open: boolean;
    source: string;
    filename: string;
    dir: string;
    onCancel: () => void;
    onConfirm: (dir: string, onlyFiles: number[] | null, hide: boolean) => void;
  }

  interface FileNode {
    key: string;
    name: string;
    depth: number;
    dir: boolean;
    size: number;
    index?: number;
    indices: number[];
    parent: string;
  }

  let { open: isOpen, source, filename, dir, onCancel, onConfirm }: Props = $props();

  let folder = $state('');
  let hide = $state(false);
  let meta = $state<TorrentMeta | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let checked = $state<Set<number>>(new Set());
  let collapsed = $state<Set<string>>(new Set());

  const nodes = $derived(buildNodes(meta));
  const rows = $derived(flatten(nodes, collapsed));
  const selectedSize = $derived(
    meta ? meta.files.filter(file => checked.has(file.index)).reduce((sum, file) => sum + file.size, 0) : 0,
  );
  const fileCount = $derived(meta?.files.length ?? 0);
  const infoHash = $derived(meta ? meta.info_hash.slice(0, 12) : '');

  $effect(() => {
    if (!isOpen) return;
    folder = dir;
    hide = false;
    void load(dir);
  });

  async function load(targetDir: string) {
    loading = true;
    error = null;
    meta = null;
    checked = new Set();
    collapsed = new Set();
    try {
      const next = await invoke<TorrentMeta>('inspect_torrent', { source, dir: targetDir });
      meta = next;
      checked = new Set(next.files.map(file => file.index));
    } catch (failure) {
      error = String(failure);
    } finally {
      loading = false;
    }
  }

  function buildNodes(value: TorrentMeta | null): Map<string, FileNode> {
    const map = new Map<string, FileNode>();
    if (!value) return map;
    for (const file of value.files) {
      const parts = file.path.split(/[\\/]+/).filter(Boolean);
      let parent = '';
      parts.forEach((part, position) => {
        const last = position === parts.length - 1;
        const key = parent ? `${parent}/${part}` : part;
        let node = map.get(key);
        if (!node) {
          node = {
            key,
            name: part,
            depth: position,
            dir: !last,
            size: 0,
            index: last ? file.index : undefined,
            indices: [],
            parent,
          };
          map.set(key, node);
        }
        node.size += file.size;
        if (last) {
          node.dir = false;
          node.index = file.index;
        }
        parent = key;
      });
    }
    for (const node of map.values()) {
      if (node.dir || node.index === undefined) continue;
      let parent = node.parent;
      while (parent) {
        map.get(parent)?.indices.push(node.index);
        parent = map.get(parent)?.parent ?? '';
      }
    }
    for (const node of map.values()) node.indices.sort((a, b) => a - b);
    return map;
  }

  function flatten(nodesMap: Map<string, FileNode>, closed: Set<string>): FileNode[] {
    const out: FileNode[] = [];
    const children = new Map<string, FileNode[]>();
    for (const node of nodesMap.values()) {
      if (node.parent) {
        const list = children.get(node.parent) ?? [];
        list.push(node);
        children.set(node.parent, list);
      }
    }
    const sortNodes = (list: FileNode[]) =>
      list.sort((a, b) => (a.dir === b.dir ? a.name.localeCompare(b.name) : a.dir ? -1 : 1));
    const walk = (node: FileNode) => {
      out.push(node);
      if (node.dir && !closed.has(node.key)) {
        for (const child of sortNodes(children.get(node.key) ?? [])) walk(child);
      }
    };
    for (const root of sortNodes([...nodesMap.values()].filter(node => !node.parent))) walk(root);
    return out;
  }

  function stateOf(node: FileNode): 'all' | 'some' | 'none' {
    if (node.indices.length === 0) return 'none';
    const count = node.indices.filter(index => checked.has(index)).length;
    if (count === 0) return 'none';
    if (count === node.indices.length) return 'all';
    return 'some';
  }

  function toggle(node: FileNode) {
    const next = new Set(checked);
    if (node.dir) {
      const all = stateOf(node) === 'all';
      for (const index of node.indices) {
        if (all) next.delete(index);
        else next.add(index);
      }
    } else if (node.index !== undefined) {
      if (next.has(node.index)) next.delete(node.index);
      else next.add(node.index);
    }
    checked = next;
  }

  function toggleCollapse(node: FileNode) {
    const next = new Set(collapsed);
    if (next.has(node.key)) next.delete(node.key);
    else next.add(node.key);
    collapsed = next;
  }

  function selectAll(all: boolean) {
    if (!meta) return;
    checked = all ? new Set(meta.files.map(file => file.index)) : new Set();
  }

  function confirmDownload(allFiles = false) {
    const target = folder.trim();
    if (!target) return;
    let onlyFiles: number[] | null = null;
    if (!allFiles && meta) {
      if (checked.size === 0) return;
      if (checked.size < meta.files.length) {
        onlyFiles = [...checked].sort((a, b) => a - b);
      }
    }
    onConfirm(target, onlyFiles, hide);
  }

  async function browse() {
    const picked = await open({
      directory: true,
      defaultPath: folder || undefined,
      title: t('adddialog.folder'),
    });
    if (typeof picked === 'string') folder = picked;
  }

  function formatSize(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!isOpen) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      onCancel();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if isOpen}
  <div class="overlay" role="presentation" onclick={onCancel}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="torrent-dialog-title"
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
    >
      <button class="close" onclick={onCancel} title={t('actions.close')} aria-label={t('actions.close')}>
        <Icon name="close" size={14} />
      </button>

      <div class="badge">
        <Icon name="archive" size={20} />
      </div>

      <h2 id="torrent-dialog-title">{t('torrent.title')}</h2>
      <p class="name" title={filename}>{meta?.name || filename}</p>
      <p class="meta">
        {#if meta}
          {t('torrent.files_count').replace('{n}', String(fileCount))} · {infoHash}
        {:else if loading}
          {t('torrent.loading')}
        {:else}
          {t('torrent.files_count').replace('{n}', '0')}
        {/if}
      </p>

      {#if loading}
        <div class="state">{t('torrent.loading')}</div>
      {:else if error}
        <div class="state error">{error}</div>
      {:else if meta}
        <div class="controls">
          <button class="link" onclick={() => selectAll(true)}>{t('torrent.all')}</button>
          <button class="link" onclick={() => selectAll(false)}>{t('torrent.none')}</button>
          <span class="selected">
            {t('torrent.selected')
              .replace('{size}', formatSize(selectedSize))
              .replace('{total}', formatSize(meta.total_size))}
          </span>
        </div>

        <div class="files">
          {#each rows as node (node.key)}
            <div class="file" class:dir={node.dir} style="padding-left: {node.depth * 14}px">
              {#if node.dir}
                <button
                  class="chevron"
                  onclick={() => toggleCollapse(node)}
                  aria-label={collapsed.has(node.key) ? t('torrent.expand') : t('torrent.collapse')}
                  title={collapsed.has(node.key) ? t('torrent.expand') : t('torrent.collapse')}
                >
                  {collapsed.has(node.key) ? '▸' : '▾'}
                </button>
              {:else}
                <span class="chevron-space"></span>
              {/if}

              <input
                type="checkbox"
                checked={node.dir ? stateOf(node) === 'all' : node.index !== undefined && checked.has(node.index)}
                indeterminate={node.dir && stateOf(node) === 'some'}
                onchange={() => toggle(node)}
                aria-label={node.name}
              />

              <span class="file-name" title={node.dir ? node.name : node.key}>{node.name}</span>
              <span class="file-size">{formatSize(node.size)}</span>
            </div>
          {/each}
        </div>

        <span class="label">{t('adddialog.folder')}</span>
        <div class="folder">
          <input type="text" class="input" bind:value={folder} placeholder="—" />
          <button class="btn" onclick={browse}>
            <Icon name="folder" size={15} />
            <span>{t('settings.browse')}</span>
          </button>
        </div>
      {/if}

      <div class="footer">
        <label class="remember">
          <input type="checkbox" bind:checked={hide} />
          <span>{t('adddialog.dont_show')}</span>
        </label>
        <div class="buttons">
          <button class="btn ghost" onclick={onCancel}>{t('adddialog.cancel')}</button>
          {#if error}
            <button class="btn primary" onclick={() => confirmDownload(true)}>
              {t('torrent.all_files')}
            </button>
          {:else}
            <button
              class="btn primary"
              onclick={() => confirmDownload()}
              disabled={loading || !meta || checked.size === 0 || !folder.trim()}
            >
              {t('adddialog.download')}
            </button>
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 1200;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.45);
    animation: overlay-in 140ms ease;
  }

  @keyframes overlay-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .dialog {
    position: relative;
    width: min(560px, 100%);
    max-height: calc(100vh - 48px);
    display: flex;
    flex-direction: column;
    padding: 24px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    animation: dialog-pop 170ms ease;
  }

  @keyframes dialog-pop {
    from { opacity: 0; transform: translateY(8px) scale(0.97); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  .close {
    position: absolute;
    top: 10px;
    right: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .close:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .badge {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    margin-bottom: 14px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--warning) 14%, transparent);
    color: var(--warning);
  }

  h2 {
    margin: 0 0 4px;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    padding-right: 18px;
  }

  .name {
    margin: 0 0 2px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    margin: 0 0 12px;
    font-size: 11.5px;
    color: var(--text-muted);
  }

  .state {
    padding: 18px 0;
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .state.error {
    color: var(--error);
    word-break: break-word;
  }

  .controls {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 7px;
  }

  .link {
    padding: 0;
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
  }

  .link:hover {
    text-decoration: underline;
  }

  .selected {
    margin-left: auto;
    font-size: 11.5px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .files {
    max-height: 300px;
    overflow-y: auto;
    scrollbar-width: thin;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    padding: 4px 0;
  }

  .file {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 9px;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .file.dir {
    color: var(--text-primary);
    font-weight: 600;
  }

  .chevron {
    width: 14px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 11px;
    cursor: pointer;
  }

  .chevron-space {
    width: 14px;
    flex-shrink: 0;
  }

  .file input[type='checkbox'] {
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    flex-shrink: 0;
    cursor: pointer;
  }

  .file-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-size {
    color: var(--text-muted);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .label {
    display: block;
    margin: 12px 0 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .folder {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .input {
    flex: 1;
    min-width: 0;
    padding: 7px 10px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 12.5px;
    outline: none;
  }

  .input:focus {
    border-color: var(--accent);
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 18px;
  }

  .remember {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--text-secondary);
    cursor: pointer;
    user-select: none;
  }

  .remember input {
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .buttons {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 12.5px;
    cursor: pointer;
    transition: background var(--transition-fast), border-color var(--transition-fast);
  }

  .btn:hover {
    background: var(--bg-tertiary);
  }

  .btn.primary {
    border-color: var(--accent);
    background: var(--accent);
    color: #fff;
    font-weight: 600;
  }

  .btn.primary:hover {
    filter: brightness(1.08);
  }

  .btn.primary:disabled {
    opacity: 0.5;
    cursor: default;
    filter: none;
  }
</style>
