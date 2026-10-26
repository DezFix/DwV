import type { DownloadCategory, DownloadItem } from './downloads.svelte.ts';
import type { View } from '../components/Sidebar.svelte';

export type SortBy = 'added' | 'name' | 'progress' | 'speed' | 'size';

export function visibleDownloads(
  items: DownloadItem[],
  view: View,
  category: DownloadCategory | null,
  query: string,
  sortBy: SortBy,
  sortDir: 'asc' | 'desc',
): DownloadItem[] {
  const needle = query.trim().toLowerCase();
  const filtered = items.filter(item => {
    if (category && item.category !== category) return false;
    if (view === 'torrent' && item.kind !== 'torrent') return false;
    if (view === 'video' && item.kind !== 'video' && item.category !== 'video') return false;
    if (view !== 'all' && view !== 'torrent' && view !== 'video' && item.status !== view) return false;
    if (!needle) return true;
    return item.filename.toLowerCase().includes(needle) || item.url.toLowerCase().includes(needle);
  });

  const value = (item: DownloadItem): string | number => {
    switch (sortBy) {
      case 'name': return item.filename.toLowerCase();
      case 'progress': return item.total_size > 0 ? item.downloaded / item.total_size : 0;
      case 'speed': return item.speed;
      case 'size': return item.total_size;
      case 'added': return item.added_at;
    }
  };

  return filtered.sort((a, b) => {
    const av = value(a);
    const bv = value(b);
    if (av < bv) return sortDir === 'asc' ? -1 : 1;
    if (av > bv) return sortDir === 'asc' ? 1 : -1;
    return 0;
  });
}
