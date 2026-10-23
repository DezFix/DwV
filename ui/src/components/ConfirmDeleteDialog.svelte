<script lang="ts">
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';

  type DeleteChoice = 'keep' | 'delete';

  interface Props {
    open: boolean;
    count: number;
    filename?: string;
    onCancel: () => void;
    onConfirm: (mode: DeleteChoice, remember: boolean) => void;
  }

  let { open, count, filename, onCancel, onConfirm }: Props = $props();

  let remember = $state(false);

  const title = $derived(
    count === 1
      ? t('delete.title_single')
      : t('delete.title_bulk').replace('{n}', String(count)),
  );

  $effect(() => {
    if (!open) remember = false;
  });

  function confirm(mode: DeleteChoice) {
    onConfirm(mode, remember);
  }

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
      aria-labelledby="delete-dialog-title"
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
    >
      <button class="close" onclick={onCancel} title={t('actions.close')} aria-label={t('actions.close')}>
        <Icon name="close" size={14} />
      </button>

      <div class="badge">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <polyline points="3 6 5 6 21 6" />
          <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
        </svg>
      </div>

      <h2 id="delete-dialog-title">{title}</h2>

      {#if count === 1 && filename}
        <p class="filename" title={filename}>{filename}</p>
      {/if}

      <p class="body">{count === 1 ? t('delete.body_single') : t('delete.body_bulk')}</p>

      <div class="choices">
        <button class="choice" onclick={() => confirm('keep')}>
          <span class="choice-title">{t('delete.keep')}</span>
          <span class="choice-sub">{t('delete.keep_hint')}</span>
        </button>
        <button class="choice danger" onclick={() => confirm('delete')}>
          <span class="choice-title">{t('delete.files')}</span>
          <span class="choice-sub">{t('delete.files_hint')}</span>
        </button>
      </div>

      <div class="footer">
        <label class="remember">
          <input type="checkbox" bind:checked={remember} />
          <span>{t('delete.remember')}</span>
        </label>
        <button class="btn ghost" onclick={onCancel}>{t('delete.cancel')}</button>
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
    width: min(430px, 100%);
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
    transition: background var(--transition-fast), color var(--transition-fast);
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
    background: color-mix(in srgb, var(--error) 14%, transparent);
    color: var(--error);
  }

  .badge svg {
    width: 19px;
    height: 19px;
  }

  h2 {
    margin: 0 0 8px;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    padding-right: 18px;
  }

  .filename {
    margin: 0 0 8px;
    padding: 7px 10px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-family: ui-monospace, 'Cascadia Code', Consolas, monospace;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .body {
    margin: 0 0 16px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text-secondary);
  }

  .choices {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .choice {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 11px 13px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    background: var(--bg-primary);
    text-align: left;
    cursor: pointer;
    transition: border-color var(--transition-fast), background var(--transition-fast);
  }

  .choice:hover {
    border-color: var(--accent);
    background: var(--bg-hover);
  }

  .choice-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .choice-sub {
    font-size: 11.5px;
    color: var(--text-muted);
  }

  .choice.danger {
    border-color: color-mix(in srgb, var(--error) 35%, var(--border-color));
  }

  .choice.danger:hover {
    border-color: var(--error);
    background: color-mix(in srgb, var(--error) 9%, transparent);
  }

  .choice.danger .choice-title {
    color: var(--error);
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

  .btn {
    padding: 7px 14px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 12.5px;
    cursor: pointer;
    transition: background var(--transition-fast);
  }

  .btn:hover {
    background: var(--bg-tertiary);
  }
</style>
