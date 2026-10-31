// tor2vid — сервер подключается к рою торрента (TCP/UTP/WebRTC + DHT)
// и отдаёт видео браузеру обычным HTTP с Range-запросами.
import express from 'express';
import WebTorrent from 'webtorrent';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import {
  FFMPEG, ffmpegAvailable, probe, plan, transcode, jobSpeed, forgetProbes, seekPoint, liveSubs, setSubsDir, setInputHeader,
} from './transcode.js';
import { startCatalog, getCatalog, coverHandler } from './catalog.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const LOCAL = process.env.TOR2VID_LOCAL === '1';

const PORT = Number(process.env.PORT) || 3000;
let listenPort = PORT;
const HAS_FFMPEG = await ffmpegAvailable();
console.log(HAS_FFMPEG ? `ffmpeg: ${FFMPEG}` : 'ffmpeg не найден — перекодирование выключено');
// На своём компьютере ресурсов больше — держим больше раздач и дольше.
const MAX_TORRENTS = Number(process.env.MAX_TORRENTS) || (LOCAL ? 6 : 3);
const IDLE_MS = (Number(process.env.IDLE_MINUTES) || (LOCAL ? 180 : 20)) * 60_000;
const METADATA_TIMEOUT_MS = 60_000;
const DOWNLOAD_DIR = process.env.DOWNLOAD_DIR || path.join(os.tmpdir(), 'tor2vid');

fs.rmSync(DOWNLOAD_DIR, { recursive: true, force: true });
fs.mkdirSync(DOWNLOAD_DIR, { recursive: true });
setSubsDir(path.join(DOWNLOAD_DIR, '.subs'));

// Только просмотр: сервер ничего не отдаёт другим участникам раздачи.
// Порт не пробрасываем (UPnP/NAT-PMP выключены), соседей в локальной сети не ищем,
// а каждое соединение держим «заблокированным» (choke) — см. noUpload().
const client = new WebTorrent({
  maxConns: Number(process.env.MAX_CONNS) || (LOCAL ? 80 : 40),
  torrentPort: 0,
  dhtPort: 0,
  natUpnp: false,
  natPmp: false,
  lsd: false,
});
client.on('error', (err) => console.error('[webtorrent]', err.message));
client.on('listening', () => console.log('[torrent] порт', client.torrentPort, '— только входящие запросы, отдача выключена'));

const lastUsed = new Map(); // infoHash -> timestamp
const owners = new Map(); // infoHash -> Set(id сеанса): кто открыл этот торрент

// По протоколу BitTorrent участник, которого мы «душим» (choke), не может
// получить от нас ни куска: на его запросы библиотека отвечает отказом.
// Снимать блокировку не даём никому, поэтому отдача невозможна.
function noUpload(wire) {
  wire.unchoke = () => {};
  wire.allowedFast = () => {};
  wire.choke();
  // Вторая защита на уровне протокола: отправка куска файла просто не происходит.
  wire.piece = (index, offset, buffer) => {
    if (wire.hasFast) wire.reject(index, offset, buffer ? buffer.length : 0);
  };
  // Описание торрента (имена и размеры файлов) другим тоже не пересылаем…
  if (wire.ut_metadata) wire.ut_metadata._onRequest = (piece) => wire.ut_metadata._reject(piece);
  // …как и адреса других участников (PEX).
  if (wire.ut_pex) wire.ut_pex._sendMessage = () => {};
}

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

function describe(t, withJobs = false) {
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
        transcodeSpeed: withJobs ? jobSpeed(`${t.infoHash}:${index}`) : null,
      };
    }),
  };
}

