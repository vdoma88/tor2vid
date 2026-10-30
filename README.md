# tor2vid

Онлайн-плеер для торрентов. Вставьте magnet-ссылку (или info-hash, или ссылку
на `.torrent`) либо перетащите `.torrent` файл — видео начнёт играть через
несколько секунд, без ожидания полной загрузки.

## Как устроено

Браузерный WebTorrent видит только WebRTC-пиров, а почти все обычные раздачи
живут на TCP/UDP. Поэтому к рою подключается сервер (Node + [webtorrent]):

1. `POST /api/torrents` — добавляет торрент, ждёт список файлов (до 60 с).
2. `GET /stream/:hash/:index` — отдаёт файл с поддержкой `Range`; при перемотке
   сервер сразу просит у пиров нужные куски.
3. `GET /subs/:hash/:index` — `.srt` из торрента как WebVTT для плеера.
4. `GET /api/probe/:hash/:index` и `GET /transcode/:hash/:index?t=&a=` —
   ffmpeg превращает MKV, AVI, HEVC, AC3/DTS во фрагментированный MP4 прямо
   на лету. H.264 только перепаковывается (почти не нагружает процессор),
   остальное перекодируется в H.264 + AAC. `t` — с какой секунды, `a` —
   звуковая дорожка (русская выбирается сама, если есть).

Качается только то, что смотрят. Торренты, которые не открывали 20 минут,
удаляются вместе с файлами; одновременно держится не больше `MAX_TORRENTS`.

MP4 и WebM играют напрямую. Для остального плеер переключается на своё
управление: перемотка перезапускает ffmpeg с нужной секунды. Встроенные в MKV
субтитры пока не показываются (внешние `.srt` — да).

## Приложение для Windows

Архив со всем нужным (Node.js и ffmpeg внутри) собирает GitHub Actions
(`.github/workflows/release.yml`) при каждом изменении кода в основной ветке
и кладёт в релиз `v<version из package.json>`. Новая версия — поднять
`version` в package.json.

Ссылка на последнюю сборку:
https://github.com/vdoma88/tor2vid/releases/latest/download/tor2vid-windows-x64.zip
— распаковать и запустить `tor2vid.bat`, браузер откроется сам.

## Запуск из исходников

```bash
npm install        # ffmpeg ставится пакетом ffmpeg-static
npm start          # http://localhost:3000
```

Переменные: `PORT`, `MAX_TORRENTS` (3), `MAX_CONNS` (40), `IDLE_MINUTES` (20),
`DOWNLOAD_DIR` (временная папка), `TOR2VID_LOCAL=1` (режим своего компьютера:
больше раздач, автооткрытие браузера, свободный порт), `FFMPEG_PATH`,
`TRANSCODE_MAX_HEIGHT` (1080), `TRANSCODE_PRESET` (veryfast), `MAX_TRANSCODES` (2).

## Хостинг

`render.yaml` — бесплатный web service на Render. Ограничения бесплатного
тарифа: 512 МБ памяти, засыпает после 15 минут без запросов (первое открытие
~1 минута), диск временный, входящие соединения от пиров не принимаются —
сервер только сам подключается к раздающим, поэтому на редких раздачах
скорость ниже, чем у обычного клиента. Процессора мало: перепаковка H.264
идёт нормально, а полное перекодирование HEVC — с паузами, поэтому там
кадр уменьшается до 480p. Для таких файлов — приложение для Windows.

## Похожие проекты

- [webtor-io/self-hosted](https://github.com/webtor-io/self-hosted) — полноценный webtor.io в Docker, с перекодированием в HLS (тяжёлый, нужен нормальный сервер).
- [YouROK/TorrServer](https://github.com/YouROK/TorrServer) — сервер на Go, популярен с ТВ-плеерами.
- [asapach/peerflix-server](https://github.com/asapach/peerflix-server) — то же на Node, старый (torrent-stream).
- [ferrolho/magnet-player](https://github.com/ferrolho/magnet-player) — только браузер, работает лишь с WebRTC-раздачами.

Используйте только раздачи, которые вам можно смотреть.

[webtorrent]: https://github.com/webtorrent/webtorrent
