'use strict';

(() => {
  const TRACKERS = [
    'wss://tracker.btorrent.xyz', 'wss://tracker.openwebtorrent.com',
    'udp://tracker.opentrackr.org:1337/announce', 'udp://explodie.org:6969',
    'udp://tracker.leechers-paradise.org:6969',
  ].map((t) => '&tr=' + encodeURIComponent(t)).join('');
  const ws = (file) => '&ws=' + encodeURIComponent('https://webtorrent.io/torrents/') +
    '&xs=' + encodeURIComponent('https://webtorrent.io/torrents/' + file);
  const EXAMPLES = {
    sintel: 'magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=Sintel' + TRACKERS + ws('sintel.torrent'),
    tears: 'magnet:?xt=urn:btih:209c8226b299b308beaf2b9cd3fb49212dbd13ec&dn=Tears+of+Steel' + TRACKERS + ws('tears-of-steel.torrent'),
    bunny: 'magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&dn=Big+Buck+Bunny' + TRACKERS + ws('big-buck-bunny.torrent'),
  };

  const $ = (id) => document.getElementById(id);
  const els = {
    form: $('source-form'), input: $('magnet'), openBtn: $('open-btn'),
    drop: $('drop'), fileInput: $('file-input'), status: $('status'),
    playerCard: $('player-card'), video: $('video'), nowTitle: $('now-title'),
    stats: $('stats'), barFill: $('bar-fill'), formatHint: $('format-hint'),
    copyLink: $('copy-link'), downloadLink: $('download-link'), shareLink: $('share-link'),
    files: $('files'), torrentName: $('torrent-name'), fileList: $('file-list'),
    closeTorrent: $('close-torrent'), recent: $('recent'), recentList: $('recent-list'),
    stage: $('stage'), mode: $('mode'), getApp: $('get-app'),
    tc: $('tc'), tcPlay: $('tc-play'), tcTime: $('tc-time'), tcSeek: $('tc-seek'), tcDur: $('tc-dur'),
    tcAudio: $('tc-audio'), tcMute: $('tc-mute'), tcFull: $('tc-full'),
  };

  let config = { local: false, ffmpeg: false };
  // Режим перекодирования: позиция = offset + video.currentTime.
  let tc = null; // { duration, offset, audio, info }
  let seeking = false;

  let current = null; // описание торрента с сервера
  let currentFile = null;
  let currentSource = null; // magnet для истории и «поделиться»
  let pollTimer = null;

  // ---------- утилиты ----------
  const fmtBytes = (n) => {
    if (!n || n < 1) return '0 Б';
    const u = ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'];
    const i = Math.max(0, Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(1024))));
    return (n / 1024 ** i).toFixed(i ? 1 : 0).replace('.', ',') + ' ' + u[i];
  };
  const el = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  };
  const setStatus = (text, kind) => {
    els.status.textContent = text || '';
    els.status.dataset.kind = kind || '';
  };
  const fmtTime = (sec) => {
    sec = Math.max(0, Math.floor(sec || 0));
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    const s = String(sec % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
  };
  const icon = (btn, name, label) => {
    btn.firstElementChild.className = 'ico ico-' + name;
    btn.setAttribute('aria-label', label);
  };
  const storage = {
    get() { try { return JSON.parse(localStorage.getItem('tor2vid.recent') || '[]'); } catch { return []; } },
    set(v) { try { localStorage.setItem('tor2vid.recent', JSON.stringify(v.slice(0, 8))); } catch { /* приватный режим */ } },
  };

  // ---------- открытие торрента ----------
  async function open(body, isFile, sourceForHistory) {
    setStatus(isFile ? 'Читаю .torrent и ищу раздающих…' : 'Ищу раздающих и получаю список файлов… Обычно до 30 секунд.', 'wait');
    els.openBtn.disabled = true;
    try {
      const res = await fetch('/api/torrents', {
        method: 'POST',
        headers: { 'Content-Type': isFile ? 'application/x-bittorrent' : 'application/json' },
        body: isFile ? body : JSON.stringify({ magnet: body }),
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(data.error || 'Сервер не смог открыть торрент.');
      currentSource = sourceForHistory || null;
      showTorrent(data);
      setStatus('');
    } catch (err) {
      setStatus(err.message || 'Что-то пошло не так.', 'error');
    } finally {
      els.openBtn.disabled = false;
    }
  }

  function showTorrent(t) {
    current = t;
    els.torrentName.textContent = t.name;
    els.files.hidden = false;
    renderFiles();
    rememberRecent(t);
    const playable = t.files.filter((f) => f.playable);
    const best = playable.sort((a, b) => (b.browserFriendly - a.browserFriendly) || (b.length - a.length))[0];
    if (best) play(best.index);
    else setStatus('В торренте нет видео или аудио, которое можно открыть.', 'error');
    startPolling();
  }

  function renderFiles() {
    els.fileList.replaceChildren();
    const files = [...current.files].sort((a, b) => a.path.localeCompare(b.path, 'ru', { numeric: true }));
    for (const f of files) {
      if (f.subtitle) continue;
      const li = el('li');
      const btn = el('button', 'file' + (currentFile === f.index ? ' is-active' : ''));
      btn.type = 'button';
      btn.disabled = !f.playable;
      btn.append(el('span', 'file-name', f.path.split('/').slice(1).join('/') || f.name), el('span', 'file-size', fmtBytes(f.length)));
      if (f.playable) btn.addEventListener('click', () => play(f.index));
      li.append(btn);
      els.fileList.append(li);
    }
  }

  function play(index) {
    const f = current.files[index];
    currentFile = index;
    const src = `/stream/${current.infoHash}/${index}`;
    els.playerCard.hidden = false;
    els.nowTitle.textContent = f.name;
    els.downloadLink.href = src + '?download=1';
    els.formatHint.hidden = true;
    els.mode.hidden = true;
    renderFiles();
    updateUrl();
    els.playerCard.scrollIntoView({ behavior: 'smooth', block: 'start' });

    if (f.browserFriendly || !config.ffmpeg) {
      playDirect(f, src);
      if (!f.browserFriendly) {
        els.formatHint.hidden = false;
        els.formatHint.textContent = 'Этот формат браузер может не показать, а перекодирование здесь выключено. Если видео не идёт — откройте «Ссылку для VLC».';
      }
    } else {
      startTranscode(f);
    }
  }

  function resetVideo() {
    els.video.pause();
    els.video.removeAttribute('src');
    els.video.replaceChildren();
    els.video.load();
  }

  function playDirect(f, src) {
    tc = null;
    els.tc.hidden = true;
    els.video.controls = true;
    resetVideo();
    addSubtitles(f);
    els.video.src = src;
    els.video.play().catch(() => { /* автозапуск запрещён — нажмут сами */ });
  }

  // ---------- перекодирование ----------
  async function startTranscode(f) {
    const index = f.index;
    resetVideo();
    els.tc.hidden = true;
    setStatus('Смотрю, что внутри файла… Это может занять до минуты.', 'wait');
    let info;
    try {
      const res = await fetch(`/api/probe/${current.infoHash}/${index}`);
      info = await res.json();
      if (!res.ok) throw new Error(info.error || 'Не удалось прочитать файл.');
    } catch (err) {
      if (currentFile === index) setStatus(err.message, 'error');
      return;
    }
    if (currentFile !== index) return; // пока ждали, выбрали другой файл

    const defAudio = Math.max(0, info.audio.findIndex((a) => /^(ru|rus)$/i.test(a.lang || '')));
    tc = { duration: info.duration || 0, offset: 0, audio: defAudio, info };
    els.video.controls = false;
    els.tc.hidden = false;
    els.tcSeek.max = String(Math.max(1, Math.floor(tc.duration)));
    els.tcSeek.disabled = !tc.duration;
    els.tcDur.textContent = tc.duration ? fmtTime(tc.duration) : '—';
    renderAudio(info.audio);
    describeMode(info);
    addSubtitles(f);
    setStatus('');
    loadTranscode(0);
  }

  function loadTranscode(at) {
    if (!tc) return;
    tc.offset = Math.max(0, Math.min(at, tc.duration ? tc.duration - 1 : at));
    tc.offset = Math.floor(tc.offset);
    els.video.pause();
    els.video.src = `/transcode/${current.infoHash}/${currentFile}?t=${tc.offset}&a=${tc.audio}`;
    shiftCues();
    els.video.play().catch(() => {});
    updateTime();
  }

  function renderAudio(list) {
    els.tcAudio.replaceChildren();
    els.tcAudio.hidden = list.length < 2;
    list.forEach((a, i) => {
      const o = el('option', null, [a.title, a.lang && a.lang.toUpperCase(), a.codec.toUpperCase(), a.channels].filter(Boolean).join(' · '));
      o.value = String(i);
      if (i === tc.audio) o.selected = true;
      els.tcAudio.append(o);
    });
  }

  const CODEC = { h264: 'H.264', hevc: 'HEVC', h265: 'HEVC', mpeg4: 'MPEG-4', av1: 'AV1', vp9: 'VP9', ac3: 'AC3', eac3: 'E-AC3', dts: 'DTS', truehd: 'TrueHD', flac: 'FLAC', aac: 'AAC', mp3: 'MP3', opus: 'Opus', vorbis: 'Vorbis' };
  const codecName = (c) => CODEC[c] || (c || '').toUpperCase();

  function describeMode(info) {
    const parts = [];
    if (info.video) parts.push(info.copyVideo ? `видео ${codecName(info.video.codec)} без перекодирования` : `видео ${codecName(info.video.codec)}${info.video.tenBit ? ' 10 бит' : ''} → H.264`);
    const a = info.audio[tc.audio];
    if (a) parts.push(['aac', 'mp3'].includes(a.codec) ? `звук ${codecName(a.codec)} как есть` : `звук ${codecName(a.codec)} → AAC`);
    els.mode.hidden = false;
    els.mode.textContent = 'Показываем через MP4: ' + parts.join(', ') + '.';
    if (info.heavy && !info.local) {
      els.formatHint.hidden = false;
      els.formatHint.textContent = 'Видео перекодируется целиком — бесплатному серверу это тяжело, возможны паузы. Для таких файлов лучше скачать приложение и смотреть у себя (внизу страницы).';
    }
    if (info.subtitles) {
      els.mode.textContent += ` Встроенные субтитры (${info.subtitles}) пока не показываются.`;
    }
  }

  // Поток после перемотки начинается с нуля — сдвигаем субтитры на то же время.
  const cueOrig = new WeakMap();
  function shiftCues() {
    const offset = tc ? tc.offset : 0;
    for (const track of els.video.textTracks) {
      for (const cue of track.cues || []) {
        if (!cueOrig.has(cue)) cueOrig.set(cue, [cue.startTime, cue.endTime]);
        const [s0, e0] = cueOrig.get(cue);
        cue.startTime = s0 - offset;
        cue.endTime = e0 - offset;
      }
    }
  }

  function position() {
    return tc ? tc.offset + (els.video.currentTime || 0) : 0;
  }

  function updateTime() {
    if (!tc) return;
    const pos = position();
    els.tcTime.textContent = fmtTime(pos);
    if (!seeking) els.tcSeek.value = String(Math.floor(pos));
    els.tcSeek.style.setProperty('--p', String(tc.duration ? pos / tc.duration : 0));
  }

  els.video.addEventListener('timeupdate', updateTime);
  els.video.addEventListener('play', () => tc && icon(els.tcPlay, 'pause', 'Пауза'));
  els.video.addEventListener('pause', () => tc && icon(els.tcPlay, 'play', 'Смотреть'));
  els.video.addEventListener('click', () => { if (tc) togglePlay(); });

  function togglePlay() {
    if (els.video.paused) els.video.play().catch(() => {});
    else els.video.pause();
  }
  els.tcPlay.addEventListener('click', togglePlay);
  els.tcSeek.addEventListener('input', () => {
    seeking = true;
    els.tcTime.textContent = fmtTime(Number(els.tcSeek.value));
  });
  els.tcSeek.addEventListener('change', () => {
    seeking = false;
    loadTranscode(Number(els.tcSeek.value));
  });
  els.tcAudio.addEventListener('change', () => {
    tc.audio = Number(els.tcAudio.value);
    describeMode(tc.info);
    loadTranscode(position());
  });
  els.tcMute.addEventListener('click', () => {
    els.video.muted = !els.video.muted;
    icon(els.tcMute, els.video.muted ? 'mute' : 'sound', els.video.muted ? 'Включить звук' : 'Выключить звук');
  });
  els.tcFull.addEventListener('click', () => {
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    else if (els.stage.requestFullscreen) els.stage.requestFullscreen().catch(() => {});
    else if (els.video.webkitEnterFullscreen) els.video.webkitEnterFullscreen();
  });
  document.addEventListener('keydown', (e) => {
    if (!tc || /^(INPUT|SELECT|TEXTAREA|BUTTON)$/.test(e.target.tagName)) return;
    if (e.key === ' ' || e.key === 'k') { e.preventDefault(); togglePlay(); }
    if (e.key === 'ArrowRight') loadTranscode(position() + 10);
    if (e.key === 'ArrowLeft') loadTranscode(position() - 10);
    if (e.key === 'f') els.tcFull.click();
  });

  // Субтитры из торрента: сначала те, что названы как видео.
  function addSubtitles(videoFile) {
    const base = videoFile.name.replace(/\.[^.]+$/, '').toLowerCase();
    const subs = current.files.filter((f) => f.subtitle).sort((a, b) => {
      const sa = a.name.toLowerCase().startsWith(base) ? 0 : 1;
      const sb = b.name.toLowerCase().startsWith(base) ? 0 : 1;
      return sa - sb;
    });
    subs.slice(0, 20).forEach((s, i) => {
      const track = document.createElement('track');
      track.kind = 'subtitles';
      track.src = `/subs/${current.infoHash}/${s.index}`;
      const lang = (s.name.match(/[._-]([a-z]{2,3})\.(srt|vtt)$/i) || [])[1];
      track.label = lang ? lang.toUpperCase() : s.name;
      if (lang) track.srclang = lang.toLowerCase();
      if (i === 0 && /^(ru|rus)$/i.test(lang || '')) track.default = true;
      track.addEventListener('load', shiftCues);
      els.video.append(track);
    });
  }

  // ---------- состояние раздачи ----------
  function startPolling() {
    clearInterval(pollTimer);
    pollTimer = setInterval(poll, 2000);
    poll();
  }

  async function poll() {
    if (!current) return;
    try {
      const res = await fetch('/api/torrents/' + current.infoHash);
      if (res.status === 404) {
        clearInterval(pollTimer);
        setStatus('Сервер закрыл раздачу (долго не смотрели или перезапуск). Откройте ссылку заново.', 'error');
        return;
      }
      const t = await res.json();
      const f = currentFile != null ? t.files[currentFile] : null;
      const parts = [
        `Пиров: ${t.numPeers}`,
        `↓ ${fmtBytes(t.downloadSpeed)}/с`,
      ];
      if (f) parts.push(`Файл: ${Math.round(f.progress * 100)}% из ${fmtBytes(f.length)}`);
      if (f && tc && f.transcodeSpeed != null) {
        parts.push(`Перекодирование: ${String(f.transcodeSpeed).replace('.', ',')}×${f.transcodeSpeed < 1 ? ' — не успевает' : ''}`);
      }
      els.stats.textContent = parts.join('  ·  ');
      els.barFill.style.setProperty('--p', String(f ? f.progress : t.progress));
    } catch { /* сеть моргнула — следующий опрос */ }
  }

  // ---------- кнопки ----------
  els.copyLink.addEventListener('click', async () => {
    const url = new URL(`/stream/${current.infoHash}/${currentFile}`, location.href).href;
    try {
      await navigator.clipboard.writeText(url);
      setStatus('Ссылка скопирована. В VLC: Медиа → Открыть URL.', 'ok');
    } catch {
      window.prompt('Скопируйте ссылку и откройте в VLC (Медиа → Открыть URL):', url);
    }
  });

  els.shareLink.addEventListener('click', async () => {
    const url = location.href;
    try {
      if (navigator.share) await navigator.share({ title: current.name, url });
      else { await navigator.clipboard.writeText(url); setStatus('Ссылка на просмотр скопирована.', 'ok'); }
    } catch { /* отменили */ }
  });

  els.closeTorrent.addEventListener('click', () => {
    if (current) fetch('/api/torrents/' + current.infoHash, { method: 'DELETE' }).catch(() => {});
    clearInterval(pollTimer);
    tc = null;
    els.tc.hidden = true;
    resetVideo();
    current = null; currentFile = null; currentSource = null;
    els.playerCard.hidden = true;
    els.files.hidden = true;
    history.replaceState(null, '', location.pathname);
    setStatus('');
  });

  els.video.addEventListener('error', () => {
    if (!els.video.getAttribute('src')) return;
    const f = current && current.files[currentFile];
    // Браузер не справился с «удобным» форматом (например, MP4 с HEVC) — пробуем через ffmpeg.
    if (!tc && f && config.ffmpeg) {
      setStatus('Браузер не понял формат — включаю перекодирование…', 'wait');
      startTranscode(f);
      return;
    }
    setStatus('Браузер не смог воспроизвести этот файл. Попробуйте «Ссылку для VLC» или другой файл из списка.', 'error');
  });
  els.video.addEventListener('waiting', () => setStatus('Подгружаю…', 'wait'));
  els.video.addEventListener('playing', () => setStatus(''));

  // ---------- ввод ----------
  els.form.addEventListener('submit', (e) => {
    e.preventDefault();
    const v = els.input.value.trim();
    if (!v) { els.input.focus(); return; }
    open(v, false, v);
  });

  document.querySelectorAll('[data-example]').forEach((b) => b.addEventListener('click', () => {
    const m = EXAMPLES[b.dataset.example];
    els.input.value = m;
    open(m, false, m);
  }));

  async function openFile(file) {
    if (!file) return;
    if (!/\.torrent$/i.test(file.name) && file.type !== 'application/x-bittorrent') {
      setStatus('Это не .torrent файл.', 'error');
      return;
    }
    open(await file.arrayBuffer(), true, null);
  }
  els.fileInput.addEventListener('change', () => openFile(els.fileInput.files[0]));
  ['dragenter', 'dragover'].forEach((ev) => document.addEventListener(ev, (e) => {
    e.preventDefault();
    els.drop.classList.add('is-over');
  }));
  ['dragleave', 'drop'].forEach((ev) => document.addEventListener(ev, (e) => {
    e.preventDefault();
    if (ev === 'drop' || e.target === document.documentElement) els.drop.classList.remove('is-over');
  }));
  document.addEventListener('drop', (e) => openFile(e.dataTransfer.files[0]));

  // ---------- адрес страницы и история ----------
  function updateUrl() {
    const p = new URLSearchParams();
    p.set('t', currentSource || current.infoHash);
    p.set('f', String(currentFile));
    history.replaceState(null, '', '#' + p.toString());
  }

  function rememberRecent(t) {
    const source = currentSource || t.infoHash;
    const list = storage.get().filter((r) => r.infoHash !== t.infoHash);
    list.unshift({ infoHash: t.infoHash, name: t.name, source });
    storage.set(list);
    renderRecent();
  }

  function renderRecent() {
    const list = storage.get();
    els.recent.hidden = !list.length;
    els.recentList.replaceChildren();
    for (const r of list) {
      const li = el('li');
      const btn = el('button', 'link', r.name);
      btn.type = 'button';
      btn.addEventListener('click', () => open(r.source, false, r.source));
      li.append(btn);
      els.recentList.append(li);
    }
  }

  async function fromHash() {
    const p = new URLSearchParams(location.hash.slice(1));
    const src = p.get('t');
    if (!src) return;
    els.input.value = src;
    await open(src, false, src);
    const f = Number(p.get('f'));
    if (current && Number.isInteger(f) && current.files[f] && f !== currentFile) play(f);
  }

  async function init() {
    try { config = await (await fetch('/api/config')).json(); } catch { /* по умолчанию */ }
    els.getApp.hidden = Boolean(config.local);
    renderRecent();
    fromHash();
  }
  init();
})();
