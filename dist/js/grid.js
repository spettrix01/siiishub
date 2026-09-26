import { $, $$, safeJsonParse } from './dom.js';
import { state, TMDB_IMG, tmdbType } from './state.js';
import { fetchPage, downloadList, downloadRemove, downloadPause, downloadResume, downloadOpenFolder, downloadPlay, tmdb } from './api.js';
import { getFullDate, fmtSpeed, fmtBytes, fmtDownloadDate } from './format.js';
import { openDetails } from './details.js';
import { openPlayerWithUrl } from './player.js';
import { openSettings } from './settings-ui.js';
import { getCardProgress } from './resume.js';
import { showConfirm, showMultiSelectConfirm } from './modal.js';
import { t, locMsg } from './i18n.js';

const downloadRatingCache = new Map();
const downloadRatingPending = new Map();
const downloadEntriesById = new Map();

const TRASH_SVG = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" d="M5 7h14M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2"/></svg>`;
const FOLDER_SVG = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"/></svg>`;
const PLAY_SVG = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M8 5.14v13.72c0 .79.87 1.27 1.54.84l10.54-6.86a1 1 0 0 0 0-1.68L9.54 4.3A1 1 0 0 0 8 5.14Z"/></svg>`;
const PAUSE_SVG = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M7 5h3.2v14H7zM13.8 5H17v14h-3.2z"/></svg>`;

const grid = $('#grid');
const cardTpl = $('#cardTpl');
const statusEl = $('#status');
const sentinel = $('#sentinel');

function setStatus(text, isErr = false) {
  statusEl.textContent = text;
  statusEl.classList.toggle('err', isErr);
}

export function clearGrid() {
  grid.innerHTML = '';
  state.items.clear();
  state.page = 1;
  state.done = false;
  state.loading = false;
  state.gen++;
}

function renderSkeletons(n = 12) {
  const frag = document.createDocumentFragment();
  for (let i = 0; i < n; i++) {
    const node = cardTpl.content.firstElementChild.cloneNode(true);
    node.classList.add('skeleton');
    node.removeAttribute('href');
    node.querySelector('img').remove();
    node.querySelector('.card-rating').remove();
    frag.appendChild(node);
  }
  grid.appendChild(frag);
}

function clearSkeletons() {
  $$('.skeleton', grid).forEach(n => n.remove());
}

function renderItem(item) {
  if (state.items.has(item.id)) return null;
  state.items.set(item.id, item);
  const node = cardTpl.content.firstElementChild.cloneNode(true);
  node.dataset.id = item.id;
  node.href = '#';
  const img = node.querySelector('img');
  if (item.poster_path) {
    img.src = `${TMDB_IMG}/w342${item.poster_path}`;
    img.alt = item.title || item.name || '';
  } else {
    img.remove();
  }
  const rating = node.querySelector('.card-rating');
  if (item.vote_average && item.vote_average > 0) {
    rating.textContent = item.vote_average.toFixed(1);
  } else {
    rating.classList.add('empty');
  }
  node.querySelector('.card-title').textContent = item.title || item.name || '—';
  node.querySelector('.card-year').textContent = getFullDate(item) || '';
  applyCardProgress(node, item.id);
  return node;
}

function applyCardProgress(card, tmdbId) {
  const bar = card.querySelector('.card-progress');
  if (!bar) return;
  const fill = bar.querySelector('.card-progress-fill');
  const p = getCardProgress(tmdbType(), tmdbId);
  if (p != null && p > 0) {
    fill.style.width = `${Math.min(100, p * 100)}%`;
    bar.hidden = false;
  } else {
    bar.hidden = true;
    fill.style.width = '0';
  }
}

