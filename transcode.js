// Перекодирование через ffmpeg: всё, что браузер не играет сам (MKV, AVI,
// HEVC, AC3/DTS…), превращается на лету во фрагментированный MP4.
// Если видео уже H.264, оно только перепаковывается — это почти бесплатно.
import { spawn } from 'node:child_process';
import fs from 'node:fs';

let ffmpegPath = process.env.FFMPEG_PATH || null;
if (!ffmpegPath) {
  try {
    const mod = await import('ffmpeg-static');
    if (mod.default && fs.existsSync(mod.default)) ffmpegPath = mod.default;
  } catch { /* пакет не установлен */ }
}
if (!ffmpegPath) ffmpegPath = 'ffmpeg'; // системный, если есть

export const FFMPEG = ffmpegPath;

const MAX_HEIGHT = Number(process.env.TRANSCODE_MAX_HEIGHT) || 1080;
const PRESET = process.env.TRANSCODE_PRESET || 'veryfast';
const MAX_JOBS = Number(process.env.MAX_TRANSCODES) || 2;

// Что браузеры понимают внутри MP4.
const VIDEO_COPY = new Set(['h264']);
const AUDIO_COPY = new Set(['aac', 'mp3']);

const probeCache = new Map(); // ключ "hash:index" -> Promise<probe>
const jobs = new Map(); // ключ -> { proc, speed, started }

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
  const out = { duration: null, video: null, audio: [], subtitles: 0 };
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
        out.subtitles += 1;
        last = null;
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
    const proc = spawn(FFMPEG, ['-hide_banner', '-i', url], { stdio: ['ignore', 'ignore', 'pipe'] });
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
  for (const [k, job] of jobs) if (k.startsWith(infoHash + ':')) job.proc.kill('SIGKILL');
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
  while (jobs.size >= MAX_JOBS) {
    const [k, j] = [...jobs.entries()].sort((x, y) => x[1].started - y[1].started)[0];
    j.proc.kill('SIGKILL');
    jobs.delete(k);
  }

  const args = ['-hide_banner', '-loglevel', 'error', '-nostats', '-progress', 'pipe:2'];
  if (start > 0) args.push('-ss', String(start));
  args.push('-i', url);
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

  const proc = spawn(FFMPEG, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  const job = { proc, speed: null, started: Date.now() };
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
    if (jobs.get(key) === job) jobs.delete(key);
    if (code && !signal) console.error('[ffmpeg] exit', code, errTail.trim());
  });

  res.status(200).set({ 'Content-Type': 'video/mp4', 'Cache-Control': 'no-store' });
  proc.stdout.pipe(res);
  res.on('close', () => proc.kill('SIGKILL'));
}
