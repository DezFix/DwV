<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';

  interface Props {
    onClose: () => void;
    onError: (message: string) => void;
  }

  let { onClose, onError }: Props = $props();

  async function support() {
    try {
      await invoke('open_url', { url: 'https://ko-fi.com/dezfix' });
      onClose();
    } catch (error) {
      onError(`${t('kofi.error')}: ${String(error)}`);
    }
  }
</script>

<div class="kofi-banner">
  <span class="heart"><Icon name="heart" size={15} /></span>
  <span class="text">{t('kofi.banner')}</span>
  <button class="kofi-btn" onclick={support}>{t('kofi.button')}</button>
  <button class="close" onclick={onClose} title={t('error.dismiss')} aria-label={t('error.dismiss')}>
    <Icon name="close" size={13} />
  </button>
</div>

<style>
  .kofi-banner {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 7px 12px;
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-elevated));
    border-bottom: 1px solid var(--border-color);
    flex-shrink: 0;
    animation: kofi-in 0.18s ease;
  }

  @keyframes kofi-in {
    from { opacity: 0; transform: translateY(-8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .heart {
    display: flex;
    align-items: center;
    color: var(--error);
    flex-shrink: 0;
  }

  .text {
    flex: 1;
    font-size: 12px;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kofi-btn {
    padding: 4px 11px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    flex-shrink: 0;
    transition: background var(--transition-fast);
  }

  .kofi-btn:hover {
    background: var(--accent-hover);
  }

  .close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    border-radius: var(--radius-sm);
    cursor: pointer;
    flex-shrink: 0;
    transition: background var(--transition-fast), color var(--transition-fast);
  }

  .close:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }
</style>
