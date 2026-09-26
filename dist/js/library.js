import { $, $$, escapeHTML } from './dom.js';
import { t, locMsg, onLangChange } from './i18n.js';
import { state, TMDB_IMG } from './state.js';
import { tmdb, downloadList, downloadStart, downloadStartGroup, rdResolveAll, torrentParseFile, downloadAddLocal } from './api.js';
import { openDetails } from './details.js';
import { listResume } from './resume.js';
import { listFavorites } from './favorites.js';
import { fmtRelative } from './format.js';
import { loadDownloads, startDownloadsPolling, stopDownloadsPolling } from './grid.js';
import { userStore } from './userstore.js';

const grid = $('#grid');
const cardTpl = $('#cardTpl');
const libraryBar = $('#libraryBar');
const libraryBadge = $('#libraryBadge');
const downloadBadge = $('#downloadBadge');

const metaCache = new Map();
const cardItems = new Map();
let gen = 0;

const SEEN_KEY = 'siiis:dl:seen';
const BADGE_INTERVAL_MS = 3000;
let badgePollTimer = 0;
let lastDownloadList = [];

function readSeen() {
  try { return new Set(JSON.parse(userStore.getItem(SEEN_KEY) || '[]')); }
  catch { return new Set(); }
}
function writeSeen(set) {
  userStore.setItem(SEEN_KEY, JSON.stringify([...set]));
}

function isDownloadDone(e) {
  const s = e?.stats;
  if (!s || s.error) return false;
  if (s.length > 0 && s.downloaded >= s.length) return true;
  return typeof s.progress === 'number' && s.progress >= 0.999;
}

function applyDownloadBadges() {
  const seen = readSeen();
  const inDownloadView = state.section === 'library' && state.libraryTab === 'download';
  const currentIds = new Set();
  let active = 0;
  let hasNew = false;
  let seenChanged = false;

  for (const e of lastDownloadList) {
    currentIds.add(e.id);
    if (isDownloadDone(e)) {
      if (!seen.has(e.id)) {
        if (inDownloadView) { seen.add(e.id); seenChanged = true; }
        else hasNew = true;
      }
    } else if (!e?.stats?.error) {
      active++;
    }
  }

  for (const id of [...seen]) {
    if (!currentIds.has(id)) { seen.delete(id); seenChanged = true; }
  }
  if (seenChanged) writeSeen(seen);

  // Polled every few seconds: the page is only touched when a badge changes.
  if (downloadBadge) {
    if (downloadBadge.textContent !== String(active)) downloadBadge.textContent = String(active);
    if (downloadBadge.hidden !== (active === 0)) downloadBadge.hidden = active === 0;
  }
  const libraryHidden = !hasNew && active === 0;
  if (libraryBadge && libraryBadge.hidden !== libraryHidden) libraryBadge.hidden = libraryHidden;
}

async function refreshBadges() {
  try { lastDownloadList = await downloadList(); }
  catch { lastDownloadList = []; }
  applyDownloadBadges();
}

export function startDownloadBadgePolling() {
  if (badgePollTimer) return;
  refreshBadges();
  badgePollTimer = setInterval(refreshBadges, BADGE_INTERVAL_MS);
}

const dlMagnetBar = $('#dlMagnetBar');
const dlMagnetHint = $('#dlMagnetHint');
const dlMagnetTorrentInput = $('#dlMagnetTorrent');
const dlMagnetTorrentBtn = $('#dlMagnetTorrentBtn');
const dlMagnetRdInput = $('#dlMagnetRd');
const dlMagnetRdBtn = $('#dlMagnetRdBtn');
const dlMagnetCancelBtn = $('#dlMagnetCancel');

let dlMagnetHintTimer = 0;
let dlMagnetAbort = null;

function beginDlMagnetCancellable() {
  const ctl = new AbortController();
  dlMagnetAbort = ctl;
  if (dlMagnetCancelBtn) dlMagnetCancelBtn.hidden = false;
  return ctl;
}

function endDlMagnetCancellable(ctl) {
  if (dlMagnetAbort === ctl) dlMagnetAbort = null;
  if (dlMagnetCancelBtn) dlMagnetCancelBtn.hidden = true;
}

dlMagnetCancelBtn?.addEventListener('click', () => {
  dlMagnetAbort?.abort();
});

