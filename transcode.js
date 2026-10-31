// Перекодирование через ffmpeg: всё, что браузер не играет сам (MKV, AVI,
// HEVC, AC3/DTS…), превращается на лету во фрагментированный MP4.
// Если видео уже H.264, оно только перепаковывается — это почти бесплатно.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

let ffmpegPath = process.env.FFMPEG_PATH || null;
if (!ffmpegPath) {
  try {
    const mod = await import('ffmpeg-static');
    if (mod.default && fs.existsSync(mod.default)) ffmpegPath = mod.default;
  } catch { /* пакет не установлен */ }
}
if (!ffmpegPath) ffmpegPath = 'ffmpeg'; // системный, если есть

export const FFMPEG = ffmpegPath;

// ffmpeg читает поток с нашего же сервера; заголовок доказывает, что это он.
let inputHeader = null;
export function setInputHeader(h) { inputHeader = h; }
const input = (url) => (inputHeader ? ['-headers', `${inputHeader}\r\n`, '-i', url] : ['-i', url]);

const MAX_HEIGHT = Number(process.env.TRANSCODE_MAX_HEIGHT) || 1080;
const PRESET = process.env.TRANSCODE_PRESET || 'veryfast';
const MAX_JOBS = Number(process.env.MAX_TRANSCODES) || 2;

// Что браузеры понимают внутри MP4.
const VIDEO_COPY = new Set(['h264']);
const AUDIO_COPY = new Set(['aac', 'mp3']);
// Текстовые субтитры ffmpeg умеет превратить в WebVTT; PGS/VobSub — картинки, их нет.
const TEXT_SUBS = new Set(['subrip', 'srt', 'ass', 'ssa', 'webvtt', 'mov_text', 'text', 'microdvd',
  'subviewer', 'subviewer1', 'sami', 'realtext', 'jacosub', 'mpl2', 'pjs', 'vplayer', 'stl']);
const MAX_SUBS = 12;

let subsDir = null;
export function setSubsDir(dir) {
  subsDir = dir;
  fs.mkdirSync(dir, { recursive: true });
}
let jobSeq = 0;
const seekCache = new Map(); // "key@t" -> секунда ключевого кадра

const probeCache = new Map(); // ключ "hash:index" -> Promise<probe>
const jobs = new Map(); // ключ -> { id, proc, speed, started, start, subs, done }
const lastJobs = new Map(); // ключ -> последний завершившийся (субтитры ещё нужны)

export function ffmpegAvailable() {
  return new Promise((resolve) => {
    const p = spawn(FFMPEG, ['-hide_banner', '-version'], { stdio: 'ignore' });
    p.on('error', () => resolve(false));
    p.on('exit', (code) => resolve(code === 0));
  });
}

function parseTime(h, m, s) {
  return Number(h) * 3600 + Number(m) * 60 + Number(s);
}

// Разбор вывода `ffmpeg -i`: длительность, видео и звуковые дорожки.
export function parseProbe(stderr) {
  const out = { duration: null, video: null, audio: [], subtitles: [] };
  const d = /Duration: (\d+):(\d+):(\d+(?:\.\d+)?)/.exec(stderr);
  if (d) out.duration = parseTime(d[1], d[2], d[3]);

  const lines = stderr.split(/\r?\n/);
  let last = null;
  for (const line of lines) {
    const m = /^\s*Stream #0:(\d+)(?:\[[^\]]*\])?(?:\(([^)]+)\))?: (Video|Audio|Subtitle): ([\w-]+)(.*)$/.exec(line);
    if (m) {
      const [, , lang, kind, codec, rest] = m;
      if (kind === 'Video') {
        if (out.video || /attached pic/.test(rest)) { last = null; continue; }
        const size = /\b(\d{2,5})x(\d{2,5})\b/.exec(rest);
        out.video = {
          codec,
          tenBit: /yuv4\d\dp1[02]|p010|10le|12le/.test(rest),
          width: size ? Number(size[1]) : null,
          height: size ? Number(size[2]) : null,
        };
        last = null;
      } else if (kind === 'Audio') {
        const ch = /(mono|stereo|5\.1|7\.1|\d+ channels)/.exec(rest);
        last = { index: out.audio.length, codec, lang: lang && lang !== 'und' ? lang : null, title: null, channels: ch ? ch[1] : null, default: /\(default\)/.test(rest) };
        out.audio.push(last);
      } else {
        last = {
          index: out.subtitles.length,
          codec,
          lang: lang && lang !== 'und' ? lang : null,
          title: null,
          text: TEXT_SUBS.has(codec),
          forced: /\(forced\)/.test(rest),
          default: /\(default\)/.test(rest),
        };
        out.subtitles.push(last);
      }
      continue;
    }
    const t = /^\s+title\s*:\s*(.+)$/.exec(line);
    if (t && last && !last.title) last.title = t[1].trim();
  }
  return out;
}

