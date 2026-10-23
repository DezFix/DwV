<script lang="ts">
  import { settingsStore } from '../stores/settings.svelte.ts';
  import { deletePrefs, type DeleteMode } from '../stores/deletePrefs.svelte.ts';
  import { setLocale, t, type LocaleSetting } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';
  import type { IconName } from '../assets/icons';

  interface Props {
    query: string;
    onClose: () => void;
  }

  let { query, onClose }: Props = $props();

  interface Section {
    id: string;
    label: string;
    icon: IconName;
  }

  const sections: Section[] = [
    { id: 'general', label: 'settings.tabs.general', icon: 'home' },
    { id: 'downloads', label: 'settings.tabs.downloads', icon: 'download' },
    { id: 'torrents', label: 'settings.tabs.torrents', icon: 'archive' },
    { id: 'network', label: 'settings.tabs.network', icon: 'globe' },
    { id: 'appearance', label: 'settings.tabs.appearance', icon: 'palette' },
    { id: 'advanced', label: 'settings.tabs.advanced', icon: 'sliders' },
  ];

  let active = $state('general');
  let contentEl = $state<HTMLElement | null>(null);
  let matchedSections = $state<Set<string>>(new Set());
  let hasQueryMatch = $state(true);
  let kofiError = $state<string | null>(null);

  async function openKofi() {
    kofiError = null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_url', { url: 'https://ko-fi.com/dezfix' });
    } catch (error) {
      kofiError = `${t('kofi.error')}: ${String(error)}`;
    }
  }

  const needle = $derived(query.trim().toLowerCase());

  const visibleSections = $derived(
    sections.filter(section => !needle || matchedSections.has(section.id)),
  );

  function rowText(row: HTMLElement): string {
    const parts = [row.textContent ?? ''];
    row.querySelectorAll<HTMLInputElement>('input').forEach(input => {
      parts.push(input.type === 'checkbox' ? (input.checked ? 'on' : 'off') : input.value);
    });
    row.querySelectorAll<HTMLSelectElement>('select').forEach(select => {
      parts.push(select.value, select.selectedOptions[0]?.textContent ?? '');
    });
    return parts.join(' ').toLowerCase();
  }

  $effect(() => {
    const q = needle;
    void settingsStore.snapshot;

    const el = contentEl;
    if (!el) return;

    const matches = new Set<string>();
    for (const block of Array.from(el.querySelectorAll<HTMLElement>('[data-section]'))) {
      const section = sections.find(item => item.id === block.dataset.section);
      const heading = `${block.querySelector('h2')?.textContent ?? ''} ${section ? t(section.label) : ''}`;
      let visibleRows = 0;
      for (const row of Array.from(block.querySelectorAll<HTMLElement>('.row'))) {
        const match = !q || `${heading} ${rowText(row)}`.toLowerCase().includes(q);
        row.hidden = !match;
        if (match) visibleRows++;
      }
      const show = visibleRows > 0;
      block.hidden = !show;
      if (show && block.dataset.section) matches.add(block.dataset.section);
    }

    matchedSections = matches;
    hasQueryMatch = !q || matches.size > 0;
  });

  function scrollTo(id: string) {
    active = id;
    const el = contentEl?.querySelector<HTMLElement>(`[data-section="${id}"]`);
    el?.scrollIntoView({ behavior: settingsStore.animations_enabled ? 'smooth' : 'auto', block: 'start' });
  }

  function onScroll() {
    if (!contentEl) return;
    let current = sections[0].id;
    for (const section of sections) {
      const el = contentEl.querySelector<HTMLElement>(`[data-section="${section.id}"]`);
      if (!el || el.hidden) continue;
      if (el.getBoundingClientRect().top - contentEl.getBoundingClientRect().top <= 80) {
        current = section.id;
      }
    }
    active = current;
  }
</script>