function removeTorrent(t) {
  lastUsed.delete(t.infoHash);
  owners.delete(t.infoHash);
  forgetProbes(t.infoHash);
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

function own(t, sid) {
  if (!owners.has(t.infoHash)) owners.set(t.infoHash, new Set());
  owners.get(t.infoHash).add(sid);
}

async function addTorrent(source, sid) {
  const existing = await client.get(source).catch(() => null);
  if (existing) {
    touch(existing);
    own(existing, sid);
    return waitReady(existing);
  }
  await makeRoom();
  // uploads: false — не раздаём никому даже по расписанию библиотеки.
  const t = client.add(source, { path: DOWNLOAD_DIR, deselect: true, uploads: false });
  t.on('wire', noUpload);
  touch(t);
  // deselect: true — качаем только то, что смотрят; файл выбирается при просмотре.
  t.on('ready', () => console.log('[ready]', t.infoHash, t.name));
  try {
    await waitReady(t);
    // info-hash у magnet-ссылки известен только после разбора — записываем владельца здесь.
    own(t, sid);
    touch(t);
    return t;
  } catch (err) {
    await removeTorrent(t);
    throw err;
  }
}

async function findTorrent(hash) {
  return client.get(hash).catch(() => null);
}

// ---------- приватные сеансы ----------
// У каждого браузера свой сеанс (cookie). Смотреть можно только то, что открыл сам:
// чужие ссылки на поток не работают, и раздать поток другим по адресу нельзя.
const INTERNAL = crypto.randomBytes(24).toString('hex'); // для ffmpeg на этом же сервере
setInputHeader(`X-Tor2vid-Internal: ${INTERNAL}`);

function sessionId(req) {
  const m = /(?:^|;\s*)sid=([a-f0-9]{32})/.exec(req.headers.cookie || '');
  return m ? m[1] : null;
}

function isInternal(req) {
  const ip = req.socket.remoteAddress || '';
  return req.get('x-tor2vid-internal') === INTERNAL && /^(::ffff:)?127\.0\.0\.1$|^::1$/.test(ip);
}

// Торрент, если он открыт этим сеансом (или запрос от нашего же ffmpeg).
async function ownedTorrent(req, hash) {
  const t = await findTorrent(hash);
  if (!t) return null;
  if (isInternal(req)) return t;
  const sid = sessionId(req);
  return sid && owners.get(t.infoHash) && owners.get(t.infoHash).has(sid) ? t : null;
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
app.set('trust proxy', 1);
app.use((req, res, next) => {
  if (!sessionId(req) && !isInternal(req)) {
    const sid = crypto.randomBytes(16).toString('hex');
    req.headers.cookie = `${req.headers.cookie ? req.headers.cookie + '; ' : ''}sid=${sid}`;
    res.cookie('sid', sid, { httpOnly: true, sameSite: 'strict', secure: req.secure, maxAge: 30 * 24 * 3600_000, path: '/' });
  }
  next();
});
app.use(express.static(path.join(__dirname, 'public'), { maxAge: '1h' }));

app.get('/healthz', (req, res) => res.json({ ok: true, torrents: client.torrents.length }));

app.get('/api/config', (req, res) => res.json({ local: LOCAL, ffmpeg: HAS_FFMPEG }));

// Каталог: старое кино из Internet Archive, обновляется раз в сутки.
startCatalog({
  cache: path.join(os.tmpdir(), 'tor2vid-catalog.json'),
  seed: path.join(__dirname, 'data', 'catalog.json'),
});
app.get('/api/catalog', (req, res) => res.set('Cache-Control', 'public, max-age=600').json(getCatalog()));
app.get('/cover/:id', (req, res) => coverHandler(req, res, path.join(os.tmpdir(), 'tor2vid-covers')));

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
      const t = await addTorrent(source, sessionId(req));
      res.json(describe(t));
    } catch (err) {
      console.error('[add]', err.message);
      res.status(422).json({ error: err.message || 'Не удалось открыть торрент.' });
    }
  },
);

app.get('/api/torrents', (req, res) => {
  const sid = sessionId(req);
  res.json(client.torrents.filter((t) => t.ready && owners.get(t.infoHash) && owners.get(t.infoHash).has(sid)).map((t) => describe(t)));
});

app.get('/api/torrents/:hash', async (req, res) => {
  const t = await ownedTorrent(req, req.params.hash);
  if (!t) return res.status(404).json({ error: 'Торрент не найден — добавьте его заново.' });
  res.json(describe(t, true));
});

// Закрыл — забываем; когда торрент больше никому не нужен, удаляем его вместе с файлами.
app.delete('/api/torrents/:hash', async (req, res) => {
  const t = await ownedTorrent(req, req.params.hash);
  if (t) {
    const set = owners.get(t.infoHash);
    if (set) set.delete(sessionId(req));
    if (!set || !set.size) await removeTorrent(t);
  }
  res.json({ ok: true });
});

// Потоковая отдача файла для плеера (Range — для перемотки). Только своему сеансу.
app.get('/stream/:hash/:index', async (req, res) => {
  const t = await ownedTorrent(req, req.params.hash);
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
    'Cache-Control': 'no-store, private',
    'Content-Disposition': 'inline',
  });

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

// ---------- перекодирование ----------
const loopback = (hash, index) => `http://127.0.0.1:${listenPort}/stream/${hash}/${index}`;