window.addEventListener('siiis:resume', (e) => {
  const ctx = e.detail;
  if (!ctx || ctx.tmdbId == null) {
    $$('.card', grid).forEach(c => {
      const id = Number(c.dataset.id);
      if (id) applyCardProgress(c, id);
    });
    return;
  }
  const expectedType = ctx.type === 'tv' ? 'tv' : 'movie';
  if (expectedType !== tmdbType()) return;
  const card = grid.querySelector(`.card[data-id="${ctx.tmdbId}"]`);
  if (card) applyCardProgress(card, ctx.tmdbId);
});

grid.addEventListener('click', e => {
  const card = e.target.closest('.card');
  if (!card || card.classList.contains('skeleton') || !grid.contains(card)) return;
  if (state.section === 'library' && state.libraryTab !== 'download') return;
  e.preventDefault();

  if (state.section === 'library' && state.libraryTab === 'download') {
    let dlIds = safeJsonParse(card.dataset.dlIds, []);
    if (!Array.isArray(dlIds)) dlIds = [];
    if (!dlIds.length && card.dataset.dlId) dlIds = [card.dataset.dlId];
    if (!dlIds.length) return;
    const tmdbId = card.dataset.tmdbId ? Number(card.dataset.tmdbId) : null;
    const cardType = card.dataset.tmdbType || 'movie';
    const title = card.dataset.title || '';
    const posterUrl = card.dataset.posterUrl || '';
    const item = {
      id: tmdbId,
      title: cardType === 'movie' ? title : undefined,
      name: cardType === 'tv' ? title : undefined,
      poster_path: posterUrl ? (posterUrl.match(/\/t\/p\/[^/]+(\/[^?#]+)/)?.[1] ?? null) : null,
      _downloadId: dlIds[0],
      _downloadIds: dlIds,
      _posterUrl: posterUrl,
    };
    openDetails(item, {
      downloadMode: true,
      downloadId: dlIds[0],
      downloadIds: dlIds,
    });
    return;
  }
  const id = Number(card.dataset.id);
  const item = state.items.get(id);
  if (!item) return;
  openDetails(item);
});

function appendItems(items) {
  const frag = document.createDocumentFragment();
  for (const it of items) {
    const n = renderItem(it);
    if (n) frag.appendChild(n);
  }
  grid.appendChild(frag);
}

export function showEmpty(title, body, ctaText, ctaFn) {
  grid.innerHTML = '';
  const wrap = document.createElement('div');
  wrap.className = 'empty';
  wrap.innerHTML = `<h3>${title}</h3><p>${body}</p>`;
  if (ctaText) {
    const btn = document.createElement('button');
    btn.className = 'primary';
    btn.textContent = ctaText;
    btn.onclick = ctaFn;
    wrap.appendChild(btn);
  }
  grid.appendChild(wrap);
}

export async function loadMore() {
  if (state.section === 'library') return;
  if (state.loading || state.done) return;
  if (!state.settings.tmdbKey) return;
  state.loading = true;
  const myGen = state.gen;
  setStatus(t('common.loading'));
  try {
    if (state.page === 1) renderSkeletons(12);
    const data = await fetchPage(state.page);
    if (myGen !== state.gen) return;
    if (state.page === 1) clearSkeletons();
    const list = (data.results || []).filter(it => it.poster_path);
    appendItems(list);
    if (state.page >= (data.total_pages || 0) || list.length === 0) {
      state.done = true;
      setStatus(state.items.size ? t('grid.endOfResults') : '');
    } else {
      setStatus('');
      // The observer only calls when the sentinel comes into reach: one
      // already in reach (a page too short to fill a big screen, or a call
      // that came while this page was loading) is looked at again.
      io.unobserve(sentinel);
      io.observe(sentinel);
    }
    state.page++;
  } catch (e) {
    if (myGen !== state.gen) return;
    clearSkeletons();
    if (e.message === 'NO_KEY') {
      showEmpty(
        t('grid.noKeyTitle'),
        t('grid.noKeyBody'),
        t('welcome.openSettings'),
        () => openSettings(),
      );
      setStatus('');
    } else {
      setStatus(t('grid.errorPrefix', { error: e.message }), true);
    }
  } finally {
    if (myGen === state.gen) state.loading = false;
  }
}

const io = new IntersectionObserver(entries => {
  for (const e of entries) if (e.isIntersecting) loadMore();
}, { root: document.getElementById('page-scroll'), rootMargin: '600px 0px' });
io.observe(sentinel);

let downloadsPollTimer = 0;

function isDownloadView() {
  return state.section === 'library' && state.libraryTab === 'download';
}

export async function loadDownloads() {
  if (!isDownloadView()) return;
  let entries;
  try {
    entries = await downloadList();
  } catch {
    entries = [];
  }
  if (!isDownloadView()) return;
  if (!entries || !entries.length) {
    showEmpty(
      t('grid.noDownloadsTitle'),
      t('grid.noDownloadsBody'),
      null,
    );
    setStatus('');
    return;
  }
  renderDownloadCards(entries);
  setStatus('');
}

export function startDownloadsPolling() {
  if (downloadsPollTimer) return;
  downloadsPollTimer = setInterval(() => {
    if (!isDownloadView()) {
      stopDownloadsPolling();
      return;
    }
    loadDownloads();
  }, 1500);
}

export function stopDownloadsPolling() {
  if (downloadsPollTimer) clearInterval(downloadsPollTimer);
  downloadsPollTimer = 0;
}

function renderDownloadCards(entries) {
  downloadEntriesById.clear();
  for (const e of entries) downloadEntriesById.set(e.id, e);

  const hasTmdb = e => e.tmdbId != null && e.tmdbType;
  const tmdbEntries = entries.filter(hasTmdb);
  const groupEntries = entries.filter(e => !hasTmdb(e) && e.group);
  const manualEntries = entries.filter(e => !hasTmdb(e) && !e.group);

  const groups = new Map();
  for (const e of tmdbEntries) {
    const key = `tmdb:${e.tmdbType}:${e.tmdbId}`;
    if (!groups.has(key)) groups.set(key, { key, entries: [] });
    groups.get(key).entries.push(e);
  }

  const rowUnits = [];
  const magnetUnits = new Map();
  for (const e of groupEntries) {
    const key = `grp:${e.group}`;
    let unit = magnetUnits.get(key);
    if (!unit) {
      unit = { key, entries: [] };
      magnetUnits.set(key, unit);
      rowUnits.push(unit);
    }
    unit.entries.push(e);
  }
  for (const e of manualEntries) {
    rowUnits.push({ key: e.id, entries: [e] });
  }

  for (const node of [...grid.children]) {
    if (node.matches?.('.card[data-group-key]')) continue;
    if (node.matches?.('.dl-rows')) continue;
    node.remove();
  }

  const seenKeys = new Set(groups.keys());
  for (const card of [...grid.querySelectorAll('.card[data-group-key]')]) {
    if (!seenKeys.has(card.dataset.groupKey)) card.remove();
  }

  renderDownloadRows(rowUnits);

  for (const group of groups.values()) {
    const escapedKey = String(group.key).replace(/[^a-zA-Z0-9_-]/g, ch => `\\${ch}`);
    const existing = grid.querySelector(`.card[data-group-key="${escapedKey}"]`);
    const card = existing || cardTpl.content.firstElementChild.cloneNode(true);
    populateDownloadCard(card, group);
    if (!existing) grid.appendChild(card);
  }
}

function renderDownloadRows(units) {
  let container = grid.querySelector('.dl-rows');
  if (!units.length) {
    if (container) container.remove();
    return;
  }
  if (!container) {
    container = document.createElement('ul');
    container.className = 'dl-rows';
    grid.insertBefore(container, grid.firstChild);
  }

  const seen = new Set(units.map(u => u.key));
  for (const row of [...container.querySelectorAll('.dl-row')]) {
    if (!seen.has(row.dataset.key)) row.remove();
  }

  for (const unit of units) {
    const escaped = String(unit.key).replace(/[^a-zA-Z0-9_-]/g, ch => `\\${ch}`);
    const existing = container.querySelector(`.dl-row[data-key="${escaped}"]`);
    const row = existing || createDownloadRow(unit);
    populateDownloadRow(row, unit);
    if (!existing) container.appendChild(row);
  }
}

function createDownloadRow(unit) {
  const row = document.createElement('li');
  row.className = 'dl-row';
  row.dataset.key = unit.key;
  row.innerHTML = `
    <div class="dl-row-info">
      <p class="dl-row-title"></p>
      <p class="dl-row-meta"></p>
    </div>
    <div class="dl-row-progress"><div class="dl-row-progress-fill"></div></div>
    <button class="dl-row-btn dl-row-btn--play" type="button" data-action="play" aria-label="${t('details.play')}" title="${t('details.play')}" hidden>${PLAY_SVG}</button>
    <button class="dl-row-btn dl-row-btn--pause" type="button" data-action="pause" aria-label="${t('grid.pause')}" title="${t('grid.pause')}" hidden>${PAUSE_SVG}</button>
    <button class="dl-row-btn dl-row-btn--folder" type="button" data-action="folder" aria-label="${t('grid.openFolderShort')}" title="${t('settings.download.openFolder')}">${FOLDER_SVG}</button>
    <button class="dl-row-btn dl-row-btn--trash" type="button" data-action="trash" aria-label="${t('grid.deleteDownload')}" title="${t('grid.deleteDownload')}">${TRASH_SVG}</button>
  `;
  row.addEventListener('click', async (e) => {
    const action = e.target.closest('[data-action]')?.dataset?.action;
    if (!action) return;
    e.preventDefault();
    e.stopPropagation();
    let ids = safeJsonParse(row.dataset.ids, []);
    if (!Array.isArray(ids)) ids = [];
    if (!ids.length) return;
    if (action === 'play') {
      try {
        const url = await downloadPlay(ids[0]);
        openPlayerWithUrl({
          url,
          title: row.dataset.title || t('grid.untitled'),
          eyebrowKey: 'details.eyebrow.download',
          ctx: { type: 'local', id: ids[0] },
        });
      } catch (err) {
        console.warn('row play failed', err);
      }
    } else if (action === 'folder') {
      downloadOpenFolder(ids[0]);
    } else if (action === 'pause' || action === 'resume') {
      const btn = e.target.closest('[data-action]');
      if (btn) btn.disabled = true;
      const fn = action === 'pause' ? downloadPause : downloadResume;
      try {
        await Promise.all(ids.map(id => fn(id)));
      } finally {
        if (btn) btn.disabled = false;
        loadDownloads();
      }
    } else if (action === 'trash') {
      const title = row.dataset.title || t('grid.thisDownload');
      let toDelete;
      if (ids.length === 1) {
        const rec = downloadEntriesById.get(ids[0]);
        const label = rec?.title || rec?.filename || t('grid.thisDownload');
        const ok = await showConfirm(
          t('grid.confirmDeleteOne', { label }),
          { title: t('grid.confirmDeleteTitle'), okLabel: t('grid.delete'), cancelLabel: t('common.cancel') },
        );
        toDelete = ok ? ids.slice() : null;
      } else {
        const items = ids.map(id => {
          const rec = downloadEntriesById.get(id);
          return {
            value: id,
            label: rec?.filename || rec?.title || t('grid.downloadFallback'),
            sub: [rec?.addon, fmtDownloadDate(Number(rec?.started_at) || 0)]
              .filter(Boolean)
              .join(' · '),
          };
        });
        toDelete = await showMultiSelectConfirm(items, {
          title: t('grid.confirmDeleteWhich'),
          description: t('grid.confirmDeleteManyFiles', { title, count: ids.length }),
          okLabel: t('grid.delete'),
          cancelLabel: t('common.cancel'),
        });
      }
      if (!toDelete || !toDelete.length) return;
      const trashBtn = row.querySelector('[data-action="trash"]');
      if (trashBtn) trashBtn.disabled = true;
      try {
        await Promise.all(toDelete.map(id => downloadRemove(id).catch(err => {
          console.warn('downloadRemove failed', id, err);
        })));
      } finally {
        if (trashBtn) trashBtn.disabled = false;
        loadDownloads();
      }
    }
  });
  return row;
}

function populateDownloadRow(row, unit) {
  const entries = unit.entries || [];
  const sorted = entries.slice().sort((a, b) =>
    (b.started_at || 0) - (a.started_at || 0) || String(a.id).localeCompare(String(b.id)));
  const head = sorted[0] || entries[0] || {};
  const isGroup = !!head.group;

  row.dataset.ids = JSON.stringify(entries.map(e => e.id));
  const title = head.title || head.filename || t('grid.untitled');
  row.dataset.title = title;

  const titleEl = row.querySelector('.dl-row-title');
  const metaEl = row.querySelector('.dl-row-meta');
  const fill = row.querySelector('.dl-row-progress-fill');

  titleEl.textContent = title;

  const statsList = entries.map(e => e.stats).filter(Boolean);
  const isError = statsList.some(s => s?.error);
  const isReady = !isError
    && entries.length > 0
    && statsList.length === entries.length
    && statsList.every(s =>
      (s.length > 0 && s.downloaded >= s.length) ||
      (typeof s.progress === 'number' && s.progress >= 0.999));
  const isPaused = !isReady && !isError && statsList.length > 0 && statsList.some(s => s?.paused);

  const sized = statsList.filter(s => s.length > 0);
  let pct = 0;
  if (sized.length) {
    const totalLen = sized.reduce((a, s) => a + s.length, 0);
    const totalDown = sized.reduce((a, s) => a + Math.min(s.downloaded, s.length), 0);
    pct = totalLen > 0 ? Math.min(100, (totalDown / totalLen) * 100) : 0;
  } else if (statsList.length) {
    const avg = statsList.reduce((a, s) =>
      a + (typeof s.progress === 'number' ? s.progress : 0), 0) / statsList.length;
    pct = Math.min(100, Math.max(0, avg * 100));
  }
  const speedVal = statsList.reduce((a, s) =>
    a + (typeof s.download_speed === 'number' ? s.download_speed : 0), 0);

  row.classList.toggle('is-error', isError);
  row.classList.toggle('is-ready', !!isReady);
  row.classList.toggle('is-paused', isPaused);
  fill.style.width = `${isError ? 100 : pct}%`;

  const playBtn = row.querySelector('.dl-row-btn--play');
  if (playBtn) {
    // A single finished file (local drop, completed HTTP) or a single-file
    // torrent that is complete or streaming is playable in-app.
    const playable = entries.length === 1
      && ((isReady && (head.kind === 'local' || head.kind === 'http'))
        || (head.kind === 'torrent' && (isReady || head.stats?.live === true)));
    playBtn.hidden = !playable;
  }

  const pauseBtn = row.querySelector('.dl-row-btn--pause');
  if (pauseBtn) {
    // librqbit can only pause a live torrent: while fetching metadata or
    // re-checking files after a restart the button would silently fail.
    const canPauseResume = entries.every(e =>
      e.kind !== 'torrent' || e.stats?.live === true || e.stats?.paused === true);
    const showPR = !isReady && !isError && statsList.length > 0 && canPauseResume;
    pauseBtn.hidden = !showPR;
    if (showPR) {
      pauseBtn.dataset.action = isPaused ? 'resume' : 'pause';
      pauseBtn.innerHTML = isPaused ? PLAY_SVG : PAUSE_SVG;
      const lbl = isPaused ? t('grid.resume') : t('grid.pause');
      pauseBtn.title = lbl;
      pauseBtn.setAttribute('aria-label', lbl);
    }
  }

  const speed = speedVal > 0 ? fmtSpeed(speedVal) : '';
  const tsMin = entries
    .map(e => Number(e.started_at) || 0)
    .filter(n => n > 0)
    .reduce((a, b) => Math.min(a, b), Number.POSITIVE_INFINITY);
  const date = Number.isFinite(tsMin) ? fmtDownloadDate(tsMin) : '';
  const kindLabel = isGroup
    ? t('grid.fileCount', { count: entries.length })
    : (head.kind === 'http' ? 'HTTP' : head.kind === 'local' ? t('grid.localFile') : 'Torrent');

  const totalSize = sized.length ? fmtBytes(sized.reduce((a, s) => a + s.length, 0)) : '';
  let status;
  if (isError) {
    status = locMsg(statsList.find(s => s?.error)?.error) || t('common.error');
  } else if (isReady) {
    status = totalSize ? t('grid.completeWithSize', { size: totalSize }) : t('grid.complete');
  } else if (isPaused) {
    status = sized.length
      ? `${t('grid.paused')} · ${t('grid.percentOfSize', { pct: pct.toFixed(0), size: totalSize })}`
      : t('grid.paused');
  } else if (sized.length) {
    status = `${t('grid.percentOfSize', { pct: pct.toFixed(0), size: totalSize })}${speed ? ' · ' + speed : ''}`;
  } else if (statsList.some(s => s.downloaded > 0)) {
    const down = statsList.reduce((a, s) => a + (s.downloaded || 0), 0);
    const parts = [fmtBytes(down)];
    if (speed) parts.push(speed);
    status = parts.join(' · ');
  } else if (statsList.length) {
    status = speed || t('grid.connecting');
  } else {
    status = t('grid.queued');
  }

  metaEl.textContent = [kindLabel, status, date].filter(Boolean).join(' · ');
}

function populateDownloadCard(card, group) {
  const sorted = group.entries.slice().sort((a, b) =>
    (b.started_at || 0) - (a.started_at || 0) || a.id.localeCompare(b.id),
  );
  const head = sorted[0];
  const ids = group.entries.map(e => e.id);

  card.dataset.groupKey = group.key;
  card.dataset.dlIds = JSON.stringify(ids);
  card.dataset.dlId = head.id;
  card.dataset.tmdbId = head.tmdbId != null ? String(head.tmdbId) : '';
  card.dataset.tmdbType = head.tmdbType || 'movie';
  card.dataset.title = head.title || '';
  card.dataset.posterUrl = head.poster_url || '';
  card.removeAttribute('href');

  const img = card.querySelector('img');
  if (head.poster_url) {
    if (img.src !== head.poster_url) img.src = head.poster_url;
    img.alt = head.title || '';
  } else if (img) {
    img.removeAttribute('src');
  }

  const rating = card.querySelector('.card-rating');
  if (rating) {
    rating.classList.add('empty');
    rating.textContent = '';
    if (head.tmdbId != null && head.tmdbType) {
      const key = `${head.tmdbType}:${head.tmdbId}`;
      const cached = downloadRatingCache.get(key);
      if (typeof cached === 'number' && cached > 0) {
        rating.textContent = cached.toFixed(1);
        rating.classList.remove('empty');
      } else if (cached !== 0) {
        ensureDownloadRating(head.tmdbType, head.tmdbId, card);
      }
    }
  }
  const bar = card.querySelector('.card-progress');
  if (bar) bar.hidden = true;

  const statsList = group.entries.map(e => e.stats).filter(Boolean);
  const isError = statsList.some(s => s?.error);
  const allReady = statsList.length === group.entries.length
    && statsList.every(s => s.length > 0 &&
      (s.downloaded >= s.length || (typeof s.progress === 'number' && s.progress >= 0.999)));
  const isReady = !isError && allReady;
  const isPaused = !isReady && !isError && statsList.length > 0 && statsList.some(s => s?.paused);
  let pct = 0;
  if (statsList.length) {
    const sized = statsList.filter(s => s.length > 0);
    if (sized.length) {
      const totalLen = sized.reduce((a, s) => a + s.length, 0);
      const totalDown = sized.reduce((a, s) => a + Math.min(s.downloaded, s.length), 0);
      pct = totalLen > 0 ? Math.min(100, (totalDown / totalLen) * 100) : 0;
    } else {
      const avg = statsList.reduce((a, s) =>
        a + (typeof s.progress === 'number' ? s.progress : 0), 0) / statsList.length;
      pct = Math.min(100, Math.max(0, avg * 100));
    }
  }
  const aggregateSpeed = statsList.reduce((a, s) =>
    a + (typeof s.download_speed === 'number' ? s.download_speed : 0), 0);

  const poster = card.querySelector('.poster');
  let trash = poster.querySelector('.card-trash');
  if (!trash) {
    trash = document.createElement('button');
    trash.type = 'button';
    trash.className = 'card-trash';
    trash.setAttribute('aria-label', t('grid.deleteDownload'));
    trash.title = t('grid.deleteDownload');
    trash.innerHTML = TRASH_SVG;
    trash.addEventListener('click', async (e) => {
      e.preventDefault();
      e.stopPropagation();
      let groupIds = safeJsonParse(card.dataset.dlIds, []);
      if (!Array.isArray(groupIds)) groupIds = [];
      if (!groupIds.length && card.dataset.dlId) groupIds = [card.dataset.dlId];
      if (!groupIds.length) return;
      const title = card.dataset.title || t('grid.thisDownload');

      let toDelete;
      if (groupIds.length === 1) {
        const ok = await showConfirm(
          t('grid.confirmDeleteOne', { label: title }),
          { title: t('grid.confirmDeleteTitle'), okLabel: t('grid.delete'), cancelLabel: t('common.cancel') },
        );
        toDelete = ok ? groupIds : null;
      } else {
        const items = groupIds.map(id => {
          const rec = downloadEntriesById.get(id);
          return {
            value: id,
            label: rec?.filename
              || rec?.title
              || (rec?.addon ? t('grid.addonDownload', { addon: rec.addon }) : t('grid.downloadFallback')),
            sub: [rec?.addon, fmtDownloadDate(Number(rec?.started_at) || 0)]
              .filter(Boolean)
              .join(' · '),
          };
        });
        toDelete = await showMultiSelectConfirm(items, {
          title: t('grid.confirmDeleteWhich'),
          description: t('grid.confirmDeleteManyDownloads', { title, count: groupIds.length }),
          okLabel: t('grid.delete'),
          cancelLabel: t('common.cancel'),
        });
      }
      if (!toDelete || !toDelete.length) return;

      trash.disabled = true;
      const remaining = groupIds.filter(id => !toDelete.includes(id));
      try {
        await Promise.all(toDelete.map(id => downloadRemove(id).catch(err => {
          console.warn('downloadRemove failed', id, err);
        })));
      } finally {
        if (!remaining.length) {
          card.remove();
          if (!grid.querySelector('.card[data-group-key]')) {
            loadDownloads();
          }
        } else {
          trash.disabled = false;
          loadDownloads();
        }
      }
    });
    poster.appendChild(trash);
  }

  let cardPause = poster.querySelector('.card-pause');
  if (!cardPause) {
    cardPause = document.createElement('button');
    cardPause.type = 'button';
    cardPause.className = 'card-pause';
    cardPause.addEventListener('click', async (e) => {
      e.preventDefault();
      e.stopPropagation();
      let groupIds = safeJsonParse(card.dataset.dlIds, []);
      if (!Array.isArray(groupIds)) groupIds = [];
      if (!groupIds.length && card.dataset.dlId) groupIds = [card.dataset.dlId];
      if (!groupIds.length) return;
      const resume = cardPause.dataset.action === 'resume';
      cardPause.disabled = true;
      const fn = resume ? downloadResume : downloadPause;
      try {
        await Promise.all(groupIds.map(id => fn(id)));
      } finally {
        cardPause.disabled = false;
        loadDownloads();
      }
    });
    poster.appendChild(cardPause);
  }
  {
    // Same gate as the rows: only live torrents (or paused ones) can react.
    const canPauseResume = group.entries.every(e =>
      e.kind !== 'torrent' || e.stats?.live === true || e.stats?.paused === true);
    const showPR = !isReady && !isError && statsList.length > 0 && canPauseResume;
    cardPause.hidden = !showPR;
    if (showPR) {
      cardPause.dataset.action = isPaused ? 'resume' : 'pause';
      cardPause.innerHTML = isPaused ? PLAY_SVG : PAUSE_SVG;
      const lbl = isPaused ? t('grid.resume') : t('grid.pause');
      cardPause.title = lbl;
      cardPause.setAttribute('aria-label', lbl);
    }
  }

  let overlay = poster.querySelector('.card-dl-overlay');
  if (!overlay) {
    overlay = document.createElement('div');
    overlay.className = 'card-dl-overlay';
    overlay.innerHTML = `
      <div class="card-dl-ring">
        <svg viewBox="0 0 64 64" aria-hidden="true">
          <circle class="card-dl-ring-track" cx="32" cy="32" r="27"/>
          <circle class="card-dl-ring-fill"  cx="32" cy="32" r="27"/>
        </svg>
        <div class="card-dl-pct"></div>
      </div>
      <div class="card-dl-speed"></div>
    `;
    poster.appendChild(overlay);
  }

  if (isReady) {
    overlay.hidden = true;
    overlay.classList.remove('is-error');
  } else {
    overlay.hidden = false;
    overlay.classList.toggle('is-error', isError);
    const ring = overlay.querySelector('.card-dl-ring');
    ring.style.setProperty('--p', isError ? 1 : (pct / 100));
    const anyStats = statsList.length > 0;
    const anySized = statsList.some(s => s.length > 0);
    overlay.querySelector('.card-dl-pct').textContent = isError
      ? '!'
      : (anySized ? `${pct.toFixed(0)}%` : '…');
    const errMsg = locMsg(statsList.find(s => s.error)?.error) || t('common.error');
    overlay.querySelector('.card-dl-speed').textContent = isError
      ? errMsg
      : (isPaused
          ? t('grid.paused')
          : (aggregateSpeed > 0
              ? fmtSpeed(aggregateSpeed)
              : (anyStats && !anySized ? t('grid.fetchingInfo') : t('grid.queued'))));
  }

  card.querySelector('.card-title').textContent = head.title || t('grid.untitled');
  const yearEl = card.querySelector('.card-year');
  if (yearEl) {
    const ts = group.entries
      .map(e => Number(e.started_at) || 0)
      .filter(n => n > 0)
      .reduce((a, b) => Math.min(a, b), Number.POSITIVE_INFINITY);
    yearEl.textContent = Number.isFinite(ts) ? fmtDownloadDate(ts) : '';
  }
}

function ensureDownloadRating(type, id, card) {
  const key = `${type}:${id}`;
  if (downloadRatingPending.has(key)) {
    downloadRatingPending.get(key).then(score => paintRating(card, score)).catch(() => {});
    return;
  }
  const p = (async () => {
    try {
      const detail = await tmdb(`/${type}/${id}`);
      const score = typeof detail?.vote_average === 'number' ? detail.vote_average : 0;
      downloadRatingCache.set(key, score);
      return score;
    } catch {
      downloadRatingCache.set(key, 0);
      return 0;
    } finally {
      downloadRatingPending.delete(key);
    }
  })();
  downloadRatingPending.set(key, p);
  p.then(score => paintRating(card, score)).catch(() => {});
}

function paintRating(card, score) {
  if (!document.body.contains(card)) return;
  const rating = card.querySelector('.card-rating');
  if (!rating) return;
  if (typeof score === 'number' && score > 0) {
    rating.textContent = score.toFixed(1);
    rating.classList.remove('empty');
  }
}