<div class="panel">
  <aside class="panel-nav">
    <div class="nav-head">
      <span class="nav-title">{t('settings.title')}</span>
      <button class="close-btn" onclick={onClose} title={t('actions.close')} aria-label={t('actions.close')}>
        <Icon name="close" size={15} />
      </button>
    </div>

    <nav class="nav-list">
      {#each visibleSections as section (section.id)}
        <button class="nav-item" class:active={active === section.id} onclick={() => scrollTo(section.id)}>
          <span class="nav-icon"><Icon name={section.icon} size={15} /></span>
          <span>{t(section.label)}</span>
        </button>
      {/each}
    </nav>
  </aside>

  <div class="panel-content" bind:this={contentEl} onscroll={onScroll}>
    <div class="sheet">
      <!-- ОБЩИЕ -->
      <section class="block" data-section="general">
        <h2>{t('settings.download_folder')}</h2>
        <div class="row">
          <span class="label">{t('settings.download_folder')}</span>
          <div class="inline">
            <input
              type="text"
              value={settingsStore.download_dir}
              readonly
              class="input grow"
              placeholder="—"
            />
            <button class="btn" onclick={async () => {
              const { open } = await import('@tauri-apps/plugin-dialog');
              const path = await open({ directory: true, title: t('settings.download_folder') });
              if (typeof path === 'string') await settingsStore.save({ download_dir: path });
            }}>
              <Icon name="folder" size={15} />
              <span>{t('settings.browse')}</span>
            </button>
          </div>
          <p class="hint">{t('settings.download_folder_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.language')}</span>
          <select
            class="input select"
            value={settingsStore.language}
            onchange={(e) => {
              const next = e.currentTarget.value as LocaleSetting;
              setLocale(next);
              settingsStore.save({ language: next });
            }}
          >
            <option value="auto">{t('settings.language_system')}</option>
            <option value="ru">Русский</option>
            <option value="en">English</option>
          </select>
          <p class="hint">{t('settings.language_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.minimize_tray')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.minimize_to_tray}
            onchange={(e) => settingsStore.save({ minimize_to_tray: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.minimize_tray_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.close_tray')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.close_to_tray}
            onchange={(e) => settingsStore.save({ close_to_tray: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.close_tray_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.notifications')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.show_notifications}
            onchange={(e) => settingsStore.save({ show_notifications: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.notifications_hint')}</p>
        </div>
      </section>

      <!-- ЗАГРУЗКИ -->
      <section class="block" data-section="downloads">
        <h2>{t('settings.tabs.downloads')}</h2>
        <div class="row">
          <span class="label">{t('settings.max_concurrent')}</span>
          <input
            type="number"
            min="1"
            max="16"
            class="input number"
            value={settingsStore.max_concurrent_downloads}
            onchange={(e) => settingsStore.save({ max_concurrent_downloads: Number(e.currentTarget.value) })}
          />
          <p class="hint">{t('settings.max_concurrent_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.speed_limit')}</span>
          <input
            type="number"
            min="0.5"
            step="0.5"
            class="input number"
            value={settingsStore.speed_limit_mbps}
            onchange={(e) => {
              const raw = Number(e.currentTarget.value);
              settingsStore.save({ speed_limit_mbps: Math.max(0.5, Number.isFinite(raw) ? raw : 2) });
            }}
          />
          <p class="hint">{t('settings.speed_limit_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.speed_limit_enabled')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.speed_limit_enabled}
            onchange={(e) => settingsStore.save({ speed_limit_enabled: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.speed_limit_enabled_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.auto_start')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.auto_start_downloads}
            onchange={(e) => settingsStore.save({ auto_start_downloads: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.auto_start_hint')}</p>
        </div>
      </section>

      <!-- ТОРРЕНТЫ -->
      <section class="block" data-section="torrents">
        <h2>{t('settings.tabs.torrents')}</h2>
        <div class="row">
          <span class="label">{t('settings.torrent_autostart')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.torrent_autostart}
            onchange={(e) => settingsStore.save({ torrent_autostart: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.torrent_autostart_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.torrent_upload_limit')}</span>
          <input
            type="number"
            min="0"
            step="10"
            class="input number"
            value={settingsStore.torrent_upload_limit_kbps}
            onchange={(e) => {
              const raw = Number(e.currentTarget.value);
              settingsStore.save({ torrent_upload_limit_kbps: Math.max(0, Number.isFinite(raw) ? raw : 0) });
            }}
          />
          <p class="hint">{t('settings.torrent_upload_limit_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.torrent_max_peers')}</span>
          <input
            type="number"
            min="1"
            max="1000"
            class="input number"
            value={settingsStore.torrent_max_peers}
            onchange={(e) => {
              const raw = Number(e.currentTarget.value);
              settingsStore.save({ torrent_max_peers: Math.min(1000, Math.max(1, Number.isFinite(raw) ? raw : 50)) });
            }}
          />
          <p class="hint">{t('settings.torrent_max_peers_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.torrent_dht')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.torrent_dht_enabled}
            onchange={(e) => settingsStore.save({ torrent_dht_enabled: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.torrent_dht_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.torrent_seed')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.torrent_seed_after_complete}
            onchange={(e) => settingsStore.save({ torrent_seed_after_complete: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.torrent_seed_hint')}</p>
        </div>

        <p class="note">{t('settings.torrent_note')}</p>
      </section>

      <!-- СЕТЬ -->
      <section class="block" data-section="network">
        <h2>{t('settings.tabs.network')}</h2>
        <div class="row">
          <span class="label">{t('settings.proxy_enable')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.proxy_enabled}
            onchange={(e) => settingsStore.save({ proxy_enabled: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.proxy_enable_hint')}</p>
        </div>

        {#if settingsStore.proxy_enabled}
          <div class="row">
            <span class="label">{t('settings.proxy_host')}</span>
            <input
              type="text"
              class="input grow"
              value={settingsStore.proxy_host}
              onchange={(e) => settingsStore.save({ proxy_host: e.currentTarget.value })}
              placeholder="proxy.example.com"
            />
          </div>
          <div class="row">
            <span class="label">{t('settings.proxy_port')}</span>
            <input
              type="number"
              min="1"
              max="65535"
              class="input number"
              value={settingsStore.proxy_port}
              onchange={(e) => settingsStore.save({ proxy_port: Number(e.currentTarget.value) })}
            />
          </div>
          <div class="row">
            <span class="label">{t('settings.proxy_user')}</span>
            <input
              type="text"
              class="input grow"
              value={settingsStore.proxy_username}
              onchange={(e) => settingsStore.save({ proxy_username: e.currentTarget.value })}
            />
          </div>
          <div class="row">
            <span class="label">{t('settings.proxy_password')}</span>
            <input
              type="password"
              class="input grow"
              value={settingsStore.proxy_password}
              onchange={(e) => settingsStore.save({ proxy_password: e.currentTarget.value })}
            />
          </div>
        {/if}

        <div class="row">
          <span class="label">{t('settings.user_agent')}</span>
          <input
            type="text"
            class="input grow"
            value={settingsStore.custom_user_agent}
            onchange={(e) => settingsStore.save({ custom_user_agent: e.currentTarget.value })}
            placeholder={t('settings.user_agent_hint')}
          />
        </div>
      </section>

      <!-- ВНЕШНИЙ ВИД -->
      <section class="block" data-section="appearance">
        <h2>{t('settings.tabs.appearance')}</h2>
        <div class="row">
          <span class="label">{t('settings.theme')}</span>
          <select
            class="input select"
            value={settingsStore.theme}
            onchange={(e) => settingsStore.save({ theme: e.currentTarget.value as 'system' | 'light' | 'dark' })}
          >
            <option value="system">{t('theme.system')}</option>
            <option value="light">{t('theme.light')}</option>
            <option value="dark">{t('theme.dark')}</option>
          </select>
          <p class="hint">{t('settings.theme_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.scale')}</span>
          <select
            class="input select"
            value={String(settingsStore.ui_scale)}
            onchange={(e) => settingsStore.save({ ui_scale: Number(e.currentTarget.value) })}
          >
            <option value="0.8" selected={settingsStore.ui_scale === 0.8}>80%</option>
            <option value="0.9" selected={settingsStore.ui_scale === 0.9}>90%</option>
            <option value="1" selected={settingsStore.ui_scale === 1}>100%</option>
            <option value="1.1" selected={settingsStore.ui_scale === 1.1}>110%</option>
            <option value="1.2" selected={settingsStore.ui_scale === 1.2}>120%</option>
            <option value="1.5" selected={settingsStore.ui_scale === 1.5}>150%</option>
          </select>
        </div>

        <div class="row">
          <span class="label">{t('settings.animations')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.animations_enabled}
            onchange={(e) => settingsStore.save({ animations_enabled: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.animations_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.truncate')}</span>
          <input
            type="checkbox"
            class="switch"
            checked={settingsStore.truncate_filenames}
            onchange={(e) => settingsStore.save({ truncate_filenames: e.currentTarget.checked })}
          />
          <p class="hint">{t('settings.truncate_hint')}</p>
        </div>
      </section>

      <!-- ДОПОЛНИТЕЛЬНО -->
      <section class="block" data-section="advanced">
        <h2>{t('settings.tabs.advanced')}</h2>
        <div class="row">
          <span class="label">{t('settings.delete_mode')}</span>
          <select
            class="input select"
            value={deletePrefs.mode}
            onchange={(e) => deletePrefs.setMode(e.currentTarget.value as DeleteMode)}
          >
            <option value="ask">{t('settings.delete_mode_ask')}</option>
            <option value="keep">{t('settings.delete_mode_keep')}</option>
            <option value="delete">{t('settings.delete_mode_delete')}</option>
          </select>
          <p class="hint">{t('settings.delete_mode_hint')}</p>
        </div>

        <div class="row">
          <span class="label">{t('settings.data')}</span>
          <div class="inline">
            <button class="btn" onclick={() => {
              const blob = new Blob([JSON.stringify(settingsStore.snapshot, null, 2)], { type: 'application/json' });
              const url = URL.createObjectURL(blob);
              const a = document.createElement('a');
              a.href = url;
              a.download = 'dwv-settings.json';
              a.click();
              URL.revokeObjectURL(url);
            }}>
              <Icon name="external" size={15} />
              <span>{t('settings.export')}</span>
            </button>
            <label class="btn file-btn">
              <Icon name="download" size={15} />
              <span>{t('settings.import')}</span>
              <input
                type="file"
                accept=".json"
                onchange={async (e) => {
                  const input = e.currentTarget;
                  const file = input.files?.[0];
                  if (!file) return;
                  try {
                    const parsed = JSON.parse(await file.text());
                    await settingsStore.save(parsed);
                  } catch (err) {
                    alert(String(err));
                  }
                  input.value = '';
                }}
              />
            </label>
          </div>
        </div>

        <div class="row">
          <span class="label">{t('settings.maintenance')}</span>
          <div class="inline">
            <button class="btn" onclick={async () => {
              const { invoke } = await import('@tauri-apps/api/core');
              await invoke('clear_cache');
            }}>
              <span>{t('settings.clear_cache')}</span>
            </button>
            <button class="btn danger" onclick={async () => {
              if (!confirm(t('settings.reset_db'))) return;
              const { invoke } = await import('@tauri-apps/api/core');
              await invoke('reset_database');
            }}>
              <span>{t('settings.reset_db')}</span>
            </button>
          </div>
        </div>

        <div class="row">
          <span class="label">{t('settings.updates')}</span>
          <div class="inline">
            <button class="btn" onclick={() => window.dispatchEvent(new CustomEvent('dwv-check-updates'))}>
              <span>{t('settings.check_updates')}</span>
            </button>
          </div>
        </div>

        <div class="row">
          <span class="label">{t('settings.about')}</span>
          <div class="about">
            <p><strong>DwV Download Manager</strong></p>
            <p>0.1.4 · Rust + Tauri + Svelte 5</p>
            <p>Copyright © DwV contributors · GPL-3.0-or-later</p>
          </div>
        </div>

        <div class="row">
          <span class="label">Ko-fi</span>
          <div class="inline">
            <button class="btn kofi-btn" onclick={openKofi}>
              <Icon name="heart" size={15} />
              <span>{t('kofi.support')}</span>
            </button>
          </div>
          {#if kofiError}
            <p class="hint error">{kofiError}</p>
          {/if}
        </div>
      </section>

      {#if !hasQueryMatch}
        <p class="no-results">{t('settings.search_empty')}</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .panel {
    display: flex;
    height: 100%;
    min-height: 0;
    background: var(--bg-elevated);
    animation: fade-in 0.18s ease;
  }

  .panel-nav {
    width: 230px;
    flex-shrink: 0;
    background: var(--bg-secondary);
    border-right: 1px solid var(--border-color);
    display: flex;
    flex-direction: column;
  }

  .nav-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 52px;
    padding: 0 8px 0 18px;
    border-bottom: 1px solid var(--border-color);
  }

  .nav-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .close-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .close-btn:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .nav-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 8px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    text-align: left;
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
    width: 15px;
    height: 15px;
  }

  .panel-content {
    flex: 1;
    overflow-y: auto;
    scroll-behavior: smooth;
  }

  .sheet {
    max-width: 620px;
    margin: 0 auto;
    padding: 32px 32px 80px;
    animation: rise-in 0.22s ease;
  }

  .block {
    margin-bottom: 40px;
    scroll-margin-top: 24px;
  }

  .block h2 {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0 0 6px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--border-color);
  }

  .row {
    display: grid;
    grid-template-columns: 190px 1fr;
    align-items: center;
    gap: 6px 16px;
    padding: 11px 0;
  }

  .row > .label {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .row .hint {
    grid-column: 2;
    font-size: 11px;
    color: var(--text-muted);
    margin: 0;
  }

  .row .hint.error {
    color: var(--error);
  }

  .inline {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .input {
    padding: 7px 10px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
    transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
  }

  .input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }

  .input.grow {
    width: 100%;
  }

  .input.number {
    width: 110px;
  }

  .input.select {
    width: 190px;
    cursor: pointer;
  }

  .switch {
    width: 38px;
    height: 21px;
    appearance: none;
    background: var(--border-color);
    border-radius: 11px;
    position: relative;
    cursor: pointer;
    transition: background var(--transition-fast);
  }

  .switch::after {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: #fff;
    transition: transform var(--transition-fast);
  }

  .switch:checked {
    background: var(--accent);
  }

  .switch:checked::after {
    transform: translateX(17px);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 7px 13px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: all var(--transition-fast);
  }

  .btn:hover {
    background: var(--bg-tertiary);
  }

  .btn.danger {
    color: var(--error);
    border-color: color-mix(in srgb, var(--error) 35%, transparent);
  }

  .btn.danger:hover {
    background: color-mix(in srgb, var(--error) 12%, transparent);
  }

  .file-btn {
    position: relative;
    overflow: hidden;
  }

  .file-btn input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }

  .about {
    font-size: 12px;
    line-height: 1.6;
    color: var(--text-secondary);
  }

  .about p {
    margin: 0 0 2px;
  }

  .no-results {
    padding: 48px 0;
    text-align: center;
    font-size: 13px;
    color: var(--text-muted);
  }

  .note {
    margin-top: 12px;
    padding: 10px 12px;
    background: var(--bg-tertiary);
    border-radius: var(--radius-sm);
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-muted);
  }
</style>