async function fileFor(req, res) {
  if (!HAS_FFMPEG) { res.status(501).json({ error: 'На этом сервере нет ffmpeg.' }); return null; }
  const t = await ownedTorrent(req, req.params.hash);
  const index = Number(req.params.index);
  if (!t || !t.ready || !t.files[index]) { res.status(404).json({ error: 'Файл не найден — откройте торрент заново.' }); return null; }
  touch(t);
  return { t, index, key: `${t.infoHash}:${index}` };
}

// Что внутри файла и как его будем показывать.
app.get('/api/probe/:hash/:index', async (req, res) => {
  const f = await fileFor(req, res);
  if (!f) return;
  try {
    const info = await probe(f.key, loopback(f.t.infoHash, f.index));
    const p = plan(info, 0);
    res.json({ ...info, copyVideo: p.copyVideo, copyAudio: p.copyAudio, heavy: p.heavy, local: LOCAL });
  } catch (err) {
    res.status(422).json({ error: err.message });
  }
});

// С какой секунды реально начнётся поток, если попросить ?t=.
app.get('/api/start/:hash/:index', async (req, res) => {
  const f = await fileFor(req, res);
  if (!f) return;
  try {
    const url = loopback(f.t.infoHash, f.index);
    const info = await probe(f.key, url);
    res.json({ start: await seekPoint(f.key, url, info, req.query.t) });
  } catch (err) {
    res.status(422).json({ error: err.message });
  }
});

// Встроенные субтитры, вытащенные текущим перекодированием. ?from= — сколько реплик уже есть.
app.get('/api/live-subs/:hash/:index/:n', async (req, res) => {
  const t = await ownedTorrent(req, req.params.hash);
  if (!t) return res.status(404).json({ error: 'Торрент не найден.' });
  const data = liveSubs(`${t.infoHash}:${Number(req.params.index)}`, Number(req.params.n));
  if (!data) return res.json({ start: null, done: false, total: 0, cues: [] });
  const from = Math.max(0, Number(req.query.from) || 0);
  res.set('Cache-Control', 'no-store').json({ start: data.start, done: data.done, total: data.cues.length, cues: data.cues.slice(from) });
});

// Поток MP4. ?t= — с какой секунды, ?a= — номер звуковой дорожки.
app.get('/transcode/:hash/:index', async (req, res) => {
  const f = await fileFor(req, res);
  if (!f) return;
  try {
    const url = loopback(f.t.infoHash, f.index);
    const info = await probe(f.key, url);
    const start = await seekPoint(f.key, url, info, req.query.t);
    const audioIndex = Math.max(0, Math.min(info.audio.length - 1, Number(req.query.a) || 0));
    const keepAlive = setInterval(() => touch(f.t), 30_000);
    res.on('close', () => clearInterval(keepAlive));
    transcode({ key: f.key, url: loopback(f.t.infoHash, f.index), info, start, audioIndex, res });
  } catch (err) {
    res.status(422).json({ error: err.message });
  }
});

// Субтитры .srt -> .vtt для <track>.
app.get('/subs/:hash/:index', async (req, res) => {
  const t = await ownedTorrent(req, req.params.hash);
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

function openBrowser(url) {
  const cmd = process.platform === 'win32' ? ['cmd', ['/c', 'start', '', url]]
    : process.platform === 'darwin' ? ['open', [url]] : ['xdg-open', [url]];
  try {
    spawn(cmd[0], cmd[1], { stdio: 'ignore', detached: true }).on('error', () => {}).unref();
  } catch { /* нет браузера — адрес уже напечатан */ }
}

// Локально: если порт занят, берём следующий.
let server;
function listen(port, triesLeft) {
  server = app.listen(port, LOCAL ? '127.0.0.1' : undefined);
  server.once('listening', () => {
    listenPort = server.address().port;
    const url = `http://localhost:${listenPort}`;
    console.log(`tor2vid: ${url}`);
    if (LOCAL) {
      console.log('Окно не закрывайте, пока смотрите. Остановить — Ctrl+C или закрыть окно.');
      if (process.env.OPEN_BROWSER !== '0') openBrowser(url);
    }
  });
  server.once('error', (err) => {
    if (err.code === 'EADDRINUSE' && LOCAL && triesLeft > 0) listen(port + 1, triesLeft - 1);
    else { console.error(err.message); process.exit(1); }
  });
}
listen(PORT, 20);

function shutdown() {
  server.close();
  client.destroy(() => process.exit(0));
  setTimeout(() => process.exit(0), 3000).unref();
}
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);