export function probe(key, url) {
  if (probeCache.has(key)) return probeCache.get(key);
  const p = new Promise((resolve, reject) => {
    const proc = spawn(FFMPEG, ['-hide_banner', ...input(url)], { stdio: ['ignore', 'ignore', 'pipe'] });
    let err = '';
    const timer = setTimeout(() => proc.kill('SIGKILL'), 90_000);
    proc.stderr.on('data', (c) => { err += c; });
    proc.on('error', (e) => { clearTimeout(timer); reject(e); });
    proc.on('exit', () => {
      clearTimeout(timer);
      const info = parseProbe(err);
      if (!info.video && !info.audio.length) reject(new Error('Не удалось прочитать файл: нет пиров или формат не распознан.'));
      else resolve(info);
    });
  });
  probeCache.set(key, p);
  p.catch(() => probeCache.delete(key));
  return p;
}

export function forgetProbes(infoHash) {
  for (const k of probeCache.keys()) if (k.startsWith(infoHash + ':')) probeCache.delete(k);
  for (const k of seekCache.keys()) if (k.startsWith(infoHash + ':')) seekCache.delete(k);
  for (const [k, job] of lastJobs) {
    if (k.startsWith(infoHash + ':')) { removeJobFiles(job); lastJobs.delete(k); }
  }
  for (const [k, job] of jobs) {
    if (k.startsWith(infoHash + ':')) {
      job.proc.kill('SIGKILL');
      removeJobFiles(job);
      jobs.delete(k);
    }
  }
}

function removeJobFiles(job) {
  for (const file of job.subs.values()) fs.rm(file, { force: true }, () => {});
}

// Откуда реально начнётся поток. При перепаковке видео (-c:v copy) ffmpeg
// начинает с ключевого кадра перед точкой перемотки — находим его, чтобы
// видео, звук и субтитры имели общий ноль. При перекодировании — ровно t.
export async function seekPoint(key, url, info, t) {
  t = Math.max(0, Number(t) || 0);
  if (!t || !plan(info).copyVideo) return t;
  const ck = `${key}@${t}`;
  if (seekCache.has(ck)) return seekCache.get(ck);
  const k = await new Promise((resolve) => {
    const proc = spawn(FFMPEG, ['-hide_banner', '-loglevel', 'error', '-ss', String(t), ...input(url),
      '-map', '0:v:0', '-c', 'copy', '-copyts', '-frames:v', '1', '-f', 'framecrc', '-'],
    { stdio: ['ignore', 'pipe', 'ignore'] });
    let outText = '';
    const timer = setTimeout(() => proc.kill('SIGKILL'), 30_000);
    proc.stdout.on('data', (c) => { outText += c; });
    proc.on('error', () => { clearTimeout(timer); resolve(t); });
    proc.on('exit', () => {
      clearTimeout(timer);
      const tb = /^#tb 0: (\d+)\/(\d+)/m.exec(outText);
      const pkt = /^0,\s*-?\d+,\s*(-?\d+),/m.exec(outText);
      if (!tb || !pkt) return resolve(t);
      const sec = (Number(pkt[1]) * Number(tb[1])) / Number(tb[2]);
      // +1 мс, чтобы при повторном -ss ffmpeg попал ровно в этот же кадр.
      resolve(sec >= 0 && sec <= t ? Math.round(sec * 1000) / 1000 + 0.001 : t);
    });
  });
  seekCache.set(ck, k);
  return k;
}

// Субтитры, которые ffmpeg уже успел вытащить, в виде списка реплик.
// Время — от начала текущего потока (совпадает с video.currentTime).
export function liveSubs(key, n) {
  const job = jobs.get(key) || lastJobs.get(key);
  if (!job || !job.subs.has(n)) return null;
  let text = '';
  try { text = fs.readFileSync(job.subs.get(n), 'utf8'); } catch { /* ещё не создан */ }
  return { start: job.start, done: job.done, cues: parseVtt(text) };
}

const ts = (v) => {
  const m = /^(?:(\d+):)?(\d+):(\d+)\.(\d+)$/.exec(v);
  return m ? Number(m[1] || 0) * 3600 + Number(m[2]) * 60 + Number(m[3]) + Number(m[4]) / 1000 : null;
};

export function parseVtt(text) {
  const cues = [];
  const blocks = text.replace(/\r/g, '').split(/\n{2,}/);
  for (const b of blocks) {
    const lines = b.split('\n');
    const i = lines.findIndex((l) => l.includes('-->'));
    if (i < 0) continue;
    const m = /^\s*(\S+)\s+-->\s+(\S+)/.exec(lines[i]);
    if (!m) continue;
    const s0 = ts(m[1]);
    const e0 = ts(m[2]);
    const body = lines.slice(i + 1).join('\n').trim();
    // Реплики, начавшиеся до точки старта, ffmpeg пишет с испорченным временем — пропускаем.
    if (s0 == null || e0 == null || !body) continue;
    cues.push({ s: s0, e: e0, text: body });
  }
  // Последний блок может быть дописан не до конца — его заберём в следующий раз.
  if (cues.length && !/\n\n$/.test(text.replace(/\r/g, ''))) cues.pop();
  return cues;
}