function setDlMagnetHint(msg, kind) {
  if (!dlMagnetHint) return;
  if (dlMagnetHintTimer) { clearTimeout(dlMagnetHintTimer); dlMagnetHintTimer = 0; }
  dlMagnetHint.textContent = msg || '';
  dlMagnetHint.className = 'dl-magnet-hint' + (kind ? ' ' + kind : '');
  if (!msg) return;
  const delay = kind === 'success' ? 3500 : kind === 'error' ? 6000 : 0;
  if (delay > 0) {
    dlMagnetHintTimer = setTimeout(() => {
      dlMagnetHintTimer = 0;
      dlMagnetHint.textContent = '';
      dlMagnetHint.className = 'dl-magnet-hint';
    }, delay);
  }
}

function parseMagnet(raw) {
  const magnet = (raw || '').trim();
  if (!magnet.toLowerCase().startsWith('magnet:?')) return null;
  let params;
  try { params = new URLSearchParams(magnet.slice('magnet:?'.length)); }
  catch { return null; }
  const xt = params.getAll('xt').find(v => v.toLowerCase().startsWith('urn:btih:'));
  if (!xt) return null;
  const rawHash = xt.slice('urn:btih:'.length);
  if (!/^[a-fA-F0-9]{40}$/.test(rawHash)) return null;
  return {
    infoHash: rawHash.toLowerCase(),
    displayName: params.get('dn') || '',
    sources: params.getAll('tr').filter(Boolean),
  };
}

async function startTorrentDownloadFromMagnet() {
  const parsed = parseMagnet(dlMagnetTorrentInput.value);
  if (!parsed) {
    setDlMagnetHint(t('library.magnetInvalid'), 'error');
    return;
  }
  dlMagnetTorrentBtn.disabled = true;
  setDlMagnetHint(t('library.torrentStarting'));
  try {
    await downloadStart({
      infoHash: parsed.infoHash,
      title: parsed.displayName || parsed.infoHash,
      sources: parsed.sources,
    });
    dlMagnetTorrentInput.value = '';
    setDlMagnetHint(t('library.downloadStarted', { title: parsed.displayName || parsed.infoHash }), 'success');
    loadDownloads();
  } catch (e) {
    setDlMagnetHint(t('library.errorPrefix', { error: locMsg((e && e.message) || (typeof e === 'string' ? e : String(e))) }), 'error');
  } finally {
    dlMagnetTorrentBtn.disabled = false;
  }
}

async function startRdDownloadFromMagnet() {
  const parsed = parseMagnet(dlMagnetRdInput.value);
  if (!parsed) {
    setDlMagnetHint(t('library.magnetInvalid'), 'error');
    return;
  }
  if (!state.settings.debridConfigured) {
    setDlMagnetHint(t('library.debridNotConfigured'), 'error');
    return;
  }
  dlMagnetRdBtn.disabled = true;
  const ctl = beginDlMagnetCancellable();
  const provLabel = state.settings.debridProvider === 'ad' ? 'AllDebrid' : 'Real-Debrid';
  setDlMagnetHint(t('library.debridResolvingMagnet', { provider: provLabel }));
  try {
    const resolved = await rdResolveAll({
      infoHash: parsed.infoHash,
      displayName: parsed.displayName,
      sources: parsed.sources,
    }, { signal: ctl.signal });
    if (ctl.signal.aborted) {
      setDlMagnetHint(t('library.debridCancelled'), 'success');
      return;
    }
    const files = (resolved || []).filter(f => f && f.url);
    if (!files.length) throw new Error(t('library.debridNoFiles'));
    const title = parsed.displayName || parsed.infoHash;
    setDlMagnetHint(t('library.downloadingNFiles', { n: files.length }));
    await downloadStartGroup({
      title,
      infoHash: parsed.infoHash,
      files: files.map(f => ({ url: f.url, filename: f.filename })),
    });
    dlMagnetRdInput.value = '';
    setDlMagnetHint(t('library.downloadStartedNFiles', { title, n: files.length }), 'success');
    loadDownloads();
  } catch (e) {
    if (e?.name === 'AbortError' || ctl.signal.aborted) {
      setDlMagnetHint(t('library.debridCancelled'), 'success');
      return;
    }
    setDlMagnetHint(t('library.errorPrefix', { error: locMsg((e && e.message) || (typeof e === 'string' ? e : String(e))) }), 'error');
  } finally {
    endDlMagnetCancellable(ctl);
    dlMagnetRdBtn.disabled = false;
  }
}

