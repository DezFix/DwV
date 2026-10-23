<script lang="ts">
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';

  interface Props {
    open: boolean;
    version: string;
    notes?: string;
    busy: boolean;
    error?: string | null;
    onUpdate: () => void;
    onSkip: () => void;
  }

  let { open, version, notes, busy, error, onUpdate, onSkip }: Props = $props();

  function handleKeydown(event: KeyboardEvent) {
    if (!open || busy) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      onSkip();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <div class="overlay" role="presentation" onclick={() => !busy && onSkip()}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="update-title"
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
    >
      <button class="close" onclick={onSkip} disabled={busy} title={t('actions.close')} aria-label={t('actions.close')}>
        <Icon name="close" size={14} />
      </button>

      <div class="badge">
        <Icon name="download" size={20} />
      </div>

      <h2 id="update-title">{t('update.title')}</h2>
      <p class="body">{t('update.available')} <strong>{version}</strong></p>

      {#if notes}
        <pre class="notes">{notes}</pre>
      {/if}

      {#if error}
        <p class="error">{t('update.failed')}: {error}</p>
      {/if}

      <div class="actions">
        <button class="btn ghost" onclick={onSkip} disabled={busy}>{t('update.skip')}</button>
        <button class="btn primary" onclick={onUpdate} disabled={busy}>
          {busy ? t('update.busy') : t('update.now')}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 1300;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.5);
    animation: overlay-in 140ms ease;
  }

  @keyframes overlay-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .dialog {
    position: relative;
    width: min(470px, 100%);
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

  .close:disabled { cursor: default; opacity: 0.4; }
  .close:not(:disabled):hover { background: var(--bg-tertiary); color: var(--text-primary); }

  .badge {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 42px;
    height: 42px;
    margin-bottom: 14px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
  }

  h2 {
    margin: 0 0 8px;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    padding-right: 18px;
  }

  .body {
    margin: 0 0 12px;
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .notes {
    max-height: 170px;
    overflow: auto;
    margin: 0 0 14px;
    padding: 9px 11px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    color: var(--text-secondary);
    font: 11.5px/1.5 ui-monospace, 'Cascadia Code', Consolas, monospace;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .error {
    margin: 0 0 12px;
    font-size: 12px;
    color: var(--error);
    word-break: break-word;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
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

  .btn:hover:not(:disabled) { background: var(--bg-tertiary); }
  .btn:disabled { opacity: 0.6; cursor: default; }

  .btn.primary {
    border-color: var(--accent);
    background: var(--accent);
    color: #fff;
    font-weight: 600;
  }

  .btn.primary:hover:not(:disabled) { background: var(--accent-hover); }
</style>
