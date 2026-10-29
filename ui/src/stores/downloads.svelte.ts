import { invoke } from '@tauri-apps/api/core';

export type DownloadStatus =
  | 'pending'
  | 'downloading'
  | 'paused'
  | 'completed'
  | 'error'
  | 'cancelled';

export type DownloadCategory =
  | 'video'
  | 'audio'
  | 'document'
  | 'archive'
  | 'program'
  | 'image'
  | 'other';

export type DownloadKind = 'file' | 'video' | 'torrent';

export interface DownloadItem {
  id: number;
  url: string;
  filename: string;
  total_size: number;
  downloaded: number;
  speed: number;
  status: DownloadStatus;
  error?: string;
  eta: number;
  added_at: number;
  file_path?: string;
  category: DownloadCategory;
  connections: number;
  kind: DownloadKind;
}

export interface MediaInfo {
  duration_secs: number;
  width: number;
  height: number;
  video_codec?: string;
  audio_codec?: string;
  bitrate?: number;
  container?: string;
  frame_rate?: string;
}

export interface PeerInfo {
  address: string;
  state: string;
  client?: string;
  downloaded: number;
  uploaded: number;
}

export interface TorrentInfo {
  info_hash?: string;
  downloaded: number;
  uploaded: number;
  ratio: number;
  peers_live: number;
  peers_seen: number;
  files: TorrentFileInfo[];
}

export interface TorrentFileInfo {
  index: number;
  path: string;
  size: number;
  downloaded: number;
}

export interface TorrentMetaFile {
  index: number;
  path: string;
  size: number;
  downloaded: number;
}

export interface TorrentMeta {
  info_hash: string;
  name: string;
  total_size: number;
  files: TorrentMetaFile[];
}

export interface DownloadDetails {
  id: number;
  kind: DownloadKind;
  url: string;
  filename: string;
  file_path?: string;
  total_size: number;
  downloaded: number;
  uploaded: number;
  speed: number;
  status: DownloadStatus;
  error?: string;
  category: DownloadCategory;
  added_at: number;
  connections: number;
  media?: MediaInfo;
  torrent?: TorrentInfo;
  peers: PeerInfo[];
}

export interface ProgressPayload {
  id: number;
  downloaded: number;
  speed: number;
  eta: number;
}

export interface StatusPayload {
  id: number;
  status: DownloadStatus;
  error?: string;
  total?: number;
}

export interface CompletePayload {
  id: number;
  file_path: string;
  total?: number;
  filename?: string;
  category?: DownloadCategory;
}

export interface ErrorPayload {
  id: number;
  error: string;
}
class DownloadsStore {
  items = $state<DownloadItem[]>([]);
  loading = $state(false);
  error = $state<string | null>(null);

  get active() {
    return this.items.filter(d => d.status === 'downloading');
  }

  get completed() {
    return this.items.filter(d => d.status === 'completed');
  }

  get errored() {
    return this.items.filter(d => d.status === 'error');
  }

  get pending() {
    return this.items.filter(d => d.status === 'pending');
  }

  get paused() {
    return this.items.filter(d => d.status === 'paused');
  }

  get totalSpeed() {
    return this.active.reduce((sum, d) => sum + d.speed, 0);
  }

  get activeCount() {
    return this.active.length;
  }

  get videoCount() {
    return this.items.filter(d => d.kind === 'video' || d.category === 'video').length;
  }

  get torrentCount() {
    return this.items.filter(d => d.kind === 'torrent').length;
  }

  get categoryCounts(): Record<DownloadCategory, number> {
    const counts = {
      video: 0,
      audio: 0,
      document: 0,
      archive: 0,
      program: 0,
      image: 0,
      other: 0,
    } as Record<DownloadCategory, number>;

    for (const item of this.items) {
      counts[item.category] = (counts[item.category] ?? 0) + 1;
    }
    return counts;
  }

  get totalSize() {
    return this.items.reduce((sum, d) => sum + d.total_size, 0);
  }

