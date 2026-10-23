<script lang="ts">
  import { downloadsStore } from '../stores/downloads.svelte.ts';
  import { settingsStore } from '../stores/settings.svelte.ts';
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';

  function formatSpeed(bytes: number): string {
    if (bytes <= 0) return '0 B/s';
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB/s`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB/s`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB/s`;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  const enabled = $derived(settingsStore.speed_limit_enabled);
  const limit = $derived(Math.max(0.5, settingsStore.speed_limit_mbps || 2));

  function onLimitChange(event: Event) {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    settingsStore.save({ speed_limit_mbps: Math.max(0.5, Number.isFinite(value) ? value : 2) });
  }

  async function toggleSpeedLimit() {
    const enabling = !settingsStore.speed_limit_enabled;
    if (enabling && settingsStore.speed_limit_mbps < 0.5) {
      await settingsStore.save({ speed_limit_enabled: true, speed_limit_mbps: 2 });
      return;
    }
    await settingsStore.save({ speed_limit_enabled: enabling });
  }
</script>

<footer class="statusbar">
  <div class="left">
    <span class="chip">
      <span class="chip-label">{t('stats.speed')}</span>
      <span class="chip-value speed">{formatSpeed(downloadsStore.totalSpeed)}</span>
    </span>
    <span class="chip">
      <span class="chip-label">{t('stats.active')}</span>
      <span class="chip-value">{downloadsStore.activeCount}</span>
    </span>
    <span class="chip">
      <span class="chip-label">{t('header.size')}</span>
      <span class="chip-value">
        {formatSize(downloadsStore.totalDownloaded)} / {formatSize(downloadsStore.totalSize)}
      </span>
    </span>
  </div>

  <div class="right">
    <div class="limiter" class:off={!enabled}>
      <button
        class="snail-btn"
        class:active={enabled}
        onclick={toggleSpeedLimit}
        title={enabled
          ? `${t('speed.snail_on')} (${limit} ${t('speed.mbps')})`
          : t('speed.snail_off')}
        aria-label={t('settings.speed_limit')}
        aria-pressed={enabled}
      >
        <Icon name="snail" size={14} />
      </button>
      <span class="chip-label">{t('settings.speed_limit')}</span>
      <input
        type="range"
        min="0.5"
        max="20"
        step="0.5"
        value={limit}
        oninput={onLimitChange}
        class="slider"
        disabled={!enabled}
        aria-label={t('settings.speed_limit')}
      />
      <span class="limit-value">
        {enabled ? `${limit} ${t('speed.mbps')}` : t('speed.unlimited')}
      </span>
    </div>

    <span class="sep"></span>

    <button
      class="link-btn"
      onclick={() => downloadsStore.clearCompleted()}
      disabled={downloadsStore.completed.length === 0}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <polyline points="3 6 5 6 21 6"/>
        <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>
      </svg>
      <span>{t('statusbar.clear')}</span>
    </button>
  </div>
</footer>

<style>
  .statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    height: 32px;
    padding: 0 12px;
    background: var(--bg-secondary);
    border-top: 1px solid var(--border-color);
    flex-shrink: 0;
    font-size: 11.5px;
  }

  .left {
    display: flex;
    align-items: center;
    gap: 18px;
  }

  .right {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .chip {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .chip-label {
    color: var(--text-muted);
    font-weight: 500;
    white-space: nowrap;
  }

  .chip-value {
    color: var(--text-primary);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .chip-value.speed {
    color: var(--accent);
  }

  .limiter {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .limiter.off {
    opacity: 0.55;
  }

  .snail-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: 1px solid var(--border-light);
    background: var(--bg-elevated);
    color: var(--text-secondary);
    border-radius: var(--radius-sm);
    cursor: pointer;
    box-shadow: inset 0 0 0 1px var(--bg-tertiary);
    transition: background var(--transition-fast), color var(--transition-fast), border-color var(--transition-fast);
  }

  .snail-btn:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
    border-color: var(--accent);
  }

  .snail-btn.active {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-glow);
  }

  .snail-btn.active:hover {
    background: var(--accent-soft);
    color: var(--accent-hover);
  }

  .slider {
    width: 110px;
    height: 4px;
    appearance: none;
    background: var(--bg-tertiary);
    border-radius: 2px;
    outline: none;
    cursor: pointer;
  }

  .slider:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .slider::-webkit-slider-thumb {
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent);
    border: 2px solid var(--bg-secondary);
    transition: transform var(--transition-fast);
  }

  .slider::-webkit-slider-thumb:hover {
    transform: scale(1.15);
  }

  .slider::-moz-range-thumb {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--accent);
    border: 2px solid var(--bg-secondary);
  }

  .limit-value {
    color: var(--text-primary);
    font-weight: 600;
    min-width: 62px;
    font-variant-numeric: tabular-nums;
  }

  .sep {
    width: 1px;
    height: 16px;
    background: var(--border-color);
  }

  .link-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    font-size: 11.5px;
    cursor: pointer;
    padding: 3px 6px;
    border-radius: var(--radius-sm);
    transition: all var(--transition-fast);
  }

  .link-btn:hover:not(:disabled) {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .link-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .link-btn svg {
    width: 13px;
    height: 13px;
  }
</style>
