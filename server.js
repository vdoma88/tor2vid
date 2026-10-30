// tor2vid — сервер подключается к рою торрента (TCP/UTP/WebRTC + DHT)
// и отдаёт видео браузеру обычным HTTP с Range-запросами.
import express from 'express';
import WebTorrent from 'webtorrent';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const PORT = Number(process.env.PORT) || 3000;
const MAX_TORRENTS = Number(process.env.MAX_TORRENTS) || 3;
const IDLE_MS = (Number(process.env.IDLE_MINUTES) || 20) * 60_000;
const METADATA_TIMEOUT_MS = 60_000;
const DOWNLOAD_DIR = process.env.DOWNLOAD_DIR || path.join(os.tmpdir(), 'tor2vid');

fs.rmSync(DOWNLOAD_DIR, { recursive: true, force: true });
fs.mkdirSync(DOWNLOAD_DIR, { recursive: true });

const client = new WebTorrent({
  maxConns: Number(process.env.MAX_CONNS) || 40,
  // Бесплатные хостинги не пускают входящие соединения — слушать порт незачем.
  torrentPort: 0,
  dhtPort: 0,
});
client.on('error', (err) => console.error('[webtorrent]', err.message));

const lastUsed = new Map(); // infoHash -> timestamp

const PLAYABLE = new Set(['.mp4', '.m4v', '.webm', '.mkv', '.mov', '.ogv', '.mp3', '.m4a', '.ogg', '.wav', '.flac', '.opus']);
const BROWSER_FRIENDLY = new Set(['.mp4', '.m4v', '.webm', '.ogv', '.mp3', '.m4a', '.ogg', '.wav', '.opus']);
const MIME = {
  '.mp4': 'video/mp4', '.m4v': 'video/mp4', '.webm': 'video/webm', '.mkv': 'video/x-matroska',
  '.mov': 'video/quicktime', '.ogv': 'video/ogg', '.avi': 'video/x-msvideo',
  '.mp3': 'audio/mpeg', '.m4a': 'audio/mp4', '.ogg': 'audio/ogg', '.wav': 'audio/wav',
  '.flac': 'audio/flac', '.opus': 'audio/opus',
  '.srt': 'text/plain; charset=utf-8', '.vtt': 'text/vtt; charset=utf-8',
};

const touch = (t) => lastUsed.set(t.infoHash, Date.now());

function describe(t) {
  return {
    infoHash: t.infoHash,
    name: t.name,
    ready: t.ready,
    length: t.length,
    progress: t.progress,
    downloaded: t.downloaded,
    downloadSpeed: t.downloadSpeed,
    uploadSpeed: t.uploadSpeed,
    numPeers: t.numPeers,
    files: (t.files || []).map((f, index) => {
      const ext = path.extname(f.name).toLowerCase();
      return {
        index,
        name: f.name,
        path: f.path,
        length: f.length,
        progress: f.progress,
        playable: PLAYABLE.has(ext),
        browserFriendly: BROWSER_FRIENDLY.has(ext),
        subtitle: ext === '.srt' || ext === '.vtt',
      };
    }),
  };
}

function removeTorrent(t) {
  lastUsed.delete(t.infoHash);
  return new Promise((resolve) => t.destroy({ destroyStore: true }, () => resolve()));
}

async function makeRoom() {
  while (client.torrents.length >= MAX_TORRENTS) {
    const oldest = [...client.torrents].sort(
      (a, b) => (lastUsed.get(a.infoHash) || 0) - (lastUsed.get(b.infoHash) || 0),
    )[0];
    console.log('[evict]', oldest.infoHash);
    await removeTorrent(oldest);
  }
}

function waitReady(t) {
  if (t.ready) return Promise.resolve(t);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error('Не удалось получить описание торрента за 60 секунд: нет пиров или ссылка неверна.'));
    }, METADATA_TIMEOUT_MS);
    const onReady = () => { cleanup(); resolve(t); };
    const onError = (err) => { cleanup(); reject(err); };
    const cleanup = () => {
      clearTimeout(timer);
      t.off('ready', onReady);
      t.off('error', onError);
    };
    t.once('ready', onReady);
    t.once('error', onError);
  });
}

async function addTorrent(source) {
  const existing = await client.get(source).catch(() => null);
  if (existing) {
    touch(existing);
    return waitReady(existing);
  }
  await makeRoom();
  const t = client.add(source, { path: DOWNLOAD_DIR, deselect: true });
  touch(t);
  // deselect: true — качаем только то, что смотрят; файл выбирается при просмотре.
  t.on('ready', () => console.log('[ready]', t.infoHash, t.name));
  try {
    return await waitReady(t);
  } catch (err) {
    await removeTorrent(t);
    throw err;
  }
}

async function findTorrent(hash) {
  return client.get(hash).catch(() => null);
}

const app = express();
app.disable('x-powered-by');
app.use((req, res, next) => {
  res.set({
    'Content-Security-Policy': "default-src 'self'; media-src 'self' blob:; img-src 'self' data:; style-src 'self'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
    'X-Content-Type-Options': 'nosniff',
    'Referrer-Policy': 'no-referrer',
  });
  next();
});
app.use(express.static(path.join(__dirname, 'public'), { maxAge: '1h' }));

