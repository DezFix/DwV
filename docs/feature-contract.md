# DwV UI / backend contract (release 0.1.1)

Фиксирует имена команд, события и поля, на которые опирается UI. Менять только вместе с обеими сторонами.

## Идентификаторы

- `1 .. 999_999_999` — обычные HTTP-загрузки.
- `1_000_000_000 + n` — торренты.
- `2_000_000_000 + n` — видео (yt-dlp).
- отрицательные — история из SQLite.

## Типы

```ts
type DownloadKind = 'file' | 'video' | 'torrent';

interface DownloadItem {
  id: number;
  url: string;
  filename: string;
  total_size: number;
  downloaded: number;
  speed: number;
  status: 'pending' | 'downloading' | 'paused' | 'completed' | 'error' | 'cancelled';
  error?: string;
  eta: number;
  added_at: number;
  file_path?: string;
  category: 'video' | 'audio' | 'document' | 'archive' | 'program' | 'other';
  connections: number;
  kind: DownloadKind;
}

interface MediaInfo {
  duration_secs: number;
  width: number;
  height: number;
  video_codec?: string;
  audio_codec?: string;
  bitrate?: number;
  container?: string;
  frame_rate?: string;
}

interface PeerInfo {
  address: string;
  state: string;
  client?: string;
  downloaded: number;
  uploaded: number;
}

interface TorrentInfo {
  info_hash?: string;
  downloaded: number;
  uploaded: number;
  ratio: number;
  peers_live: number;
  peers_seen: number;
}

interface DownloadDetails {
  id: number;
  kind: DownloadKind;
  url: string;
  filename: string;
  file_path?: string;
  total_size: number;
  downloaded: number;
  uploaded: number;
  speed: number;
  status: DownloadItem['status'];
  error?: string;
  category: DownloadItem['category'];
  added_at: number;
  connections: number;
  media?: MediaInfo;
  torrent?: TorrentInfo;
  peers: PeerInfo[];
}
```

## Команды

- `add_download(url) -> number` — HTTP.
- `add_video(url) -> number` — yt-dlp, лучшее качество.
- `add_torrent(source) -> number` — magnet или путь/URL к `.torrent`.
- `get_download_details(id) -> DownloadDetails`.
- `pause_download(id)`, `resume_download(id)`, `remove_download(id, delete_file)` — работают для всех трёх пространств id.
- `pause_all()`, `resume_all()` — включая торренты.
- `get_settings()`, `save_settings(settings)`.

## События

Для всех видов загрузок используются одни события:

- `download-added` — `DownloadItem` сразу после добавления (до первого прогресса).
- `download-progress` — `{ id, downloaded, speed, eta }`.
- `download-status` — `{ id, status, error?, total? }`.
- `download-complete` — `{ id, file_path }`.
- `download-error` — `{ id, error }`.
- `download-removed` — `{ id }`.

События `torrent-progress`, `torrent-complete`, `torrent-error` больше не отправляются.

## Поведение

- `add_torrent` сначала вставляет строку в `downloads`, отправляет `download-added`, затем запускает загрузку. Имя: `dn` из magnet, иначе имя `.torrent`, иначе `torrent`. `total_size` появляется после метаданных.
- `add_video` вставляет строку сразу (`Pending`, имя `video-<id>.mp4`), затем качает через `dwv-media`. По завершении имя и путь заменяются реальными.
- `remove_download` для видео прерывает задачу, для торрента удаляет из сессии (файлы — по флагу).
- `pause_download` для торрента ставит на паузу; для видео пауза не поддерживается.
- При `torrent_autostart = false` строка создаётся в `Paused`, загрузка стартует по `resume_download`.
- `get_download_details` для локального готового файла с медиа-расширением запускает `ffprobe` из `%APPDATA%\DwV\bin\ffprobe.exe`, кэширует по `path + size`. Ошибка probe не роняет команду: `media` отсутствует.
- Список пиров отдаётся только для торрентов, отсортирован по адресу.

## Настройки (новые поля)

```ts
speed_limit_enabled: boolean;        // default true
torrent_upload_limit_kbps: number;   // default 0 = без лимита
torrent_seed_after_complete: boolean;// default false
torrent_max_peers: number;           // default 50
torrent_dht_enabled: boolean;        // default true
```

Дефолты в Rust и TS совпадают. Тема по умолчанию `system`, лимит скорости `2.0`.

Лимит применяется, только если `speed_limit_enabled`. Значение меньше `0.5` трактуется как `0.5`. При выключенном лимите HTTP-очередь и торрент-сессия получают `None`.

## Deep link