if (dlMagnetTorrentBtn) dlMagnetTorrentBtn.addEventListener('click', startTorrentDownloadFromMagnet);
if (dlMagnetRdBtn) dlMagnetRdBtn.addEventListener('click', startRdDownloadFromMagnet);
if (dlMagnetTorrentInput) {
  dlMagnetTorrentInput.addEventListener('keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); startTorrentDownloadFromMagnet(); }
  });
}
if (dlMagnetRdInput) {
  dlMagnetRdInput.addEventListener('keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); startRdDownloadFromMagnet(); }
  });
}

async function handleTorrentFile(file, mode) {
  if (!file) return;
  const btn = mode === 'rd' ? $('#dlMagnetRdClip') : $('#dlMagnetTorrentClip');
  if (mode === 'rd' && !state.settings.debridConfigured) {
    setDlMagnetHint(t('library.debridNotConfigured'), 'error');
    return;
  }
  if (btn) btn.disabled = true;
  const ctl = mode === 'rd' ? beginDlMagnetCancellable() : null;
  setDlMagnetHint(t('library.readingFile', { name: file.name }));
  try {
    const buf = new Uint8Array(await file.arrayBuffer());
    const parsed = await torrentParseFile(buf);
    if (!parsed?.infoHash) throw new Error(t('library.torrentNoInfoHash'));
    if (mode === 'rd') {
      const provLabel = state.settings.debridProvider === 'ad' ? 'AllDebrid' : 'Real-Debrid';
      setDlMagnetHint(t('library.debridResolvingTorrent', { provider: provLabel }));
      const resolved = await rdResolveAll({
        infoHash: parsed.infoHash,
        displayName: parsed.name,
        sources: parsed.trackers,
      }, { signal: ctl.signal });
      if (ctl.signal.aborted) {
        setDlMagnetHint(t('library.debridCancelled'), 'success');
        return;
      }
      const files = (resolved || []).filter(f => f && f.url);
      if (!files.length) throw new Error(t('library.debridNoFiles'));
      setDlMagnetHint(t('library.downloadingNFiles', { n: files.length }));
      await downloadStartGroup({
        title: parsed.name || parsed.infoHash,
        infoHash: parsed.infoHash,
        files: files.map(f => ({ url: f.url, filename: f.filename })),
      });
    } else {
      setDlMagnetHint(t('library.torrentStarting'));
      await downloadStart({
        infoHash: parsed.infoHash,
        title: parsed.name || parsed.infoHash,
        sources: parsed.trackers,
      });
    }
    setDlMagnetHint(t('library.downloadStarted', { title: parsed.name || parsed.infoHash }), 'success');
    loadDownloads();
  } catch (e) {
    if (ctl && (e?.name === 'AbortError' || ctl.signal.aborted)) {
      setDlMagnetHint(t('library.debridCancelled'), 'success');
      return;
    }
    setDlMagnetHint(t('library.errorPrefix', { error: locMsg((e && e.message) || (typeof e === 'string' ? e : String(e))) }), 'error');
  } finally {
    if (ctl) endDlMagnetCancellable(ctl);
    if (btn) btn.disabled = false;
  }
}

const dlMagnetTorrentClip = $('#dlMagnetTorrentClip');
const dlMagnetTorrentFile = $('#dlMagnetTorrentFile');
const dlMagnetRdClip = $('#dlMagnetRdClip');
const dlMagnetRdFile = $('#dlMagnetRdFile');

if (dlMagnetTorrentClip && dlMagnetTorrentFile) {
  dlMagnetTorrentClip.addEventListener('click', () => dlMagnetTorrentFile.click());
  dlMagnetTorrentFile.addEventListener('change', async e => {
    const f = e.target.files?.[0];
    e.target.value = '';
    if (f) await handleTorrentFile(f, 'p2p');
  });
}
if (dlMagnetRdClip && dlMagnetRdFile) {
  dlMagnetRdClip.addEventListener('click', () => dlMagnetRdFile.click());
  dlMagnetRdFile.addEventListener('change', async e => {
    const f = e.target.files?.[0];
    e.target.value = '';
    if (f) await handleTorrentFile(f, 'rd');
  });
}

// --- Drag & drop di file audio/video locali nella sezione Download ---
const TAURI = window.__TAURI__;
const dlDropOverlay = $('#dlDropOverlay');
const LOCAL_MEDIA_RE = /\.(mp4|mkv|avi|mov|webm|m4v|wmv|flv|ts|m2ts|mts|mpg|mpeg|3gp|ogv|vob|divx|mp3|flac|m4a|aac|ogg|opus|wav|wma|aiff|aif|mka|alac)$/i;

