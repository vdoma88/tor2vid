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
  };

  let current = null; // описание торрента с сервера
  let currentFile = null;
  let currentSource = null; // magnet для истории и «поделиться»
  let pollTimer = null;

  // ---------- утилиты ----------
  const fmtBytes = (n) => {
    if (!n) return '0 Б';
    const u = ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'];
    const i = Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
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
    els.video.replaceChildren();
    addSubtitles(f);
    els.video.src = src;
    els.video.play().catch(() => { /* автозапуск запрещён — нажмут сами */ });
    els.formatHint.hidden = f.browserFriendly;
    els.formatHint.textContent = f.browserFriendly ? '' :
      'Этот формат браузер может не показать (часто бывает с MKV в HEVC/AC3 и с AVI). Если видео не идёт или нет звука — откройте «Ссылку для VLC».';
    renderFiles();
    updateUrl();
    els.playerCard.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

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
    els.video.removeAttribute('src');
    els.video.load();
    current = null; currentFile = null; currentSource = null;
    els.playerCard.hidden = true;
    els.files.hidden = true;
    history.replaceState(null, '', location.pathname);
    setStatus('');
  });

  els.video.addEventListener('error', () => {
    if (!els.video.getAttribute('src')) return;
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

  renderRecent();
  fromHash();
})();
