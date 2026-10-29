import { invoke } from '@tauri-apps/api/core';
import type { LocaleSetting } from '../i18n/index.svelte.ts';

export interface CategorySettings {
  category: string;
  enabled: boolean;
  path: string;
  auto_extract: boolean;
}

export interface Settings {
  download_dir: string;
  max_concurrent_downloads: number;
  speed_limit_mbps: number;
  speed_limit_enabled: boolean;
  auto_start_downloads: boolean;
  connections_per_download: number;
  ask_save_path: boolean;
  autostart: boolean;
  minimize_to_tray: boolean;
  close_to_tray: boolean;
  show_notifications: boolean;
  theme: 'system' | 'light' | 'dark';
  proxy_enabled: boolean;
  proxy_host: string;
  proxy_port: number;
  proxy_username: string;
  proxy_password: string;
  categories: CategorySettings[];
  custom_user_agent: string;
  ui_scale: number;
  animations_enabled: boolean;
  truncate_filenames: boolean;
  language: LocaleSetting;
  torrent_dir: string;
  torrent_autostart: boolean;
  torrent_upload_limit_kbps: number;
  torrent_max_peers: number;
  torrent_seed_after_complete: boolean;
  torrent_dht_enabled: boolean;
}

const defaultCategories = (): CategorySettings[] => [
  { category: 'video', enabled: true, path: 'Videos', auto_extract: false },
  { category: 'audio', enabled: true, path: 'Music', auto_extract: false },
  { category: 'document', enabled: true, path: 'Documents', auto_extract: false },
  { category: 'archive', enabled: true, path: 'Archives', auto_extract: true },
  { category: 'program', enabled: true, path: 'Programs', auto_extract: false },
  { category: 'image', enabled: true, path: 'Images', auto_extract: false },
  { category: 'other', enabled: true, path: 'Other', auto_extract: false },
];

class SettingsStore {
  download_dir = $state('');
  max_concurrent_downloads = $state(3);
  speed_limit_mbps = $state(2);
  speed_limit_enabled = $state(true);
  auto_start_downloads = $state(true);
  connections_per_download = $state(8);
  ask_save_path = $state(true);
  autostart = $state(true);
  minimize_to_tray = $state(true);
  close_to_tray = $state(true);
  show_notifications = $state(true);
  theme = $state<'system' | 'light' | 'dark'>('system');
  proxy_enabled = $state(false);
  proxy_host = $state('');
  proxy_port = $state(0);
  proxy_username = $state('');
  proxy_password = $state('');
  categories = $state<CategorySettings[]>(defaultCategories());
  custom_user_agent = $state('');
  ui_scale = $state(1);
  animations_enabled = $state(true);
  truncate_filenames = $state(false);
  language = $state<LocaleSetting>('auto');
  torrent_dir = $state('');
  torrent_autostart = $state(true);
  torrent_upload_limit_kbps = $state(0);
  torrent_max_peers = $state(50);
  torrent_seed_after_complete = $state(false);
  torrent_dht_enabled = $state(true);

  get snapshot(): Settings {
    return {
      download_dir: this.download_dir,
      max_concurrent_downloads: this.max_concurrent_downloads,
      speed_limit_mbps: this.speed_limit_mbps,
      speed_limit_enabled: this.speed_limit_enabled,
      auto_start_downloads: this.auto_start_downloads,
      connections_per_download: this.connections_per_download,
      ask_save_path: this.ask_save_path,
      autostart: this.autostart,
      minimize_to_tray: this.minimize_to_tray,
      close_to_tray: this.close_to_tray,
      show_notifications: this.show_notifications,
      theme: this.theme,
      proxy_enabled: this.proxy_enabled,
      proxy_host: this.proxy_host,
      proxy_port: this.proxy_port,
      proxy_username: this.proxy_username,
      proxy_password: this.proxy_password,
      categories: this.categories.map(c => ({ ...c })),
      custom_user_agent: this.custom_user_agent,
      ui_scale: this.ui_scale,
      animations_enabled: this.animations_enabled,
      truncate_filenames: this.truncate_filenames,
      language: this.language,
      torrent_dir: this.torrent_dir,
      torrent_autostart: this.torrent_autostart,
      torrent_upload_limit_kbps: this.torrent_upload_limit_kbps,
      torrent_max_peers: this.torrent_max_peers,
      torrent_seed_after_complete: this.torrent_seed_after_complete,
      torrent_dht_enabled: this.torrent_dht_enabled,
    };
  }

  async load() {
    try {
      const s = await invoke<Settings>('get_settings');
      Object.assign(this, s);
      if (this.speed_limit_mbps < 0.5) {
        this.speed_limit_mbps = 2;
      }
    } catch (error) {
      console.error('Failed to load settings:', error);
    }
  }

  async save(patch: Partial<Settings> = {}) {
    if (patch.speed_limit_mbps !== undefined) {
      const raw = Number(patch.speed_limit_mbps);
      patch = {
        ...patch,
        speed_limit_mbps: Math.max(0.5, Number.isFinite(raw) ? raw : 2),
      };
    }
    Object.assign(this, patch);
    try {
      await invoke('save_settings', { settings: this.snapshot });
    } catch (error) {
      console.error('Failed to save settings:', error);
    }
  }

  async updateCategory(category: string, patch: Partial<CategorySettings>) {
    this.categories = this.categories.map(c =>
      c.category === category ? { ...c, ...patch } : c
    );
    await this.save();
  }

  reset() {
    Object.assign(this, {
      download_dir: '',
      max_concurrent_downloads: 3,
      speed_limit_mbps: 2,
      speed_limit_enabled: true,
      auto_start_downloads: true,
      connections_per_download: 8,
      ask_save_path: true,
      autostart: true,
      minimize_to_tray: true,
      close_to_tray: true,
      show_notifications: true,
      theme: 'system' as const,
      proxy_enabled: false,
      proxy_host: '',
      proxy_port: 0,
      proxy_username: '',
      proxy_password: '',
      categories: defaultCategories(),
      custom_user_agent: '',
      ui_scale: 1,
      animations_enabled: true,
      truncate_filenames: false,
      language: 'auto' as const,
      torrent_dir: '',
      torrent_autostart: true,
      torrent_upload_limit_kbps: 0,
      torrent_max_peers: 50,
      torrent_seed_after_complete: false,
      torrent_dht_enabled: true,
    });
  }
}

export const settingsStore = new SettingsStore();
