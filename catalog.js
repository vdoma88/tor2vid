// Каталог: старое кино на русском из Internet Archive, у которого в архиве
// стоит отметка «общественное достояние». Раз в сутки сервер берёт самые
// скачиваемые записи, проверяет у каждой торрент и длительность и собирает
// два списка — фильмы и мультфильмы, по 20 штук.
import fs from 'node:fs';
import path from 'node:path';

const IA = 'https://archive.org';
const MAX_YEAR = Number(process.env.CATALOG_MAX_YEAR) || 1950;
const TOP = 20;
const REFRESH_MS = 24 * 3600_000;
const UA = { 'User-Agent': 'tor2vid/1.0 (+https://github.com/vdoma88/tor2vid)' };

// Только явные отметки общественного достояния. CC BY-NC и подобные на
// советском кино почти всегда ставит не правообладатель, а загрузивший.
const PD_LICENSE = /creativecommons\.org\/(publicdomain\/(mark|zero)\/|licenses\/publicdomain)/;
// Не кино: речи, хроника, нарезки, обзоры.
const NOT_FILM = /speech|речь|stock shot|newsreel|киножурнал|parade|парад|funeral|похорон|anthem|гимн|extrait|clip|trailer|трейлер|supercut|excerpt|playing his own|bootleg/i;
const ANIMATION = /anim|cartoon|мульт|soyuzmult|союзмульт/i;

let state = { updatedAt: null, maxYear: MAX_YEAR, films: [], cartoons: [] };
let cacheFile = null;
let refreshing = null;

