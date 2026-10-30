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

Качается только то, что смотрят. Торренты, которые не открывали 20 минут,
удаляются вместе с файлами; одновременно держится не больше `MAX_TORRENTS`.

Браузер играет MP4/WebM (H.264, VP9, AV1). MKV — если внутри H.264 + AAC/Opus;
HEVC, AC3/DTS и AVI браузер обычно не тянет — для них есть «Ссылка для VLC»
(тот же поток, VLC его перематывает).

## Запуск

```bash
npm install
npm start          # http://localhost:3000
```

Переменные: `PORT`, `MAX_TORRENTS` (3), `MAX_CONNS` (40), `IDLE_MINUTES` (20),
`DOWNLOAD_DIR` (временная папка).

## Хостинг

`render.yaml` — бесплатный web service на Render. Ограничения бесплатного
тарифа: 512 МБ памяти, засыпает после 15 минут без запросов (первое открытие
~1 минута), диск временный, входящие соединения от пиров не принимаются —
сервер только сам подключается к раздающим, поэтому на редких раздачах
скорость ниже, чем у обычного клиента.

## Похожие проекты

- [webtor-io/self-hosted](https://github.com/webtor-io/self-hosted) — полноценный webtor.io в Docker, с перекодированием в HLS (тяжёлый, нужен нормальный сервер).
- [YouROK/TorrServer](https://github.com/YouROK/TorrServer) — сервер на Go, популярен с ТВ-плеерами.
- [asapach/peerflix-server](https://github.com/asapach/peerflix-server) — то же на Node, старый (torrent-stream).
- [ferrolho/magnet-player](https://github.com/ferrolho/magnet-player) — только браузер, работает лишь с WebRTC-раздачами.

Используйте только раздачи, которые вам можно смотреть.

[webtorrent]: https://github.com/webtorrent/webtorrent