// План: что копировать, что перекодировать.
export function plan(info, audioIndex = 0) {
  const v = info.video;
  const a = info.audio[audioIndex] || info.audio[0] || null;
  const copyVideo = Boolean(v && VIDEO_COPY.has(v.codec) && !v.tenBit && (!v.height || v.height <= Math.max(MAX_HEIGHT, 1080)));
  const copyAudio = Boolean(a && AUDIO_COPY.has(a.codec));
  return { copyVideo, copyAudio, audio: a, heavy: Boolean(v) && !copyVideo };
}

export function jobSpeed(key) {
  const j = jobs.get(key);
  return j ? j.speed : null;
}

export function transcode({ key, url, info, start = 0, audioIndex = 0, res }) {
  const { copyVideo, copyAudio, audio } = plan(info, audioIndex);

  // Один зритель — одно перекодирование этого файла; лишние старые гасим.
  const old = jobs.get(key);
  if (old) old.proc.kill('SIGKILL');
  const prev = lastJobs.get(key);
  if (prev) { removeJobFiles(prev); lastJobs.delete(key); }
  if (old) { removeJobFiles(old); jobs.delete(key); }
  while (jobs.size >= MAX_JOBS) {
    const [k, j] = [...jobs.entries()].sort((x, y) => x[1].started - y[1].started)[0];
    j.proc.kill('SIGKILL');
    removeJobFiles(j);
    jobs.delete(k);
  }

  const args = ['-hide_banner', '-loglevel', 'error', '-nostats', '-progress', 'pipe:2'];
  if (start > 0) args.push('-ss', String(start));
  args.push(...input(url));
  if (info.video) args.push('-map', '0:v:0');
  if (audio) args.push('-map', `0:a:${audio.index}`);

  if (info.video) {
    if (copyVideo) {
      args.push('-c:v', 'copy');
    } else {
      args.push(
        '-c:v', 'libx264', '-preset', PRESET, '-crf', '23', '-pix_fmt', 'yuv420p',
        '-vf', `scale=-2:'min(${MAX_HEIGHT},ih)':flags=fast_bilinear`,
        '-g', '48', '-keyint_min', '48', '-sc_threshold', '0',
      );
    }
  }
  if (audio) {
    if (copyAudio) args.push('-c:a', 'copy');
    else args.push('-c:a', 'aac', '-b:a', '160k', '-ac', '2');
  } else {
    args.push('-an');
  }
  args.push(
    '-sn', '-dn', '-max_muxing_queue_size', '2048',
    '-f', 'mp4', '-movflags', 'frag_keyframe+empty_moov+default_base_moof',
    'pipe:1',
  );

  // Тот же проход ffmpeg заодно вытаскивает встроенные текстовые субтитры
  // в отдельные .vtt — по мере того как идёт видео, не скачивая файл целиком.
  const id = ++jobSeq;
  const subs = new Map();
  if (subsDir) {
    for (const sub of info.subtitles.filter((x) => x.text).slice(0, MAX_SUBS)) {
      const file = path.join(subsDir, `${key.replace(/\W/g, '_')}_${id}_${sub.index}.vtt`);
      subs.set(sub.index, file);
      args.push('-map', `0:s:${sub.index}`, '-c:s', 'webvtt', '-avoid_negative_ts', 'disabled',
        '-flush_packets', '1', '-f', 'webvtt', '-y', file);
    }
  }

  const proc = spawn(FFMPEG, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  const job = { id, proc, speed: null, started: Date.now(), start, subs, done: false };
  jobs.set(key, job);

  let errTail = '';
  proc.stderr.on('data', (chunk) => {
    const s = chunk.toString();
    const sp = /speed=\s*([\d.]+)x/.exec(s);
    if (sp) job.speed = Number(sp[1]);
    else if (!/^\w+=/.test(s)) errTail = (errTail + s).slice(-500);
  });
  proc.on('error', (e) => { console.error('[ffmpeg]', e.message); res.destroy(); });
  proc.on('exit', (code, signal) => {
    job.done = true;
    if (jobs.get(key) === job) {
      jobs.delete(key);
      lastJobs.set(key, job);
    }
    if (code && !signal) console.error('[ffmpeg] exit', code, errTail.trim());
  });

  res.status(200).set({ 'Content-Type': 'video/mp4', 'Cache-Control': 'no-store' });
  proc.stdout.pipe(res);
  res.on('close', () => proc.kill('SIGKILL'));
}