  get totalDownloaded() {
    return this.items.reduce((sum, d) => sum + d.downloaded, 0);
  }

  async load() {
    this.loading = true;
    this.error = null;
    try {
      this.items = await invoke<DownloadItem[]>('get_downloads');
    } catch (error) {
      this.error = String(error);
      this.items = [];
    } finally {
      this.loading = false;
    }
  }

  async add(url: string, dir?: string) {
    await invoke<number>('add_download', { url, dir });
  }

  async addAs(url: string, filename: string, dir?: string) {
    await invoke<number>('add_download_as', { url, filename, dir });
  }

  async addVideo(url: string, dir?: string, quality?: string) {
    await invoke<number>('add_video', { url, dir, quality });
  }

  async addTorrent(source: string, dir?: string, onlyFiles?: number[] | null) {
    await invoke<number>('add_torrent', { source, dir, onlyFiles });
  }

  addItem(item: DownloadItem) {
    const index = this.items.findIndex(existing => existing.id === item.id);
    if (index >= 0) {
      this.items[index] = { ...this.items[index], ...item };
    } else {
      this.items = [item, ...this.items];
    }
  }

  removeItem(id: number) {
    this.items = this.items.filter(item => item.id !== id);
  }

  updateProgress(payload: ProgressPayload) {
    this.items = this.items.map(item =>
      item.id === payload.id
        ? {
            ...item,
            downloaded: payload.downloaded,
            speed: payload.speed,
            eta: payload.eta,
            status: 'downloading' as const,
          }
        : item
    );
  }

  updateStatus(payload: StatusPayload) {
    this.items = this.items.map(item =>
      item.id === payload.id
        ? {
            ...item,
            status: payload.status,
            error: payload.error,
            total_size: payload.total ?? item.total_size,
          }
        : item
    );
  }

  onComplete(payload: CompletePayload) {
    this.items = this.items.map(item => {
      if (item.id !== payload.id) return item;
      const total = payload.total && payload.total > 0 ? payload.total : item.total_size;
      return {
        ...item,
        status: 'completed' as const,
        file_path: payload.file_path,
        filename: payload.filename ?? item.filename,
        category: payload.category ?? item.category,
        total_size: total,
        downloaded: total,
        speed: 0,
        eta: 0,
      };
    });
  }

  onError(payload: ErrorPayload) {
    this.items = this.items.map(item =>
      item.id === payload.id
        ? { ...item, status: 'error' as const, error: payload.error, speed: 0, eta: 0 }
        : item
    );
  }

  async remove(id: number, deleteFile = false) {
    try {
      await invoke('remove_download', { id, deleteFile });
      this.items = this.items.filter(item => item.id !== id);
    } catch (error) {
      console.error('Failed to remove download:', error);
      throw error;
    }
  }

  async pause(id: number) {
    try {
      await invoke('pause_download', { id });
    } catch (error) {
      console.error('Failed to pause download:', error);
    }
  }

  async resume(id: number) {
    try {
      await invoke('resume_download', { id });
    } catch (error) {
      console.error('Failed to resume download:', error);
    }
  }

  clearError() {
    this.error = null;
  }

  async pauseAll() {
    try {
      await invoke('pause_all');
      this.items = this.items.map(d =>
        d.status === 'downloading' ? { ...d, status: 'paused' as const, speed: 0 } : d,
      );
    } catch (error) {
      console.error('Failed to pause all:', error);
    }
  }

  async resumeAll() {
    try {
      await invoke('resume_all');
      this.items = this.items.map(d =>
        d.status === 'paused' && d.id > 0 ? { ...d, status: 'downloading' as const } : d,
      );
    } catch (error) {
      console.error('Failed to resume all:', error);
    }
  }

  async clearCompleted() {
    try {
      await invoke('clear_completed');
      this.items = this.items.filter(d => d.status !== 'completed');
    } catch (error) {
      console.error('Failed to clear completed:', error);
    }
  }
}

export const downloadsStore = new DownloadsStore();
