<script lang="ts">
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';

  interface Props {
    open: boolean;
    filename: string;
    suggested: string;
    onCancel: () => void;
    onRename: () => void;
  }

  let { open, filename, suggested, onCancel, onRename }: Props = $props();

  function handleKeydown(event: KeyboardEvent) {
    if (!open) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      onCancel();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <div class="overlay" role="presentation" onclick={onCancel}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="duplicate-title"
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
    >
      <button class="close" onclick={onCancel} title={t('actions.close')} aria-label={t('actions.close')}>
        <Icon name="close" size={14} />
      </button>

      <div class="badge">
        <Icon name="archive" size={19} />
      </div>

      <h2 id="duplicate-title">{t('duplicate.title')}</h2>
      <p class="body">{t('duplicate.body')}</p>

      <dl class="names">
        <div>
          <dt>{t('duplicate.existing')}</dt>
          <dd title={filename}>{filename}</dd>
        </div>
        <div>
          <dt>{t('duplicate.suggested')}</dt>
          <dd title={suggested}>{suggested}</dd>
        </div>
      </dl>

      <div class="actions">
        <button class="btn ghost" onclick={onCancel}>{t('duplicate.cancel')}</button>
        <button class="btn primary" onclick={onRename}>{t('duplicate.rename')}</button>
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
    width: min(460px, 100%);
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
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--warning);
  }

  h2 {
    margin: 0 0 8px;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    padding-right: 18px;
  }

  .body {
    margin: 0 0 14px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text-secondary);
  }

  .names {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0 0 18px;
  }

  .names > div {
    padding: 8px 10px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
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
    font-family: ui-monospace, 'Cascadia Code', Consolas, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .btn {
    padding: 8px 14px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 12.5px;
    cursor: pointer;
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
    background: var(--accent-hover);
  }
</style>