app.get('/healthz', (req, res) => res.json({ ok: true, torrents: client.torrents.length }));

// Добавить торрент: JSON { magnet } или тело — сам .torrent файл.
app.post(
  '/api/torrents',
  express.json({ limit: '16kb' }),
  express.raw({ type: ['application/x-bittorrent', 'application/octet-stream'], limit: '5mb' }),
  async (req, res) => {
    let source;
    if (Buffer.isBuffer(req.body) && req.body.length) {
      source = req.body;
    } else if (req.body && typeof req.body.magnet === 'string') {
      const m = req.body.magnet.trim();
      if (/^magnet:\?/i.test(m) || /^[a-f0-9]{40}$/i.test(m)) source = m;
      else if (/^https?:\/\/.+/i.test(m)) source = m; // ссылка на .torrent
    }
    if (!source) return res.status(400).json({ error: 'Нужна magnet-ссылка, info-hash, ссылка на .torrent или сам файл.' });
    try {
      const t = await addTorrent(source);
      res.json(describe(t));
    } catch (err) {
      console.error('[add]', err.message);
      res.status(422).json({ error: err.message || 'Не удалось открыть торрент.' });
    }
  },
);

app.get('/api/torrents', (req, res) => res.json(client.torrents.filter((t) => t.ready).map(describe)));

app.get('/api/torrents/:hash', async (req, res) => {
  const t = await findTorrent(req.params.hash);
  if (!t) return res.status(404).json({ error: 'Торрент не найден — добавьте его заново.' });
  res.json(describe(t));
});

app.delete('/api/torrents/:hash', async (req, res) => {
  const t = await findTorrent(req.params.hash);
  if (t) await removeTorrent(t);
  res.json({ ok: true });
});

// Потоковая отдача файла с поддержкой перемотки (Range).
app.get('/stream/:hash/:index', async (req, res) => {
  const t = await findTorrent(req.params.hash);
  if (!t || !t.ready) return res.status(404).send('Торрент не найден');
  const file = t.files[Number(req.params.index)];
  if (!file) return res.status(404).send('Файл не найден');
  touch(t);
  file.select();

  const total = file.length;
  const ext = path.extname(file.name).toLowerCase();
  res.set({
    'Accept-Ranges': 'bytes',
    'Content-Type': MIME[ext] || 'application/octet-stream',
    'Cache-Control': 'no-store',
  });
  if (req.query.download) {
    res.set('Content-Disposition', `attachment; filename*=UTF-8''${encodeURIComponent(file.name)}`);
  }

  let start = 0;
  let end = total - 1;
  const range = req.headers.range && /bytes=(\d*)-(\d*)/.exec(req.headers.range);
  if (range) {
    if (range[1] === '' && range[2] !== '') {
      start = Math.max(0, total - Number(range[2]));
    } else {
      start = Number(range[1] || 0);
      if (range[2]) end = Math.min(Number(range[2]), total - 1);
    }
    if (start >= total || start > end) {
      res.set('Content-Range', `bytes */${total}`);
      return res.status(416).end();
    }
    res.status(206).set('Content-Range', `bytes ${start}-${end}/${total}`);
  }
  res.set('Content-Length', String(end - start + 1));
  if (req.method === 'HEAD') return res.end();

  const stream = file.createReadStream({ start, end });
  const keepAlive = setInterval(() => touch(t), 30_000);
  const done = () => { clearInterval(keepAlive); stream.destroy(); };
  res.on('close', done);
  stream.on('error', (err) => { console.error('[stream]', err.message); done(); res.destroy(); });
  stream.pipe(res);
});

// Субтитры .srt -> .vtt для <track>.
app.get('/subs/:hash/:index', async (req, res) => {
  const t = await findTorrent(req.params.hash);
  const file = t && t.ready && t.files[Number(req.params.index)];
  if (!file || file.length > 2_000_000) return res.status(404).end();
  touch(t);
  file.select();
  try {
    const buf = Buffer.from(await file.arrayBuffer());
    let text = buf.toString('utf8').replace(/^\uFEFF/, '');
    if (text.includes('\uFFFD')) text = new TextDecoder('windows-1251').decode(buf);
    if (!/^WEBVTT/.test(text)) {
      text = 'WEBVTT\n\n' + text.replace(/\r/g, '').replace(/(\d\d:\d\d:\d\d),(\d\d\d)/g, '$1.$2');
    }
    res.type('text/vtt; charset=utf-8').send(text);
  } catch (err) {
    res.status(500).end();
  }
});

// Чистим торренты, которые давно никто не смотрит.
setInterval(() => {
  const now = Date.now();
  for (const t of [...client.torrents]) {
    if (now - (lastUsed.get(t.infoHash) || 0) > IDLE_MS) {
      console.log('[idle]', t.infoHash);
      removeTorrent(t);
    }
  }
}, 60_000).unref();

const server = app.listen(PORT, () => console.log(`tor2vid: http://localhost:${PORT}`));

function shutdown() {
  server.close();
  client.destroy(() => process.exit(0));
  setTimeout(() => process.exit(0), 3000).unref();
}
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);