`dwv://add?url=<url>&type=<file|video|torrent>`

- без `type` тип определяется по URL (`magnet:`/`.torrent` → torrent, иначе file);
- ссылка без параметров просто открывает приложение;
- второй экземпляр приложения пересылает запрос первому через IPC.

## Окно

- Кнопка «свернуть» вызывает `hide()`, если `minimize_to_tray`, иначе `minimize()`.
- Системное сворачивание окна при `minimize_to_tray` прячет окно в трей.
- Крестик: `close_to_tray` → скрыть, иначе закрыть.
- Трей: «Показать» показывает и фокусирует окно; «Настройки» показывает окно и отправляет событие `open-settings`; «Пауза»/«Продолжить» вызывают `pause_all`/`resume_all`; «Выход» завершает процесс.

## Уточнения релиза 0.1.0

### `add_torrent`

- Строка `DownloadInfo` вставляется в `state.downloads` и событие `download-added` уходит до первого прогресса.
- `id = 1_000_000_000 + n`, `kind = "torrent"`, имя: `dn` из magnet, иначе имя `.torrent`, иначе `torrent`.
- `torrent_autostart = false` → строка в `paused`, загрузка по `resume_download`.
- Прогресс/статус/завершение/ошибка идут через `download-progress`, `download-status`, `download-complete`, `download-error`.
- Папка: `download_dir`; отдельной папки торрентов больше нет.
- `pause_download`, `resume_download`, `remove_download` управляют торрент-сессией. `remove` с `delete_file` удаляет файлы торрента.
- Лимит скачивания общий с HTTP; лимит отдачи — `torrent_upload_limit_kbps` (0 = без лимита).
- `torrent_seed_after_complete = false` → после завершения торрент останавливается (раздача не продолжается).

### `add_video`

- `id = 2_000_000_000 + n`, `kind = "video"`, строка вставляется сразу, `filename = video-<id>.mp4`.
- Загрузка через `dwv_media::download_video`, после завершения имя и путь заменяются реальными.
- Пауза для видео не поддерживается: `pause_download` возвращает `Ok(())`, но статус не меняется; UI не показывает паузу для `kind = "video"`.
- `remove_download` прерывает задачу.
- События те же `download-*`.

### `get_download_details(id)`

- Поддерживает живые id и отрицательные id истории.
- `kind` определяется по id/строке: `file`, `video`, `torrent`.
- Для готового локального файла с медиа-расширением заполняется `media` через `ffprobe.exe` из `%APPDATA%\DwV\bin`; если `ffprobe` ещё не скачан, `media` отсутствует (фоновая загрузка не блокирует команду).
- Для торрента заполняются `torrent` (downloaded, uploaded, ratio, peers_live, peers_seen) и `peers` (адрес, состояние, клиент, отдано/принято). Список отсортирован по адресу.
- Для остальных `peers = []`.

### IPC расширения

Добавляется вариант `{"type":"video","url":"..."}` в `dwv-ipc::Request`. Обработчик:
- `add` / `download` → `add_download`;
- `torrent` → `add_torrent`;
- `video` → `add_video`.
После принятия запроса отправляется `download-added`, если строка ещё не отправлена.

### Скорость

- `speed_limit_enabled = false` → HTTP-очередь и торрент-сессия получают `None` (без лимита).
- `speed_limit_enabled = true` → лимит `max(speed_limit_mbps, 0.5) * 1_000_000` байт/с, по умолчанию `2.0`.
- Snail в UI просто переключает `speed_limit_enabled`; значение лимита берётся из настроек.

### UI-контракт деталей

- `kind = "torrent"` → вкладки «Файл» и «Пиры».
- `kind = "video"` → вкладки «Файл» и «Видео».
- `kind = "file"` → вкладка «Файл».
- Если `media` нет, вкладка «Видео» показывает подсказку, что метаданные ещё не готовы.
## Изменения 0.1.1

- Все загрузки (HTTP, видео, торренты) сохраняются прямо в `download_dir`; подпапки категорий и `torrent_dir` не используются.
- `remove_download` для отрицательных id истории удаляет файл/папку при `delete_file = true`.
- Добавлена команда `open_url(url)` для http/https; `open_file` возвращает ошибку, если путь не существует.
- IPC: команда расширения `{"type":"video","url":"..."}`; ping показывает скрытое окно.
- yt-dlp: путь берётся через `--print-to-file after_move` с файловым fallback; консольные окна скрыты.
- UI: чекбоксы мультивыбора, диалог удаления с запоминанием, улитка в статус-баре, Ko-fi, `clipboard-manager:allow-read-text`.