function inDownloadView() {
  return state.section === 'library' && state.libraryTab === 'download';
}

function dropPaths(payload) {
  if (Array.isArray(payload)) return payload;
  if (payload && Array.isArray(payload.paths)) return payload.paths;
  return [];
}

function showDropOverlay(show) {
  if (dlDropOverlay) dlDropOverlay.hidden = !show;
}

async function handleLocalDrop(paths) {
  const media = paths.filter(p => typeof p === 'string' && LOCAL_MEDIA_RE.test(p));
  if (!media.length) {
    setDlMagnetHint(t('dl.dropUnsupported'), 'error');
    return;
  }
  let added = 0;
  for (const p of media) {
    try { await downloadAddLocal(p); added++; }
    catch (e) { console.warn('downloadAddLocal failed', p, e); }
  }
  if (added) {
    setDlMagnetHint(t('dl.localAdded', { n: added }), 'success');
    loadDownloads();
  }
}

if (TAURI?.event?.listen) {
  TAURI.event.listen('tauri://drag-enter', () => {
    if (inDownloadView()) showDropOverlay(true);
  }).catch(() => {});
  TAURI.event.listen('tauri://drag-leave', () => showDropOverlay(false)).catch(() => {});
  TAURI.event.listen('tauri://drag-drop', e => {
    showDropOverlay(false);
    if (!inDownloadView()) return;
    handleLocalDrop(dropPaths(e.payload));
  }).catch(() => {});
}

function updateDlMagnetBarVisibility() {
  if (!dlMagnetBar) return;
  const visible = state.section === 'library' && state.libraryTab === 'download';
  dlMagnetBar.hidden = !visible;
  if (!visible) setDlMagnetHint('');
  updateDlMagnetDebridSub();
}

function updateDlMagnetDebridSub() {
  const sub = document.getElementById('dlMagnetDebridSub');
  if (!sub) return;
  const provider = state.settings.debridProvider || '';
  const ok = !!state.settings.debridConfigured;
  if (provider === 'rd') sub.textContent = ok ? t('library.debridSubReady', { provider: 'Real-Debrid' }) : t('library.debridSubNoToken', { provider: 'Real-Debrid' });
  else if (provider === 'ad') sub.textContent = ok ? t('library.debridSubReady', { provider: 'AllDebrid' }) : t('library.debridSubNoToken', { provider: 'AllDebrid' });
  else sub.textContent = t('library.debridSubConfigure');
}

onLangChange(updateDlMagnetDebridSub);

function setActiveTab(tab) {
  state.libraryTab = tab;
  $$('[data-library-tab]', libraryBar).forEach(b => {
    const active = b.dataset.libraryTab === tab;
    b.classList.toggle('is-active', active);
    b.setAttribute('aria-selected', active);
  });
  applyDownloadBadges();
  updateDlMagnetBarVisibility();
}

async function loadMeta(entry) {
  const key = `${entry.type}:${entry.tmdbId}`;
  let meta = metaCache.get(key);
  if (meta === undefined) {
    if (!state.settings.tmdbKey) return null;
    try {
      meta = await tmdb(`/${entry.type}/${entry.tmdbId}`);
      metaCache.set(key, meta);
    } catch {
      metaCache.set(key, false);
      return null;
    }
  }
  if (meta === false) return null;
  return { ...entry, meta };
}

function renderEmpty(title, body) {
  grid.innerHTML = '';
  const wrap = document.createElement('div');
  wrap.className = 'empty';
  wrap.innerHTML = `<h3>${escapeHTML(title)}</h3><p>${escapeHTML(body)}</p>`;
  grid.appendChild(wrap);
}

function renderResumeItems(items) {
  grid.innerHTML = '';
  cardItems.clear();
  if (!items.length) {
    renderEmpty(
      t('library.emptyResumeTitle'),
      t('library.emptyResumeBody'),
    );
    return;
  }
  const frag = document.createDocumentFragment();
  for (const it of items) {
    const meta = it.meta;
    const node = cardTpl.content.firstElementChild.cloneNode(true);
    const cardId = `${it.type}:${it.tmdbId}`;
    node.dataset.id = cardId;
    node.dataset.libraryTab = 'continua';
    node.href = '#';
    const img = node.querySelector('img');
    if (meta.poster_path) {
      img.src = `${TMDB_IMG}/w342${meta.poster_path}`;
      img.alt = meta.title || meta.name || '';
    } else {
      img.remove();
    }
    const rating = node.querySelector('.card-rating');
    if (meta.vote_average && meta.vote_average > 0) {
      rating.textContent = meta.vote_average.toFixed(1);
    } else {
      rating.classList.add('empty');
    }
    node.querySelector('.card-title').textContent = meta.title || meta.name || '—';
    const seen = fmtRelative(it.ts);
    node.querySelector('.card-year').textContent = it.type === 'tv'
      ? `S${String(it.season).padStart(2, '0')}E${String(it.episode).padStart(2, '0')} · ${seen}`
      : seen;
    const bar = node.querySelector('.card-progress');
    const fill = bar.querySelector('.card-progress-fill');
    fill.style.width = `${Math.max(0, Math.min(100, (it.time / it.duration) * 100))}%`;
    bar.hidden = false;
    cardItems.set(cardId, it);
    frag.appendChild(node);
  }
  grid.appendChild(frag);
}

