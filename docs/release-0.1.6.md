# DwV 0.1.6

Исправлено скачивание с pixeldrain и похожих файлообменников, которые отвечали `403` на HTTP/2-запросы. Загрузчик переведён на HTTP/1.1 и системный TLS Windows (Schannel), HTTP-загрузки идут в одну сессию.

Проверено на `https://pixeldrain.com/api/file/19NQShy1?download`: файл 954 894 679 байт скачан полностью.

Остальное как в 0.1.5: автозапуск, `dwv://` и `magnet:`, расширение 1.2.5, yt-dlp, обновление из GitHub.

## Файлы

- `DwV_0.1.6_x64-setup.exe` — установщик.
- `DwV-0.1.6-portable.exe` — портативный exe.
- `DwV_0.1.6_x64_en-US.msi` — MSI.
- `dwv-firefox-1.2.5.xpi` — расширение для Firefox/LibreWolf.
- `latest.json` — манифест обновления.

## SHA-256

```
1eef262dc59600efe211b3ca9ff43cfab00b795520e2315c06e573a3c2b9025c  DwV_0.1.6_x64-setup.exe
57cfeddbfca9ada66ed35f7335dbeca40d7342cb85814dc7d1c432bbcf295622  DwV-0.1.6-portable.exe
745e2a61941bd366901753284d66711f50d383d5cecf9d2e49cd8d90d071a328  DwV_0.1.6_x64_en-US.msi
5ba5676b65d5d2fdf10b662ef7eef4eb5245975e75109de94ffd4813252fea73  dwv-firefox-1.2.5.xpi
```
