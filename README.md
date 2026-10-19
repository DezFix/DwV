# DwV

Менеджер загрузок на Rust с перехватом файлов из браузера.

- Сегментное скачивание с докачкой (HTTP/HTTPS)
- Расширения для Chrome и Firefox: перехват загрузок, контекстное меню, галерея медиа со страницы
- Видео с любых сайтов через yt-dlp (автообновление при запуске)
- Торренты через librqbit
- Интерфейс на Slint, трей, инсталлятор для Windows

Статус: ранняя разработка (Фаза 0 — каркас).

## Сборка

```powershell
cargo build --release
```

Бинарник приложения: `target/release/dwv.exe`

## CLI (текущий движок скачивания)

```powershell
cargo run -p dwv-cli -- https://example.com/file.zip -o file.zip -n 8
cargo run -p dwv-cli -- <url1> <url2> <url3> -j 3
```

`-n` — соединений на файл, `-j` — параллельных загрузок в очереди. Ctrl+C останавливает; `.part` и `.dwvmeta` сохраняются, повторный запуск продолжит с места обрыва.

## Структура

```
crates/
  dwv-core/     движок скачивания (HTTP-сегменты, очередь, докачка)
  dwv-media/    yt-dlp / ffmpeg
  dwv-torrent/  librqbit
  dwv-ipc/      native messaging для расширений + локальный WS API
  dwv-app/      GUI (Slint)
  dwv-cli/      headless-версия для тестов и скриптов
ui/             Slint-файлы
extension/      расширения Chrome/Firefox
```

## Лицензия

GPL-3.0-or-later, см. [LICENSE](LICENSE)

## Донаты

https://ko-fi.com/dezfix