async function loadContinua() {
  const myGen = ++gen;
  const entries = listResume(50);
  if (!entries.length) {
    renderResumeItems([]);
    return;
  }
  const resolved = await Promise.all(entries.map(loadMeta));
  if (myGen !== gen) return;
  renderResumeItems(resolved.filter(Boolean));
}

function renderFavoriteItems(items) {
  grid.innerHTML = '';
  cardItems.clear();
  if (!items.length) {
    renderEmpty(
      t('library.emptyFavoritesTitle'),
      t('library.emptyFavoritesBody'),
    );
    return;
  }
  const frag = document.createDocumentFragment();
  for (const it of items) {
    const meta = it.meta;
    const node = cardTpl.content.firstElementChild.cloneNode(true);
    const cardId = `${it.type}:${it.tmdbId}`;
    node.dataset.id = cardId;
    node.dataset.libraryTab = 'preferiti';
    node.href = '#';
    const img = node.querySelector('img');
    if (meta.poster_path) {
      img.src = `${TMDB_IMG}/w342${meta.poster_path}`;
      img.alt = meta.title || meta.name || '';
    } else {
      img.remove();
    }
    const rating = node.querySelector('.card-rating');
    if (meta.vote_average && meta.vote_average > 0) {
      rating.textContent = meta.vote_average.toFixed(1);
    } else {
      rating.classList.add('empty');
    }
    node.querySelector('.card-title').textContent = meta.title || meta.name || '—';
    const date = it.type === 'tv' ? meta.first_air_date : meta.release_date;
    node.querySelector('.card-year').textContent = (date || '').slice(0, 4) || '';
    cardItems.set(cardId, it);
    frag.appendChild(node);
  }
  grid.appendChild(frag);
}

async function loadPreferiti() {
  const myGen = ++gen;
  const entries = listFavorites(100);
  if (!entries.length) {
    renderFavoriteItems([]);
    return;
  }
  const resolved = await Promise.all(entries.map(loadMeta));
  if (myGen !== gen) return;
  renderFavoriteItems(resolved.filter(Boolean));
}

function loadDownload() {
  cardItems.clear();
  loadDownloads();
  startDownloadsPolling();
}

function leaveDownloadTab() {
  stopDownloadsPolling();
}

export function refreshDlMagnetBarVisibility() {
  updateDlMagnetBarVisibility();
}

export async function loadLibrary() {
  if (state.section !== 'library') return;
  setActiveTab(state.libraryTab);
  if (state.libraryTab === 'preferiti') {
    leaveDownloadTab();
    return loadPreferiti();
  }
  if (state.libraryTab === 'download') {
    return loadDownload();
  }
  leaveDownloadTab();
  return loadContinua();
}

$$('[data-library-tab]', libraryBar).forEach(btn => {
  btn.addEventListener('click', () => {
    const tab = btn.dataset.libraryTab;
    if (state.libraryTab === tab) return;
    if (state.libraryTab === 'download' && tab !== 'download') leaveDownloadTab();
    setActiveTab(tab);
    loadLibrary();
  });
});

grid.addEventListener('click', e => {
  if (state.section !== 'library') return;
  const card = e.target.closest('.card');
  if (!card || !card.dataset.libraryTab) return;
  e.preventDefault();
  const it = cardItems.get(card.dataset.id);
  if (!it) return;
  openDetails(it.meta, { type: it.type });
});

window.addEventListener('siiis:resume', () => {
  if (state.section === 'library' && state.libraryTab === 'continua') loadLibrary();
});

window.addEventListener('siiis:favorite', () => {
  if (state.section === 'library' && state.libraryTab === 'preferiti') loadLibrary();
});