function plain(html, max = 320) {
  const text = String(html || '')
    .replace(/<br\s*\/?>/gi, ' ')
    .replace(/<[^>]+>/g, ' ')
    .replace(/&nbsp;/g, ' ').replace(/&amp;/g, '&').replace(/&quot;/g, '"').replace(/&#39;/g, "'")
    .replace(/\s+/g, ' ')
    .trim();
  return text.length > max ? text.slice(0, max).replace(/\s+\S*$/, '') + '…' : text;
}

// Русское название: из заголовка («Aerograd (Аероград)») или из описания
// («Chapaev (Чапаев) …», «(Russian: Снежная королева, …»).
export function russianTitle(title, description) {
  const t = String(title || '');
  const inTitle = /([А-ЯЁ][А-Яа-яЁё0-9 .,!?«»"№-]{1,60})/.exec(t);
  if (inTitle) return inTitle[1].replace(/[\s,.-]+$/, '').trim();
  const d = plain(description, 2000);
  const m = /\((?:Russian|рус\.?)?:?\s*([А-ЯЁ][А-Яа-яЁё0-9 .!?«»"№-]{1,60}?)\s*(?:[,;)]|\s+romanized)/.exec(d);
  return m ? m[1].trim() : null;
}

// Ключ для склейки копий одного фильма («Battleship Potemkin (1925) - A Film…»).
function dedupeKey(title, year) {
  const base = String(title || '')
    .toLowerCase()
    .split(/[([\]|:]| - /)[0]
    .replace(/\b(19|20)\d\d\b/g, '')
    .replace(/[^a-zа-яё0-9]+/gi, ' ')
    .trim();
  return `${base}|${year || ''}`;
}

function seconds(v) {
  if (v == null) return 0;
  const s = String(v);
  if (/^\d+(\.\d+)?$/.test(s)) return Number(s);
  const parts = s.split(':').map(Number);
  if (parts.some(Number.isNaN)) return 0;
  return parts.reduce((acc, n) => acc * 60 + n, 0);
}

async function getJson(url) {
  const res = await fetch(url, { headers: UA, signal: AbortSignal.timeout(60_000) });
  if (!res.ok) throw new Error(`${res.status} ${url}`);
  return res.json();
}

async function search() {
  const q = `mediatype:movies AND (language:rus OR language:Russian OR language:ru OR language:"Русский") AND licenseurl:[* TO *] AND year:[1900 TO ${MAX_YEAR}]`;
  const fl = ['identifier', 'title', 'year', 'downloads', 'licenseurl', 'subject'];
  const url = `${IA}/advancedsearch.php?q=${encodeURIComponent(q)}&${fl.map((f) => `fl[]=${f}`).join('&')}&sort[]=downloads+desc&rows=1000&output=json`;
  const data = await getJson(url);
  return data.response.docs;
}

async function details(doc) {
  const meta = await getJson(`${IA}/metadata/${encodeURIComponent(doc.identifier)}`);
  const files = meta.files || [];
  const torrent = files.find((f) => f.name.endsWith('_archive.torrent'));
  if (!torrent) return null;
  const videos = files.filter((f) => f.source === 'original' && /\.(mp4|m4v|webm|mkv|avi|mov|mpe?g|ogv)$/i.test(f.name));
  const lengths = files.map((f) => seconds(f.length)).filter(Boolean);
  const longest = lengths.length ? Math.max(...lengths) : seconds(meta.metadata.runtime);
  const m = meta.metadata;
  const subject = [].concat(m.subject || []).join('; ');
  return {
    id: doc.identifier,
    title: plain(m.title, 140),
    titleRu: russianTitle(m.title, m.description),
    year: Number(String(m.year || doc.year).slice(0, 4)) || null,
    minutes: longest ? Math.round(longest / 60) : null,
    downloads: doc.downloads || 0,
    description: plain(m.description),
    silent: /silent|немое|немой/i.test(subject + ' ' + m.title) || (Number(m.year) || 9999) < 1931,
    animation: ANIMATION.test(`${subject} ${m.title}`),
    videoFiles: videos.length,
    torrent: `${IA}/download/${encodeURIComponent(doc.identifier)}/${encodeURIComponent(torrent.name)}`,
    page: `${IA}/details/${encodeURIComponent(doc.identifier)}`,
  };
}

// Русское название из Википедии: статья о фильме в английской → ссылка на русскую.
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const words = (s) => String(s).toLowerCase().replace(/[^a-z0-9а-яё ]+/gi, ' ').split(/\s+/).filter((w) => w.length > 2);

async function wikiRussianTitle(item) {
  const base = item.title.split(/[([\]|]| - /)[0].replace(/\b(19|20)\d\d\b/g, '').replace(/^\d{4}:\s*/, '').trim();
  if (!base) return null;
  const q = `${base} ${item.year || ''} ${item.animation ? 'animated' : ''} film`;
  const url = `https://en.wikipedia.org/w/api.php?action=query&format=json&generator=search&gsrlimit=1&prop=langlinks&lllang=ru&gsrsearch=${encodeURIComponent(q)}`;
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      const r = await fetch(url, { headers: UA, signal: AbortSignal.timeout(20_000) });
      if (r.status === 429 || r.status >= 500) { await sleep(2000 * (attempt + 1)); continue; }
      const data = await r.json();
      const page = Object.values((data.query && data.query.pages) || {})[0];
      if (!page || !page.langlinks) return null;
      // Найденная статья должна быть о том же: общее значимое слово в названии.
      const a = new Set(words(base));
      if (!words(page.title).some((w) => a.has(w))) return null;
      return page.langlinks[0]['*'].replace(/\s*\((?:мульт)?фильм[^)]*\)$/i, '').trim() || null;
    } catch {
      await sleep(1500);
    }
  }
  return null;
}

async function pool(items, limit, fn) {
  const out = [];
  let i = 0;
  const worker = async () => {
    while (i < items.length) {
      const item = items[i++];
      try { out.push(await fn(item)); } catch (err) { console.error('[catalog]', item.identifier, err.message); }
    }
  };
  await Promise.all(Array.from({ length: limit }, worker));
  return out;
}

export async function buildCatalog() {
  const docs = await search();
  const seen = new Set();
  const candidates = [];
  for (const d of docs) {
    const title = Array.isArray(d.title) ? d.title[0] : d.title;
    if (!PD_LICENSE.test(d.licenseurl || '')) continue;
    if (NOT_FILM.test(title || '')) continue;
    const key = dedupeKey(title, d.year);
    if (seen.has(key)) continue;
    seen.add(key);
    candidates.push(d);
    if (candidates.length >= 120) break;
  }
  const items = (await pool(candidates, 4, details)).filter(Boolean);
  items.sort((a, b) => b.downloads - a.downloads);
  // Ещё раз склеиваем копии — уже по русскому названию и году.
  const seenRu = new Set();
  const unique = items.filter((x) => {
    if (!x.titleRu) return true;
    const k = `${x.titleRu.toLowerCase()}|${x.year}`;
    if (seenRu.has(k)) return false;
    seenRu.add(k);
    return true;
  });
  const cartoons = unique.filter((x) => x.animation && (x.minutes || 0) >= 5).slice(0, TOP);
  const films = unique.filter((x) => !x.animation && (x.minutes || 0) >= 40).slice(0, TOP);
  for (const x of [...films, ...cartoons]) {
    if (!x.titleRu) {
      x.titleRu = await wikiRussianTitle(x);
      await sleep(400);
    }
  }
  return { updatedAt: new Date().toISOString(), maxYear: MAX_YEAR, films, cartoons };
}

export function getCatalog() {
  return state;
}

export async function refreshCatalog() {
  if (refreshing) return refreshing;
  refreshing = (async () => {
    try {
      const next = await buildCatalog();
      if (next.films.length || next.cartoons.length) {
        state = next;
        if (cacheFile) fs.writeFile(cacheFile, JSON.stringify(state, null, 1), () => {});
        console.log(`[catalog] фильмов: ${state.films.length}, мультфильмов: ${state.cartoons.length}`);
      }
    } catch (err) {
      console.error('[catalog] не обновился:', err.message);
    } finally {
      refreshing = null;
    }
  })();
  return refreshing;
}

// Стартуем со снимка (из репозитория или с прошлого запуска), потом обновляем.
export function startCatalog({ cache, seed }) {
  cacheFile = cache;
  for (const file of [cache, seed]) {
    try {
      const data = JSON.parse(fs.readFileSync(file, 'utf8'));
      if (data && (data.films || []).length) { state = data; break; }
    } catch { /* нет файла */ }
  }
  const age = state.updatedAt ? Date.now() - Date.parse(state.updatedAt) : Infinity;
  if (age > REFRESH_MS) refreshCatalog();
  setInterval(refreshCatalog, REFRESH_MS).unref();
}

// Обложки отдаём сами: строгий CSP и без запросов браузера к archive.org.
const ID_RE = /^[A-Za-z0-9._-]{1,120}$/;
export async function coverHandler(req, res, imgDir) {
  const id = req.params.id;
  if (!ID_RE.test(id)) return res.status(400).end();
  const file = path.join(imgDir, id + '.jpg');
  res.set('Cache-Control', 'public, max-age=604800');
  if (fs.existsSync(file)) return res.type('image/jpeg').sendFile(file);
  try {
    const r = await fetch(`${IA}/services/img/${encodeURIComponent(id)}`, { headers: UA, signal: AbortSignal.timeout(30_000) });
    const type = r.headers.get('content-type') || '';
    if (!r.ok || !type.startsWith('image/')) return res.status(404).end();
    const buf = Buffer.from(await r.arrayBuffer());
    fs.mkdirSync(imgDir, { recursive: true });
    fs.writeFile(file, buf, () => {});
    res.type(type.split(';')[0]).send(buf);
  } catch {
    res.status(502).end();
  }
}
