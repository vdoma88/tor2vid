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
    files: $('files'), torrentName: $('torrent-name'), fileList: $('file-list'),
    closeTorrent: $('close-torrent'), recent: $('recent'), recentList: $('recent-list'),
    stage: $('stage'), mode: $('mode'), getApp: $('get-app'),
    tc: $('tc'), tcPlay: $('tc-play'), tcTime: $('tc-time'), tcSeek: $('tc-seek'), tcDur: $('tc-dur'),
    tcAudio: $('tc-audio'), tcSubs: $('tc-subs'), tcMute: $('tc-mute'), tcFull: $('tc-full'),
    viewPlayer: $('view-player'), viewCatalog: $('view-catalog'), catalogStatus: $('catalog-status'),
    catalogNote: $('catalog-note'),
  };

  let config = { local: false, ffmpeg: false };
  // Режим перекодирования: позиция = offset + video.currentTime.
  let tc = null; // { duration, offset, audio, info }
  let seeking = false;
  let extTracks = []; // внешние .srt из торрента: { label, lang, track }
  let embTrack = null; // одна дорожка под встроенные субтитры, создаётся один раз
  let emb = null; // { n, from, timer } — какие встроенные субтитры показываем

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
    els.formatHint.hidden = true;
    els.mode.hidden = true;
    renderFiles();
    els.playerCard.scrollIntoView({ behavior: 'smooth', block: 'start' });

    if (f.browserFriendly || !config.ffmpeg) {
      playDirect(f, src);
      if (!f.browserFriendly) {
        els.formatHint.hidden = false;
        els.formatHint.textContent = 'Этот формат браузер может не показать, а перекодирование здесь выключено. Если видео не идёт — выберите другой файл.';
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
    stopEmbedded();
    emb = null;
    if (embTrack) embTrack.mode = 'disabled';
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
    stopEmbedded();
    emb = null;
    if (embTrack) embTrack.mode = 'disabled';
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
    renderSubs(info);
    setStatus('');
    loadTranscode(0);
  }

  let loadSeq = 0;
  async function loadTranscode(at) {
    if (!tc) return;
    const seq = ++loadSeq;
    const want = Math.floor(Math.max(0, Math.min(at, tc.duration ? tc.duration - 1 : at)));
    let start = want;
    // При перепаковке поток начнётся с ключевого кадра — узнаём, с какого.
    if (want > 0 && tc.info.copyVideo) {
      try {
        const r = await (await fetch(`/api/start/${current.infoHash}/${currentFile}?t=${want}`)).json();
        if (typeof r.start === 'number') start = r.start;
      } catch { /* возьмём как есть */ }
    }
    if (seq !== loadSeq || !tc) return; // пока ждали, перемотали ещё раз
    tc.offset = start;
    els.video.pause();
    els.video.src = `/transcode/${current.infoHash}/${currentFile}?t=${start}&a=${tc.audio}`;
    shiftCues();
    resetEmbedded();
    els.video.play().catch(() => {});
    updateTime();
  }

  // ---------- субтитры в режиме перекодирования ----------
  const subLabel = (x) => [x.title, x.lang && x.lang.toUpperCase(), x.forced && 'только надписи'].filter(Boolean).join(' · ') || `Дорожка ${x.index + 1}`;

  function renderSubs(info) {
    els.tcSubs.replaceChildren();
    const add = (value, text, disabled) => {
      const o = el('option', null, text);
      o.value = value;
      o.disabled = Boolean(disabled);
      els.tcSubs.append(o);
    };
    add('off', 'Без субтитров');
    info.subtitles.forEach((x) => add(`emb:${x.index}`, x.text ? subLabel(x) : `${subLabel(x)} — картинками, не показываются`, !x.text));
    extTracks.forEach((x, i) => add(`ext:${i}`, `${x.label} · файл`));
    els.tcSubs.hidden = els.tcSubs.options.length < 2;

    // Звук не русский, а русские субтитры есть — включаем их. Иначе — только «надписи» под язык звука.
    const audio = info.audio[tc.audio];
    const audioRu = audio && /^(ru|rus)$/i.test(audio.lang || '');
    const ru = (lang) => /^(ru|rus)$/i.test(lang || '');
    let pick = 'off';
    const embRu = info.subtitles.find((x) => x.text && ru(x.lang) && !x.forced);
    const extRu = extTracks.findIndex((x) => ru(x.lang));
    const forced = info.subtitles.find((x) => x.text && x.forced && (!audio || !audio.lang || x.lang === audio.lang));
    if (!audioRu && embRu) pick = `emb:${embRu.index}`;
    else if (!audioRu && extRu >= 0) pick = `ext:${extRu}`;
    else if (forced) pick = `emb:${forced.index}`;
    els.tcSubs.value = pick;
    selectSubs(pick);
  }

  function selectSubs(value) {
    for (const x of extTracks) x.track.mode = 'disabled';
    if (embTrack) embTrack.mode = 'disabled';
    stopEmbedded();
    emb = null;
    if (value.startsWith('ext:')) {
      const x = extTracks[Number(value.slice(4))];
      if (x) x.track.mode = 'showing';
    } else if (value.startsWith('emb:')) {
      if (!embTrack) embTrack = els.video.addTextTrack('subtitles', 'Встроенные', 'ru');
      embTrack.mode = 'showing';
      emb = { n: Number(value.slice(4)), from: 0, timer: null };
      resetEmbedded();
    }
  }

  function clearEmbCues() {
    if (!embTrack || !embTrack.cues) return;
    for (const cue of [...embTrack.cues]) embTrack.removeCue(cue);
    // Chrome оставляет на экране реплику, удалённую во время показа, — перерисовываем.
    if (embTrack.mode === 'showing') {
      embTrack.mode = 'hidden';
      embTrack.mode = 'showing';
    }
  }

  function stopEmbedded() {
    if (emb && emb.timer) clearTimeout(emb.timer);
    clearEmbCues();
  }

  function resetEmbedded() {
    if (!emb) return;
    stopEmbedded();
    emb.from = 0;
    emb.timer = setTimeout(pullEmbedded, 1000);
  }

  // Подтягиваем реплики, которые ffmpeg успел вытащить вместе с видео.
  async function pullEmbedded() {
    const my = emb;
    if (!my || !tc || !current) return;
    let done = false;
    try {
      const r = await (await fetch(`/api/live-subs/${current.infoHash}/${currentFile}/${my.n}?from=${my.from}`)).json();
      if (emb !== my) return;
      // Ответ от старого потока (до перемотки) — пропускаем.
      if (r.start != null && Math.abs(r.start - tc.offset) < 0.01) {
        for (const c of r.cues) embTrack.addCue(new VTTCue(c.s, c.e, c.text));
        my.from = r.total;
        done = r.done;
      }
    } catch { /* следующая попытка */ }
    if (emb === my && !done) my.timer = setTimeout(pullEmbedded, 2000);
  }

  els.tcSubs.addEventListener('change', () => selectSubs(els.tcSubs.value));

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
    const pics = info.subtitles.filter((x) => !x.text).length;
    if (pics) {
      els.mode.textContent += ` Субтитры картинками (PGS/VobSub, ${pics}) браузер показать не может.`;
    }
  }

  // Поток после перемотки начинается с нуля — сдвигаем субтитры на то же время.
  const cueOrig = new WeakMap();
  function shiftCues() {
    const offset = tc ? tc.offset : 0;
    for (const { track } of extTracks) {
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
    extTracks = [];
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
      extTracks.push({ label: track.label, lang: (lang || '').toLowerCase(), track: track.track });
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

  // Только просмотр: без пункта «Сохранить видео» в меню плеера.
  els.video.controlsList.add('nodownload');
  els.video.addEventListener('contextmenu', (e) => e.preventDefault());

  els.closeTorrent.addEventListener('click', () => {
    if (current) fetch('/api/torrents/' + current.infoHash, { method: 'DELETE' }).catch(() => {});
    clearInterval(pollTimer);
    tc = null;
    stopEmbedded();
    emb = null;
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
    setStatus('Браузер не смог воспроизвести этот файл. Попробуйте другой файл из списка.', 'error');
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

  // ---------- история (только в этом браузере) ----------

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

  // ---------- вкладки ----------
  let catalogLoaded = false;

  function showView(name) {
    const isCatalog = name === 'catalog';
    els.viewPlayer.hidden = isCatalog;
    els.viewCatalog.hidden = !isCatalog;
    document.querySelectorAll('.tab').forEach((t) => {
      if (t.dataset.tab === name) t.setAttribute('aria-current', 'page');
      else t.removeAttribute('aria-current');
    });
    // В адрес страницы ссылка на торрент не попадает — им не поделишься случайно.
    history.replaceState(null, '', isCatalog ? '#catalog' : location.pathname);
    if (isCatalog && !catalogLoaded) loadCatalog();
    window.scrollTo({ top: 0 });
  }

  document.querySelectorAll('.tab').forEach((t) => t.addEventListener('click', (e) => {
    e.preventDefault();
    showView(t.dataset.tab);
  }));

  // ---------- каталог ----------
  async function loadCatalog() {
    els.catalogStatus.textContent = 'Загружаю список…';
    els.catalogStatus.dataset.kind = 'wait';
    try {
      const res = await fetch('/api/catalog');
      if (!res.ok) throw new Error();
      const c = await res.json();
      renderShelf('films', c.films || []);
      renderShelf('cartoons', c.cartoons || []);
      catalogLoaded = true;
      els.catalogStatus.textContent = (c.films || []).length || (c.cartoons || []).length ? '' : 'Список пока пуст — сервер ещё собирает его. Загляните через пару минут.';
      els.catalogStatus.dataset.kind = '';
      const date = c.updatedAt ? new Date(c.updatedAt).toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' }) : null;
      els.catalogNote.textContent = `Источник — Internet Archive (archive.org): фильмы до ${c.maxYear || 1950} года с отметкой «общественное достояние», по числу просмотров.` +
        (date ? ` Обновлено ${date}.` : '') + ' Отметку ставит тот, кто загрузил фильм, поэтому в списке только старое кино, а копии с сомнительными отметками отсеяны.';
    } catch {
      els.catalogStatus.textContent = 'Не удалось загрузить каталог. Попробуйте обновить страницу.';
      els.catalogStatus.dataset.kind = 'error';
    }
  }

  function renderShelf(name, items) {
    const box = $(`catalog-${name}`);
    const list = $(`${name}-list`);
    box.hidden = !items.length;
    $(`${name}-count`).textContent = items.length ? String(items.length) : '';
    list.replaceChildren();
    for (const item of items) {
      const li = el('li');
      const card = el('button', 'card');
      card.type = 'button';
      const cover = el('span', 'card-cover');
      const img = document.createElement('img');
      img.src = `/cover/${encodeURIComponent(item.id)}`;
      img.alt = '';
      img.loading = 'lazy';
      img.decoding = 'async';
      img.addEventListener('error', () => img.remove());
      cover.append(img);
      if (item.silent) cover.append(el('span', 'card-badge', 'Немое'));
      const body = el('span', 'card-body');
      // «1929: Arsenal with score…», «Aelita: Reina de Marte 1924 (Subtitulada…)» → «Arsenal», «Aelita».
      const orig = item.title.replace(/^\d{4}:\s*/, '').split(/\s*[([:]|\s+with\s+(?:score|english|subtitles)\b|\s+-\s+/i)[0].replace(/\s+(19|20)\d\d$/, '').trim() || item.title;
      const name = item.titleRu || orig;
      body.append(el('span', 'card-title', name));
      if (item.titleRu && orig.toLowerCase() !== name.toLowerCase()) body.append(el('span', 'card-orig', orig));
      body.append(el('span', 'card-meta', [item.year, item.minutes && `${item.minutes} мин`].filter(Boolean).join(' · ')));
      card.append(cover, body);
      card.setAttribute('aria-label', `${item.titleRu || item.title}, ${item.year || ''} — смотреть`);
      card.addEventListener('click', () => {
        showView('player');
        els.input.value = item.torrent;
        open(item.torrent, false, item.torrent);
      });
      li.append(card);
      list.append(li);
    }
  }

  async function init() {
    try { config = await (await fetch('/api/config')).json(); } catch { /* по умолчанию */ }
    els.getApp.hidden = Boolean(config.local);
    renderRecent();
    showView(location.hash === '#catalog' ? 'catalog' : 'player');
  }
  init();
})();
