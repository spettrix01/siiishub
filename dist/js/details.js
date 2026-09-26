import { IS_PHONE, IS_TV } from './platform.js';
import { $, escapeHTML, DOTS_HTML, CHECK_SVG, COPY_SVG, copyToClipboard, openExternal } from './dom.js';
import { state, tmdbType, TMDB_IMG } from './state.js';
import { tmdb, fetchStreams, fetchTvSeason, rdPlay, onMediaProgress, downloadStart, downloadPlay, downloadFiles, downloadList, destroyTorrentSession, deviceVideoCaps } from './api.js';
import { fmtFullDate, fmtMoney, fmtRuntime, fmtVote } from './format.js';
import { sha256 } from './hash.js';
import { streamPickerHtml, setupStreamPicker } from './picker.js';
import { openPlayerWithUrl, openPlayerLoading, attachToOpenPlayer, pushPlayerLog, showPlayerError, setPlayerAbortController } from './player.js';
import { closeModal, showConfirm } from './modal.js';
import { openSettings } from './settings-ui.js';
import { isFavorite, toggleFavorite } from './favorites.js';
import { saveWatchedStream, getWatchedStream, lastEpisode } from './resume.js';
import { t, locMsg, tmdbLang } from './i18n.js';

const detailsModal = $('#detailsModal');
// What the TV's hardware video decoders take, asked once (android-player).
const videoCaps = IS_TV ? deviceVideoCaps() : Promise.resolve(null);
// Streams a TV lists at a time (renderList).
const STREAM_BATCH = 20;

const STATUS_KEYS = {
  'Released': 'details.status.released',
  'Post Production': 'details.status.postProduction',
  'In Production': 'details.status.inProduction',
  'Planned': 'details.status.planned',
  'Returning Series': 'details.status.returning',
  'Ended': 'details.status.ended',
  'Canceled': 'details.status.canceled',
  'Pilot': 'details.status.pilot',
};
const statusLabel = (status) => (STATUS_KEYS[status] ? t(STATUS_KEYS[status]) : status);

const PLAY_ICON = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M8 5.14v13.72c0 .79.87 1.27 1.54.84l10.54-6.86a1 1 0 0 0 0-1.68L9.54 4.3A1 1 0 0 0 8 5.14Z"/></svg>`;
const DOWNLOAD_ICON = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 4v11m0 0 4-4m-4 4-4-4M5 19h14"/></svg>`;

async function streamDownloadId(s) {
  if (s.infoHash) return String(s.infoHash).toLowerCase();
  if (!s.url) return null;
  const bytes = await sha256(new TextEncoder().encode(s.url));
  return [...bytes.slice(0, 20)]
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
}

// A stream across sessions: the torrent and its file (a season pack holds
// every episode), or a hash of the URL, which may carry the debrid key.
async function streamWatchKey(s) {
  const id = await streamDownloadId(s);
  return id ? `${id}:${s.fileIdx ?? ''}` : null;
}

const QUALITY_TIERS = [
  { re: /2160p|\b4k\b|uhd/, rank: 4, label: '2160p', cls: 'q-2160' },
  { re: /1080p|fullhd|fhd/, rank: 3, label: '1080p', cls: 'q-1080' },
  { re: /720p|hd\b/,        rank: 2, label: '720p',  cls: 'q-720'  },
  { re: /480p|sd\b|cam\b/,  rank: 1, label: '480p',  cls: 'q-480'  },
];
const QUALITY_FALLBACK = { rank: 0, label: 'SD', cls: 'q-sd' };

const RE_SEEDERS = /👤\s*([\d.,]+\s*[KkMm]?)/;
const RE_SIZE    = /💾\s*([\d.,]+\s*[KMGTkmgt]?[Bb]?)/;

function seedersNum(s) {
  if (!s) return 0;
  const m = String(s).match(/([\d.,]+)\s*([kKmM]?)/);
  if (!m) return 0;
  const n = parseFloat(m[1].replace(/,/g, ''));
  if (!Number.isFinite(n)) return 0;
  const suf = m[2].toLowerCase();
  return suf === 'm' ? n * 1e6 : suf === 'k' ? n * 1e3 : n;
}
const STAT_STOP  = '\\n👤💾🌐🌎⚡✨🔎📹🔊⭐🏷️|';
const RE_SOURCE_GEAR  = new RegExp(`⚙️?\\s*([^${STAT_STOP}]+?)(?=$|[${STAT_STOP}])`);
const RE_SOURCE_INDEX = new RegExp(`🔎\\s*([^${STAT_STOP}]+?)(?=$|[${STAT_STOP}])`);
const RE_LANG_TAG     = new RegExp(`🌐\\s*([^${STAT_STOP}]+?)(?=$|[${STAT_STOP}])`);
const RE_FLAGS        = /\p{Regional_Indicator}\p{Regional_Indicator}/gu;

function qualityRank(text) {
  const t = (text || '').toLowerCase();
  for (const tier of QUALITY_TIERS) {
    if (tier.re.test(t)) return { rank: tier.rank, label: tier.label, cls: tier.cls };
  }
  return { ...QUALITY_FALLBACK };
}

function extractSource(text) {
  return (text.match(RE_SOURCE_GEAR) || text.match(RE_SOURCE_INDEX))?.[1]?.trim() || null;
}

function extractLang(text) {
  const tagged = text.match(RE_LANG_TAG)?.[1]?.trim();
  if (tagged) return tagged;
  const flags = text.match(RE_FLAGS);
  if (flags?.length) return [...new Set(flags)].slice(0, 6).join(' ');
  return null;
}

function buildMagnet(infoHash, displayName) {
  const params = new URLSearchParams();
  params.append('xt', `urn:btih:${infoHash}`);
  if (displayName) params.append('dn', displayName);
  for (const tr of (state.settings.tracker_fallbacks || [])) params.append('tr', tr);
  return `magnet:?${params.toString()}`;
}

function streamLink(s) {
  if (s.url) return s.url;
  if (s.infoHash) {
    const dn = (s.title || s.name || '').split('\n')[0].slice(0, 200);
    return buildMagnet(s.infoHash, dn);
  }
  return '';
}

function parseStreamMeta(s) {
  const meta = s.title || s.description || '';
  const fullText = `${s.name || ''} ${meta}`;
  const lines = meta.split('\n').map(l => l.trim()).filter(Boolean);
  const titleLine = lines[0]?.replace(/^📄\s*/, '')
    || (s.name || '').split('\n')[0]
    || '—';
  const metaLine = lines.slice(1).join(' ');
  return {
    q: qualityRank(fullText),
    av1: /\bav1\b/i.test(fullText),
    titleLine,
    seeders: metaLine.match(RE_SEEDERS)?.[1]?.trim() || null,
    size:    metaLine.match(RE_SIZE)?.[1]?.trim()    || null,
    source:  extractSource(metaLine),
    lang:    extractLang(metaLine),
    rd:      detectRdStatus(s),
  };
}

const RE_DEBRID_TAG = /\[(RD|AD|PM|TB|DL|OC|EC|RS)([^\]]*)\]/i;
const DEBRID_NAMES = {
  RD: 'RD',
  AD: 'AD',
  PM: 'PM',
  TB: 'TB',
  DL: 'DL',
  OC: 'OC',
  EC: 'EC',
  RS: 'RS',
};

function detectRdStatus(s) {
  const m = (s.name || '').split('\n')[0].match(RE_DEBRID_TAG);
  if (!m) return null;
  const provider = m[1].toUpperCase();
  const inner = (m[2] || '').trim();
  const name = DEBRID_NAMES[provider] || provider;
  if (inner === '+' || inner.includes('⚡')) return { state: 'cached', label: `${name} ⚡` };
  if (/download/i.test(inner) || inner.includes('⬇')) return { state: 'download', label: `${name} ⬇` };
  return { state: 'rd', label: name };
}

export async function openDetails(item, opts = {}) {
  const downloadMode = !!opts.downloadMode;
  const downloadId = opts.downloadId || item._downloadId || null;

  const downloadIds = Array.isArray(opts.downloadIds) && opts.downloadIds.length
    ? opts.downloadIds.slice()
    : (Array.isArray(item._downloadIds) && item._downloadIds.length
        ? item._downloadIds.slice()
        : (downloadId ? [downloadId] : []));

  const type = opts.type
    ? opts.type
    : (downloadMode
      ? (item.name && !item.title ? 'tv' : 'movie')
      : tmdbType());
  detailsModal.hidden = false;
  document.body.style.overflow = 'hidden';

  const myGen = ++state.detailGen;
  state.detail = {
    type,
    id: item.id,
    item,
    imdb: null,
    seasons: [],
    downloadMode,
    downloadId,
    downloadIds,
  };

  const body = $('#detailsBody');
  body.innerHTML = `<div class="dossier-loading">${DOTS_HTML}</div>`;
  $('.dossier-card', detailsModal).scrollTop = 0;

  if (downloadMode && !item.id) {
    if (myGen !== state.detailGen) return;
    renderDownloadStubDossier(item, downloadId);
    return;
  }

  try {
    const detail = await tmdb(`/${type}/${item.id}`, {
      append_to_response: 'external_ids,credits,videos,images',
      include_image_language: `${(tmdbLang() || 'en').split('-')[0]},en,null`,
    });
    if (myGen !== state.detailGen) return;
    renderDetails(item, detail, type);
  } catch (e) {
    if (myGen !== state.detailGen) return;
    if (e.message === 'NO_KEY') return;
    if (downloadMode) {
      renderDownloadStubDossier(item, downloadId);
      return;
    }
    body.innerHTML = `<div class="dossier-error">${escapeHTML(t('details.loadError', { error: e.message }))}</div>`;
  }
}

function renderDownloadStubDossier(item, downloadId) {
  const body = $('#detailsBody');
  const title = item.title || item.name || '—';
  const poster = item._posterUrl || (item.poster_path ? `${TMDB_IMG}/w500${item.poster_path}` : '');
  body.innerHTML = `
    <div class="dossier-content no-hero">
      <div class="dossier-head">
        <div class="dossier-poster">
          ${poster ? `<img src="${escapeHTML(poster)}" alt="">` : ''}
        </div>
        <div class="dossier-text">
          <h2 class="dossier-title" id="detailsTitle">${escapeHTML(title)}</h2>
          <div class="dossier-actions">
            <button class="play-btn" type="button" data-play>
              <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M8 5.14v13.72c0 .79.87 1.27 1.54.84l10.54-6.86a1 1 0 0 0 0-1.68L9.54 4.3A1 1 0 0 0 8 5.14Z"/></svg>
              <span>${escapeHTML(t('details.play'))}</span>
            </button>
          </div>
        </div>
      </div>
    </div>`;
  body.querySelector('[data-play]')?.addEventListener('click', () => {
    playDownload(downloadId, title);
  });
}

function downloadPlayCtx(se = null) {
  const imdb = state.detail?.imdb || null;
  const tmdbId = state.detail?.id ?? null;
  if (!imdb && tmdbId == null) return null;
  if (state.detail?.type === 'tv') {
    if (!se) return null;
    return { type: 'tv', tmdbId, imdb, season: se.s, episode: se.e };
  }
  return { type: 'movie', tmdbId, imdb };
}

async function playDownload(downloadId, title) {
  const ids = (state.detail?.downloadIds && state.detail.downloadIds.length)
    ? state.detail.downloadIds.slice()
    : (downloadId ? [downloadId] : []);
  if (!ids.length) return;
  try {
    const aggregate = await loadAggregateFiles(ids);
    let firstId = ids[0];
    let firstIdx = null;
    let firstSE = null;
    if (aggregate.length) {
      const sortedSE = aggregate
        .filter(it => it._se)
        .sort((a, b) => (a._se.s - b._se.s) || (a._se.e - b._se.e));
      const pick = sortedSE[0]
        || aggregate.slice().sort((a, b) => b.size - a.size)[0];
      firstId = pick.downloadId;
      firstIdx = pick.idx;
      firstSE = pick._se || null;
    }
    const url = firstIdx != null
      ? await downloadPlay(firstId, firstIdx)
      : await downloadPlay(firstId);
    closeModal('#detailsModal');
    await openPlayerWithUrl({
      url,
      title,
      eyebrow: t('details.eyebrow.download'),
      eyebrowKey: 'details.eyebrow.download',
      ctx: downloadPlayCtx(firstSE),
    });
    if (aggregate.length >= 2) {
      buildAggregatePicker(aggregate, title, firstId, firstIdx);
    }
  } catch (err) {
    console.warn('downloadPlay failed', err);
  }
}

async function loadAggregateFiles(downloadIds) {
  const recById = new Map();
  try {
    for (const e of (await downloadList()) || []) recById.set(e.id, e);
  } catch {}
  const settled = await Promise.allSettled(
    downloadIds.map(id => downloadFiles(id).then(files => ({ id, files }))),
  );
  const out = [];
  for (const r of settled) {
    if (r.status !== 'fulfilled') continue;
    const { id, files } = r.value;
    if (!Array.isArray(files)) continue;
    const rec = recById.get(id);
    const recSE = (rec && rec.season != null && rec.episode != null)
      ? { s: Number(rec.season), e: Number(rec.episode) }
      : null;
    const useRecSE = !!recSE && files.length === 1;
    for (const f of files) {
      out.push({
        downloadId: id,
        idx: f.idx,
        path: f.path,
        basename: f.basename,
        size: f.size,
        _se: useRecSE ? recSE : parseSEFromName(f.basename || f.path || ''),
      });
    }
  }
  return out;
}

function buildAggregatePicker(aggregate, baseTitle, currentId, currentIdx) {
  const playerPickers = document.getElementById('playerPickers');
  const playerTitleEl = document.getElementById('playerTitle');
  if (!playerPickers) return;

  const bySeason = new Map();
  for (const f of aggregate) {
    const key = f._se ? f._se.s : null;
    if (!bySeason.has(key)) bySeason.set(key, []);
    bySeason.get(key).push(f);
  }

  for (const [k, arr] of bySeason) {
    arr.sort((a, b) => {
      if (a._se && b._se) return a._se.e - b._se.e;
      return (a.path || '').localeCompare(b.path || '');
    });
  }
  const seasonNumbers = [...bySeason.keys()].filter(k => k != null).sort((a, b) => a - b);
  const hasNullBucket = bySeason.has(null);
  const totalBuckets = seasonNumbers.length + (hasNullBucket ? 1 : 0);

  const currentFile = aggregate.find(f =>
    f.downloadId === currentId && f.idx === currentIdx);
  const currentSeason = currentFile?._se ? currentFile._se.s : null;

  const itemsForBucket = (key) => (bySeason.get(key) || []).map(f => ({
    value: `${f.downloadId}#${f.idx}`,
    label: fileLabel(f.basename, f._se),
    _downloadId: f.downloadId,
    _idx: f.idx,
    _se: f._se || null,
  }));

  const onPick = async (picked) => {
    if (!picked) return;
    try {
      const url = await downloadPlay(picked._downloadId, picked._idx);
      if (playerTitleEl) {
        playerTitleEl.textContent = `${baseTitle} — ${picked.label}`;
      }
      await attachToOpenPlayer(url, { ctx: downloadPlayCtx(picked._se) });
    } catch (err) {
      console.warn('switch episode failed', err);
    }
  };

  if (totalBuckets <= 1) {
    const items = itemsForBucket(seasonNumbers[0] ?? null);
    const pickerLabel = aggregate.some(f => f._se) ? t('details.picker.episode') : t('details.picker.version');
    const initialValue = `${currentId}#${currentIdx}`;
    playerPickers.innerHTML = streamPickerHtml('episode', pickerLabel, 'player-pick');
    const root = playerPickers.querySelector('[data-stream-pick="episode"]');
    setupStreamPicker(root, {
      items,
      value: initialValue,
      popup: IS_PHONE,
      onChange: (val) => onPick(items.find(it => it.value === val)),
    });
    return;
  }

  playerPickers.innerHTML =
    streamPickerHtml('season', t('details.picker.season'), 'player-pick')
    + streamPickerHtml('episode', t('details.picker.episode'), 'player-pick');
  const seasonRoot = playerPickers.querySelector('[data-stream-pick="season"]');
  const epRoot = playerPickers.querySelector('[data-stream-pick="episode"]');

  const seasonItems = [
    ...seasonNumbers.map(n => ({ value: String(n), label: t('details.seasonN', { n }) })),
    ...(hasNullBucket ? [{ value: 'extra', label: t('details.extra') }] : []),
  ];
  const initialSeason = currentSeason != null
    ? String(currentSeason)
    : (hasNullBucket ? 'extra' : String(seasonNumbers[0]));

  const initialBucketKey = initialSeason === 'extra' ? null : Number(initialSeason);
  let epItems = itemsForBucket(initialBucketKey);
  const initialEpValue = `${currentId}#${currentIdx}`;

  const epPicker = setupStreamPicker(epRoot, {
    items: epItems,
    value: initialEpValue,
    popup: IS_PHONE,
    onChange: (val) => onPick(epItems.find(it => it.value === val)),
  });

  setupStreamPicker(seasonRoot, {
    items: seasonItems,
    value: initialSeason,
    popup: IS_PHONE,
    onChange: (seasonVal) => {
      const bucketKey = seasonVal === 'extra' ? null : Number(seasonVal);
      epItems = itemsForBucket(bucketKey);
      epPicker.setItems(epItems);
      if (epItems[0]) {
        epPicker.setValue(epItems[0].value, true);
      }
    },
  });
}

function parseSEFromName(name) {
  const m = name.match(/[Ss](\d{1,2})[\s._-]*[Ee](\d{1,3})/);
  if (!m) return null;
  return { s: Number(m[1]), e: Number(m[2]) };
}

function fileLabel(basename, se = null) {
  se = se || parseSEFromName(basename);
  if (se) {
    const sLabel = `S${String(se.s).padStart(2, '0')}E${String(se.e).padStart(2, '0')}`;
    const rest = basename.replace(/[Ss]\d{1,2}[\s._-]*[Ee]\d{1,3}/, '').trim();
    const cleaned = rest
      .replace(/\.[a-z0-9]{1,4}$/i, '')
      .replace(/[._]+/g, ' ')
      .replace(/\s+/g, ' ')
      .trim()
      .slice(0, 70);
    return cleaned ? `${sLabel} — ${cleaned}` : sLabel;
  }
  return basename
    .replace(/\.[a-z0-9]{1,4}$/i, '')
    .replace(/[._]+/g, ' ')
    .slice(0, 80);
}

function pickTrailerUrl(detail) {
  const videos = detail?.videos?.results || [];
  const yt = videos.filter(v => v?.site === 'YouTube' && v?.key);
  if (!yt.length) return null;
  const trailers = yt.filter(v => v.type === 'Trailer');
  const pick = trailers.find(v => v.official)
    || trailers[0]
    || yt.find(v => v.type === 'Teaser')
    || yt[0];
  return pick ? `https://www.youtube.com/watch?v=${pick.key}` : null;
}

function renderDetails(item, detail, type) {
  const isTV = type === 'tv';
  const body = $('#detailsBody');

  const backdrop = (item.backdrop_path || detail.backdrop_path)
    ? `${TMDB_IMG}/w1280${item.backdrop_path || detail.backdrop_path}` : '';
  const poster = (item.poster_path || detail.poster_path)
    ? `${TMDB_IMG}/w500${item.poster_path || detail.poster_path}` : '';

  const title = item.title || item.name || detail.title || detail.name || '';
  const origTitle = isTV ? detail.original_name : detail.original_title;
  const showOrig = origTitle && origTitle !== title;

  let director = null, composer = null, dop = null;
  const writers = [];
  for (const c of detail.credits?.crew || []) {
    if (!isTV && !director && c.job === 'Director') director = c;
    else if (!composer && c.job === 'Original Music Composer') composer = c;
    else if (!isTV && !dop && c.job === 'Director of Photography') dop = c;
    else if (!isTV && writers.length < 3 &&
             (c.job === 'Screenplay' || c.job === 'Writer' || c.job === 'Story' || c.job === 'Author')) {
      writers.push(c);
    }
  }
  const creators = isTV ? (detail.created_by || []).slice(0, 4) : [];
  const networks = isTV ? (detail.networks || []).slice(0, 3) : [];

  const cast = (detail.credits?.cast || []).slice(0, 8);
  const genres = detail.genres || [];

  const dateLabel = isTV ? t('details.spec.firstAir') : t('details.spec.release');
  const tvDateValue = isTV && detail.last_air_date && detail.last_air_date !== detail.first_air_date
    ? `${fmtFullDate(detail.first_air_date)} – ${fmtFullDate(detail.last_air_date)}`
    : fmtFullDate(isTV ? detail.first_air_date : detail.release_date);

  const countries = ((isTV ? detail.origin_country : null) ||
    (detail.production_countries || []).map(c => c.iso_3166_1)).join(' · ');

  const tvSpecs = isTV ? `
    ${detail.number_of_seasons ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.seasons'))}</dt><dd>${detail.number_of_seasons}</dd></div>` : ''}
    ${detail.number_of_episodes ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.episodes'))}</dt><dd>${detail.number_of_episodes}</dd></div>` : ''}
    ${detail.episode_run_time?.length ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.episodeRuntime'))}</dt><dd>${detail.episode_run_time[0]} min</dd></div>` : ''}
    ${detail.status ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.status'))}</dt><dd>${escapeHTML(statusLabel(detail.status))}</dd></div>` : ''}
    ${creators.length ? `<div class="dossier-spec"><dt>${escapeHTML(creators.length > 1 ? t('details.spec.creators') : t('details.spec.creator'))}</dt><dd>${creators.map(c => escapeHTML(c.name)).join(', ')}</dd></div>` : ''}
    ${networks.length ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.network'))}</dt><dd>${networks.map(n => escapeHTML(n.name)).join(' · ')}</dd></div>` : ''}
  ` : '';

  const movieSpecs = !isTV ? `
    ${detail.runtime ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.runtime'))}</dt><dd>${fmtRuntime(detail.runtime)}</dd></div>` : ''}
    ${director ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.director'))}</dt><dd>${escapeHTML(director.name)}</dd></div>` : ''}
    ${writers.length ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.writers'))}</dt><dd>${writers.map(w => escapeHTML(w.name)).join(', ')}</dd></div>` : ''}
    ${dop ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.dop'))}</dt><dd>${escapeHTML(dop.name)}</dd></div>` : ''}
    ${detail.budget ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.budget'))}</dt><dd>${fmtMoney(detail.budget)}</dd></div>` : ''}
    ${detail.revenue ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.revenue'))}</dt><dd>${fmtMoney(detail.revenue)}</dd></div>` : ''}
  ` : '';

  const castHtml = cast.length ? `
    <div class="dossier-cast-block">
      <p class="section-eye">${escapeHTML(t('details.cast.title'))}</p>
      <div class="dossier-cast">
        ${cast.map(c => {
          const portrait = c.profile_path ? `${TMDB_IMG}/w185${c.profile_path}` : '';
          const hasId = Number.isFinite(c.id);
          const tag = hasId ? 'a' : 'figure';
          const attrs = hasId
            ? ` href="https://www.themoviedb.org/person/${c.id}" data-person-id="${c.id}" class="cast-link"`
            : '';
          return `
            <${tag}${attrs}>
              <div class="cast-portrait${portrait ? '' : ' is-empty'}">
                ${portrait ? `<img src="${escapeHTML(portrait)}" alt="" loading="lazy">` : ''}
              </div>
              <figcaption>
                ${escapeHTML(c.name || '')}
                ${c.character ? `<small>${escapeHTML(c.character)}</small>` : ''}
              </figcaption>
            </${tag}>`;
        }).join('')}
      </div>
    </div>` : '';

  const desktopHtml = `
    ${backdrop ? `<div class="dossier-hero"><img src="${escapeHTML(backdrop)}" alt="" loading="eager">${IS_PHONE ? `<h2 class="dossier-hero-title">${escapeHTML(title)}</h2>` : ''}</div>` : ''}
    <div class="dossier-content${backdrop ? '' : ' no-hero'}">
      <div class="dossier-head">
        <div class="dossier-poster">
          ${poster ? `<img src="${escapeHTML(poster)}" alt="">` : ''}
        </div>
        <div class="dossier-text">
          <h2 class="dossier-title" id="detailsTitle">
            ${escapeHTML(title)}
            ${detail.external_ids?.imdb_id ? `<span class="dossier-imdb">${escapeHTML(detail.external_ids.imdb_id)}</span>` : ''}
          </h2>
          ${showOrig ? `<p class="dossier-original">${escapeHTML(origTitle)}</p>` : ''}
          ${detail.tagline ? `<p class="dossier-tagline">${escapeHTML(detail.tagline)}</p>` : ''}

          <dl class="dossier-grid">
            <div class="dossier-spec"><dt>${escapeHTML(dateLabel)}</dt><dd>${tvDateValue}</dd></div>
            ${tvSpecs}
            ${movieSpecs}
            <div class="dossier-spec"><dt>${escapeHTML(t('details.spec.rating'))}</dt><dd>★ ${fmtVote(detail.vote_average)} <small>(${detail.vote_count || 0})</small></dd></div>
            <div class="dossier-spec"><dt>${escapeHTML(t('details.spec.language'))}</dt><dd>${escapeHTML((detail.original_language || '—').toUpperCase())}</dd></div>
            ${composer ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.music'))}</dt><dd>${escapeHTML(composer.name)}</dd></div>` : ''}
            ${countries ? `<div class="dossier-spec"><dt>${escapeHTML(t('details.spec.countries'))}</dt><dd>${escapeHTML(countries)}</dd></div>` : ''}
          </dl>

          ${genres.length ? `
            <div class="dossier-genres">
              ${genres.map(g => `<span>${escapeHTML(g.name)}</span>`).join('')}
            </div>` : ''}

          <div class="dossier-actions">
            ${state.detail?.downloadMode ? `
            <button class="play-btn" type="button" data-play>
              <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M8 5.14v13.72c0 .79.87 1.27 1.54.84l10.54-6.86a1 1 0 0 0 0-1.68L9.54 4.3A1 1 0 0 0 8 5.14Z"/></svg>
              <span>${escapeHTML(t('details.play'))}</span>
            </button>` : ''}
            ${state.detail?.downloadMode ? '' : (() => {
              const trailerUrl = pickTrailerUrl(detail);
              return trailerUrl ? `
                <button class="trailer-btn" type="button" data-trailer="${escapeHTML(trailerUrl)}" title="${escapeHTML(t('details.trailerTooltip'))}">
                  <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M21.6 7.2a2.5 2.5 0 0 0-1.76-1.77C18.28 5 12 5 12 5s-6.28 0-7.84.43A2.5 2.5 0 0 0 2.4 7.2 26 26 0 0 0 2 12a26 26 0 0 0 .4 4.8 2.5 2.5 0 0 0 1.76 1.77C5.72 19 12 19 12 19s6.28 0 7.84-.43a2.5 2.5 0 0 0 1.76-1.77A26 26 0 0 0 22 12a26 26 0 0 0-.4-4.8Z"/><path fill="#0c0a14" d="m10 15.5 5-3.5-5-3.5z"/></svg>
                  <span>${escapeHTML(t('details.trailer'))}</span>
                </button>` : '';
            })()}
            ${state.detail?.downloadMode ? '' : `
              <button class="fav-btn" type="button" data-fav aria-pressed="${isFavorite({ type, tmdbId: item.id ?? state.detail?.id }) ? 'true' : 'false'}" aria-label="${escapeHTML(t('details.fav.add'))}" title="${escapeHTML(t('details.fav.add'))}">
                <svg class="ico-heart-outline" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 21s-7.5-4.6-7.5-10.4A4.6 4.6 0 0 1 12 6a4.6 4.6 0 0 1 7.5 4.6C19.5 16.4 12 21 12 21Z"/></svg>
                <svg class="ico-heart-filled" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M12 21s-7.5-4.6-7.5-10.4A4.6 4.6 0 0 1 12 6a4.6 4.6 0 0 1 7.5 4.6C19.5 16.4 12 21 12 21Z"/></svg>
              </button>`}
          </div>
        </div>
      </div>
      ${detail.overview ? `<p class="dossier-overview">${escapeHTML(detail.overview)}</p>` : ''}
      ${castHtml}
      ${state.detail?.downloadMode ? '' : renderStreamsShell(detail, type)}
    </div>
  `;
  body.innerHTML = IS_PHONE
    ? mobileDetailsHtml({ item, detail, type, isTV, title, backdrop, director, writers, creators })
    : desktopHtml;

  state.detail.imdb = detail.external_ids?.imdb_id || null;
  state.detail.cast = detail.credits?.cast || [];
  state.detail.title = title;
  state.detail.isTV = isTV;
  state.detail.fullDetail = detail;
  state.detail.derived = {
    director, writers, dop, composer, creators, networks,
    countries,
    origTitle: showOrig ? origTitle : null,
    statusLabel: statusLabel(detail.status),
  };
  if (isTV) {
    state.detail.seasons = (detail.seasons || []).filter(s => s.season_number > 0);
  }

  body.querySelectorAll('a.cast-link').forEach(el => {
    el.addEventListener('click', e => {
      e.preventDefault();
      openExternal(el.getAttribute('href'));
    });
  });

  const imdbBadge = body.querySelector('.dossier-imdb');
  if (imdbBadge) {
    imdbBadge.title = t('details.imdb.copyTooltip');
    imdbBadge.addEventListener('click', async (e) => {
      e.preventDefault();
      if (imdbBadge.classList.contains('is-copied')) return;
      const id = imdbBadge.textContent.trim();
      if (!id) return;
      if (await copyToClipboard(id)) {
        const orig = imdbBadge.textContent;
        imdbBadge.classList.add('is-copied');
        imdbBadge.innerHTML = `<svg class="hint-icon" viewBox="0 0 24 24" width="1em" height="1em" aria-hidden="true"><circle cx="12" cy="12" r="10" fill="currentColor" opacity="0.18"/><path d="m7 12.5 3.2 3.2L17 9" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg><span>${escapeHTML(t('details.imdb.copied'))}</span>`;
        setTimeout(() => {
          imdbBadge.classList.remove('is-copied');
          imdbBadge.textContent = orig;
        }, 1100);
      }
    });
  }

  if (state.detail?.downloadMode) {
    const dlId = state.detail.downloadId;
    body.querySelector('[data-play]')?.addEventListener('click', () => {
      playDownload(dlId, title);
    });
    return;
  }

  body.querySelector('[data-fav]')?.addEventListener('click', (e) => {
    const btn = e.currentTarget;
    const tmdbId = item.id ?? state.detail?.id;
    if (tmdbId == null) return;
    const nowFav = toggleFavorite({ type, tmdbId });
    btn.setAttribute('aria-pressed', nowFav ? 'true' : 'false');
    btn.setAttribute(
      'aria-label',
      nowFav ? t('details.fav.remove') : t('details.fav.add'),
    );
    btn.title = nowFav ? t('details.fav.remove') : t('details.fav.add');
  });

  body.querySelector('[data-trailer]')?.addEventListener('click', (e) => {
    const url = e.currentTarget.getAttribute('data-trailer');
    if (url) openExternal(url);
  });

  bindStreamsSection(detail, type);
}

function renderStreamsShell(detail, type) {
  const imdb = detail.external_ids?.imdb_id;
  const isTV = type === 'tv';

  if (!imdb) {
    return shellEmpty(t('details.noImdb'));
  }

  if (isTV) {
    const seasons = (detail.seasons || []).filter(s => s.season_number > 0);
    if (!seasons.length) return shellEmpty(t('details.noSeasons'));
    state.detail.seasons = seasons;
    return `
      <div class="streams" data-streams data-imdb="${escapeHTML(imdb)}" data-type="series" data-tv-id="${detail.id}">
        <div class="streams-picker">
          ${streamPickerHtml('season', t('details.picker.season'))}
          ${streamPickerHtml('episode', t('details.picker.episode'))}
        </div>
        <header class="streams-head">
          <p class="section-eye">${escapeHTML(t('details.streams.title'))}</p>
          <span class="streams-count" data-stream-count></span>
          <div class="streams-filter-slot" data-stream-filter></div>
        </header>
        <div class="streams-body" data-stream-body>
          <div class="streams-loading">${DOTS_HTML}</div>
        </div>
      </div>`;
  }

  return `
    <div class="streams" data-streams data-imdb="${escapeHTML(imdb)}" data-type="movie">
      <header class="streams-head">
        <p class="section-eye">${escapeHTML(t('details.streams.title'))}</p>
        <span class="streams-count" data-stream-count></span>
        <div class="streams-filter-slot" data-stream-filter></div>
      </header>
      <div class="streams-body" data-stream-body>
        <div class="streams-loading">${DOTS_HTML}</div>
      </div>
    </div>`;
}

function shellEmpty(msg) {
  return `
    <div class="streams" data-streams>
      <header class="streams-head"><p class="section-eye">${escapeHTML(t('details.streams.title'))}</p></header>
      <p class="streams-empty">${escapeHTML(msg)}</p>
    </div>`;
}

async function bindStreamsSection(detail, type) {
  const root = detailsModal.querySelector('[data-streams]');
  if (!root || !root.dataset.imdb) return;

  const isTV = type === 'tv';
  const imdb = root.dataset.imdb;

  if (!isTV) {
    loadStreams(root, 'movie', imdb);
    return;
  }

  const seasonRoot = root.querySelector('[data-stream-pick="season"]');
  const epRoot = root.querySelector('[data-stream-pick="episode"]');
  const tvId = Number(root.dataset.tvId);

  const seasonItems = (state.detail.seasons || []).map(s => ({
    value: s.season_number,
    label: s.name && !/^Stagione\s+\d+$/i.test(s.name)
      ? `${s.season_number} · ${s.name}`
      : t('details.seasonN', { n: s.season_number }),
  }));

  // A series opens on the episode watched last (played, or in Continue watching).
  const last = lastEpisode(tvId);
  const startSeason = last && seasonItems.some(i => i.value === last.season)
    ? last.season
    : seasonItems[0]?.value;

  const seasonPicker = setupStreamPicker(seasonRoot, {
    items: seasonItems,
    value: startSeason,
    popup: IS_PHONE,
    onChange: v => loadSeason(Number(v)),
  });

  const epPicker = setupStreamPicker(epRoot, {
    items: [{ value: '', label: t('details.loadingShort') }],
    value: '',
    popup: IS_PHONE,
    onChange: () => triggerEpisode(),
  });

  state.detail.seasonPicker = seasonPicker;
  state.detail.epPicker = epPicker;

  async function loadSeason(seasonNumber) {
    epPicker.setItems([{ value: '', label: t('details.loadingShort') }]);
    epPicker.setValue('', false);
    try {
      const data = await fetchTvSeason(tvId, seasonNumber);
      const items = (data.episodes || []).map(ep => ({
        value: ep.episode_number,
        label: `E${String(ep.episode_number).padStart(2, '0')} · ${ep.name || ''}`,
      }));
      if (!items.length) {
        epPicker.setItems([{ value: '', label: t('details.noEpisodesShort') }]);
        epPicker.setValue('', false);
        renderStreamsEmpty(root, t('details.noEpisodesSeason'));
        return;
      }
      epPicker.setItems(items);
      const startEpisode = last && seasonNumber === last.season && items.some(i => i.value === last.episode)
        ? last.episode
        : items[0].value;
      epPicker.setValue(startEpisode, false);
      triggerEpisode();
    } catch (e) {
      epPicker.setItems([{ value: '', label: t('details.errorShort') }]);
      epPicker.setValue('', false);
      renderStreamsEmpty(root, t('details.errorMsg', { error: e.message }));
    }
  }

  function triggerEpisode() {
    const s = Number(seasonPicker.getValue());
    const e = Number(epPicker.getValue());
    if (!Number.isFinite(s) || !Number.isFinite(e)) return;
    loadStreams(root, 'series', `${imdb}:${s}:${e}`);
  }

  await loadSeason(Number(seasonPicker.getValue()));
}

async function loadStreams(root, type, id) {
  const body = root.querySelector('[data-stream-body]');
  const counter = root.querySelector('[data-stream-count]');
  const filterSlot = root.querySelector('[data-stream-filter]');
  if (!body) return;
  body._streamsAbort?.abort();
  body._streamsAbort = new AbortController();
  const myGen = state.detailGen;
  body.innerHTML = `<div class="streams-loading">${DOTS_HTML}</div>`;
  if (counter) counter.textContent = '';
  if (filterSlot) filterSlot.innerHTML = '';

  try {
    const data = await fetchStreams(type, id);
    if (myGen !== state.detailGen) return;
    const streams = data.streams || [];

    if (!streams.length) {
      const hasAddons = (state.settings.addons || []).some(a => a.enabled !== false);
      if (!hasAddons) {
        body.innerHTML = `
          <p class="streams-empty">
            ${t('details.noAddons', { settings: '<a href="#" data-go-addons>' + escapeHTML(t('details.noAddonsLink')) + '</a>' })}
          </p>`;
        body.querySelector('[data-go-addons]')?.addEventListener('click', e => {
          e.preventDefault();
          closeModal('#detailsModal');
          openSettings('addons');
        });
      } else {
        body.innerHTML = `<p class="streams-empty">${escapeHTML(t('details.noStreams'))}</p>`;
      }
      return;
    }

    const enriched = streams.map(s => ({ s, meta: parseStreamMeta(s) }));
    // What a TV cannot decode in hardware (4K, AV1 on a 1080p Fire TV Stick)
    // it would decode in software, too slowly, until it runs out of memory:
    // those streams go last, and ask before playing.
    const caps = await videoCaps;
    if (myGen !== state.detailGen) return;
    const no4k = !!caps && !caps.hevc4k && !caps.avc4k;
    const noAv1 = !!caps && caps.av1 === false;
    const unsupported = e => (no4k && e.meta.q.rank === 4 ? 'no4k' : noAv1 && e.meta.av1 ? 'noAv1' : null);
    const rank = e => (unsupported(e) ? -1 : e.meta.q.rank);
    enriched.sort((a, b) => rank(b) - rank(a));
    if (counter) counter.textContent = enriched.length === 1
      ? t('details.resultsOne', { n: enriched.length })
      : t('details.resultsMany', { n: enriched.length });

    try {
      const downloads = await downloadList();
      if (myGen !== state.detailGen) return;
      const downloadedIds = new Set((downloads || []).map(d => d.id));
      await Promise.all(enriched.map(async (e) => {
        const dlId = await streamDownloadId(e.s);
        if (!dlId) return;
        e.s._dlId = dlId;
        e.s._isDownloaded = downloadedIds.has(dlId);
      }));
      if (myGen !== state.detailGen) return;
    } catch (err) {
      console.warn('downloaded-state hydration failed', err);
    }

    // The stream this movie or episode was last played from.
    const lastKey = getWatchedStream(buildPlayCtx(type, id));
    const keys = lastKey ? await Promise.all(enriched.map(e => streamWatchKey(e.s))) : [];
    if (myGen !== state.detailGen) return;
    enriched.forEach((e, i) => { e.s._watched = !!lastKey && keys[i] === lastKey; });

    const addonCounts = new Map();
    const resCounts = new Map();
    for (const { s, meta } of enriched) {
      const k = s._addon || '—';
      addonCounts.set(k, (addonCounts.get(k) || 0) + 1);
      const r = meta.q.label;
      resCounts.set(r, (resCounts.get(r) || 0) + 1);
    }
    const addonNames = [...addonCounts.keys()];
    const RES_ORDER = ['2160p', '1080p', '720p', '480p', 'SD'];
    const resLabels = RES_ORDER.filter(r => resCounts.has(r));
    let activeAddon = null;
    let activeResolution = null;
    let resPicker = null;
    let addonPicker = null;
    let sortBy = 'quality';

    body.innerHTML = `<ul class="stream-list" data-stream-list></ul>`;
    const listEl = body.querySelector('[data-stream-list]');

    let listGen = 0;
    const renderList = () => {
      const items = enriched
        .map((e, i) => ({ e, i }))
        .filter(({ e }) => !activeAddon || (e.s._addon || '—') === activeAddon)
        .filter(({ e }) => !activeResolution || e.meta.q.label === activeResolution);
      if (sortBy === 'seeders') {
        items.sort((a, b) => seedersNum(b.e.meta.seeders) - seedersNum(a.e.meta.seeders));
      }
      // The stream last played from leads the list.
      items.sort((a, b) => Number(!!b.e.s._watched) - Number(!!a.e.s._watched));
      const html = items.map(({ e, i }) => streamItemHtml(e.s, e.meta, i));
      // A TV lays a long list out slowly (a Fire TV Stick takes half a second
      // over ninety streams, with the remote waiting): the first streams show
      // at once and the others follow a batch at a time, each once the frame
      // before it is drawn.
      const batch = IS_TV ? STREAM_BATCH : html.length;
      const gen = ++listGen;
      listEl.innerHTML = html.slice(0, batch).join('');
      const more = (from) => {
        if (from >= html.length) return;
        requestAnimationFrame(() => setTimeout(() => {
          if (gen !== listGen || myGen !== state.detailGen || !listEl.isConnected) return;
          listEl.insertAdjacentHTML('beforeend', html.slice(from, from + batch).join(''));
          more(from + batch);
        }, 0));
      };
      more(batch);
    };

    const refreshFilters = () => {
      let resTotal = 0;
      const resLive = new Map();
      for (const { s, meta } of enriched) {
        if (activeAddon && (s._addon || '—') !== activeAddon) continue;
        resLive.set(meta.q.label, (resLive.get(meta.q.label) || 0) + 1);
        resTotal++;
      }
      if (resPicker) {
        const items = [{ value: '', label: t('details.filter.allRes', { n: resTotal }) }]
          .concat(RES_ORDER
            .filter(r => resLive.get(r))
            .map(r => ({ value: r, label: `${r} (${resLive.get(r)})` })));
        resPicker.setItems(items);
        if (activeResolution && !resLive.get(activeResolution)) {
          activeResolution = null;
          resPicker.setValue('', false);
        }
      }

      let addonTotal = 0;
      const addonLive = new Map();
      for (const { s, meta } of enriched) {
        if (activeResolution && meta.q.label !== activeResolution) continue;
        const k = s._addon || '—';
        addonLive.set(k, (addonLive.get(k) || 0) + 1);
        addonTotal++;
      }
      if (addonPicker) {
        const items = addonNames.length >= 2
          ? [{ value: '', label: t('details.filter.allAddons', { n: addonTotal }) }]
              .concat(addonNames
                .filter(n => addonLive.get(n))
                .map(n => ({ value: n, label: `${n} (${addonLive.get(n)})` })))
          : [{ value: '', label: `${addonNames[0] || t('details.filter.allAddonsBare')} (${addonTotal})` }];
        addonPicker.setItems(items);
        if (activeAddon && !addonLive.get(activeAddon)) {
          activeAddon = null;
          addonPicker.setValue('', false);
        }
      }
    };

    if (filterSlot) {
      if (counter) counter.textContent = '';
      const showResFilter = resLabels.length >= 2;
      filterSlot.innerHTML = `
        ${streamPickerHtml('sort', '', 'streams-pick-compact')}
        ${showResFilter ? streamPickerHtml('resolution', '', 'streams-pick-compact') : ''}
        ${streamPickerHtml('addon', '', 'streams-pick-compact')}
      `;

      const sortRoot = filterSlot.querySelector('[data-stream-pick="sort"]');
      setupStreamPicker(sortRoot, {
        items: [
          { value: 'quality', label: t('details.sort.quality') },
          { value: 'seeders', label: t('details.sort.seeders') },
        ],
        value: 'quality',
        popup: IS_PHONE,
        onChange: v => { sortBy = v || 'quality'; renderList(); },
      });

      if (showResFilter) {
        const resRoot = filterSlot.querySelector('[data-stream-pick="resolution"]');
        const resItems = [{ value: '', label: t('details.filter.allRes', { n: enriched.length }) }]
          .concat(resLabels.map(r => ({ value: r, label: `${r} (${resCounts.get(r)})` })));
        resPicker = setupStreamPicker(resRoot, {
          items: resItems,
          value: '',
          popup: IS_PHONE,
          onChange: v => { activeResolution = v || null; refreshFilters(); renderList(); },
        });
      }

      const pickerRoot = filterSlot.querySelector('[data-stream-pick="addon"]');
      const pickerItems = addonNames.length >= 2
        ? [{ value: '', label: t('details.filter.allAddons', { n: enriched.length }) }]
            .concat(addonNames.map(n => ({ value: n, label: `${n} (${addonCounts.get(n)})` })))
        : [{ value: '', label: `${addonNames[0] || t('details.filter.allAddonsBare')} (${enriched.length})` }];
      addonPicker = setupStreamPicker(pickerRoot, {
        items: pickerItems,
        value: '',
        popup: IS_PHONE,
        onChange: v => { activeAddon = v || null; refreshFilters(); renderList(); },
      });
    }

    renderList();

    body.addEventListener('click', async (e) => {
      const copyBtn = e.target.closest('[data-copy]');
      if (copyBtn) {
        const ok = await copyToClipboard(copyBtn.dataset.copy);
        copyBtn.dataset.state = ok ? 'ok' : 'err';
        const orig = copyBtn.innerHTML;
        copyBtn.innerHTML = ok ? '✓' : '✕';
        setTimeout(() => { delete copyBtn.dataset.state; copyBtn.innerHTML = orig; }, 1400);
        return;
      }
      const playBtn = e.target.closest('[data-rd-play]');
      if (playBtn) {
        const entry = enriched[Number(playBtn.dataset.rdPlay)];
        const why = entry && unsupported(entry);
        if (why) {
          const go = await showConfirm(t(`details.stream.${why}Body`), {
            title: t(`details.stream.${why}Title`),
            okLabel: t('details.stream.playAnyway'),
            focusCancel: true,
          });
          if (!go || myGen !== state.detailGen) return;
        }
        if (entry) {
          const ctx = buildPlayCtx(type, id, entry.s._addon);
          startRdPlayback(entry.s, entry.meta, playBtn, ctx).then(played => {
            if (!played || myGen !== state.detailGen) return;
            for (const x of enriched) x.s._watched = x === entry;
            renderList();
          });
        }
        return;
      }
      const dlBtn = e.target.closest('[data-download]');
      if (dlBtn) {
        const entry = enriched[Number(dlBtn.dataset.download)];
        if (entry && (entry.s.url || entry.s.infoHash) && !entry.s._isDownloaded) {
          const item = state.detail?.item || {};
          const posterPath = item.poster_path;
          const title = item.title || item.name || entry.meta.titleLine || '';
          dlBtn.disabled = true;
          dlBtn.innerHTML = '✓';

          const seParts = type === 'series' ? String(id).split(':') : [];
          const dlSeason = seParts.length === 3 ? Number(seParts[1]) : NaN;
          const dlEpisode = seParts.length === 3 ? Number(seParts[2]) : NaN;

          downloadStart({
            url: entry.s.url || null,
            infoHash: entry.s.url ? null : entry.s.infoHash,
            title,
            posterUrl: posterPath ? `${TMDB_IMG}/w500${posterPath}` : null,
            addon: entry.s._addon || null,
            sources: entry.s.sources || [],
            fileHint: entry.s.behaviorHints?.filename || '',
            tmdbId: state.detail?.id || null,
            tmdbType: state.detail?.type === 'tv' ? 'tv' : 'movie',
            season: Number.isFinite(dlSeason) ? dlSeason : null,
            episode: Number.isFinite(dlEpisode) ? dlEpisode : null,
          })
            .then(() => {
              entry.s._isDownloaded = true;
              renderList();
            })
            .catch(err => {
              console.warn('downloadStart failed', err);
              dlBtn.innerHTML = '✕';
              setTimeout(() => {
                dlBtn.innerHTML = DOWNLOAD_ICON;
                dlBtn.disabled = false;
              }, 1500);
            });
        }
      }
    }, { signal: body._streamsAbort.signal });
  } catch (e) {
    if (myGen !== state.detailGen) return;
    const msg = (e && e.message) || (typeof e === 'string' ? e : String(e));
    body.innerHTML = `<p class="streams-empty">${escapeHTML(t('details.networkError', { error: msg }))}</p>`;
  }
}

function streamItemHtml(s, meta, idx) {
  const { q, titleLine, seeders, size, source, lang, rd } = meta;
  const link = streamLink(s);
  const safeUrl = escapeHTML(link);
  const safeTitle = escapeHTML(titleLine || '—');
  const addon = escapeHTML(s._addon || '');

  const stat = (val, mod) => val
    ? `<span class="stream-stat stream-stat--${mod}">${escapeHTML(val)}</span>`
    : '';
  const stats = [stat(seeders, 'seed'), stat(size, 'size'), stat(source, 'src'), stat(lang, 'lang')]
    .filter(Boolean).join('');

  const canPlay = !!(s.url || s.infoHash);
  const playBtn = canPlay
    ? `<button type="button" class="stream-btn stream-btn--play" data-rd-play="${idx}" aria-label="${escapeHTML(t('details.stream.playAria'))}" title="${escapeHTML(t('details.stream.playTitle'))}">${PLAY_ICON}<span>${escapeHTML(t('details.stream.play'))}</span></button>`
    : '';

  const copyBtn = link
    ? `<button type="button" class="stream-btn stream-btn--icon" data-copy="${safeUrl}" aria-label="${escapeHTML(t('details.stream.copyLink'))}" title="${escapeHTML(t('details.stream.copyLink'))}">${COPY_SVG}</button>`
    : `<button type="button" class="stream-btn stream-btn--icon" disabled aria-label="${escapeHTML(t('details.stream.linkUnavailable'))}">—</button>`;

  const downloadBtn = (s.infoHash || s.url)
    ? (s._isDownloaded
        ? `<button type="button" class="stream-btn stream-btn--icon is-done" disabled aria-label="${escapeHTML(t('details.stream.downloaded'))}" title="${escapeHTML(t('details.stream.downloadedTooltip'))}">${CHECK_SVG}</button>`
        : `<button type="button" class="stream-btn stream-btn--icon" data-download="${idx}" aria-label="${escapeHTML(t('details.stream.download'))}" title="${escapeHTML(t('details.stream.download'))}">${DOWNLOAD_ICON}</button>`)
    : '';

  const rdBadge = rd
    ? `<span class="stream-rd stream-rd--${rd.state}" title="${escapeHTML(rd.state === 'cached' ? t('details.rd.cached') : rd.state === 'download' ? t('details.rd.download') : t('details.rd.plain'))}">${escapeHTML(rd.label)}</span>`
    : '';
  const watchedBadge = s._watched
    ? `<span class="stream-watched">${escapeHTML(t('details.stream.lastWatched'))}</span>`
    : '';

  return `
    <li class="stream${s._watched ? ' is-watched' : ''}">
      <div class="stream-info">
        <div class="stream-meta">
          <span class="stream-q ${escapeHTML(q.cls || '')}">${escapeHTML(q.label)}</span>
          ${watchedBadge}
          ${rdBadge}
          ${stats ? `<div class="stream-stats">${stats}</div>` : ''}
        </div>
        <p class="stream-title">${safeTitle}</p>
        ${addon ? `<p class="stream-source">${addon}</p>` : ''}
      </div>
      <div class="stream-actions">${playBtn}${downloadBtn}${copyBtn}</div>
    </li>`;
}

function buildPlayCtx(type, id, addonName) {
  if (type === 'series') {
    const [imdb, s, e] = id.split(':');
    return {
      type: 'tv',
      tmdbId: state.detail?.id ?? null,
      imdb,
      season: Number(s),
      episode: Number(e),
      addon: addonName || null,
      seasons: state.detail?.seasons || [],
    };
  }
  return {
    type: 'movie',
    tmdbId: state.detail?.id ?? null,
    imdb: state.detail?.imdb || null,
  };
}

async function runPlayback(stream, meta, ctx) {
  const titleLine = meta.titleLine || stream.name || t('details.playback.title');
  const eyebrow = meta.q?.label
    ? `${meta.q.label}${meta.size ? ' · ' + meta.size : ''}`
    : t('details.playback.streamEyebrow');
  const eyebrowKey = meta.q?.label ? null : 'details.playback.streamEyebrow';

  const torrentForLoading = !state.settings.rdAvailable && stream.infoHash && !stream.url
    ? { infoHash: String(stream.infoHash).toLowerCase() }
    : null;
  openPlayerLoading({ title: titleLine, eyebrow, eyebrowKey, torrent: torrentForLoading });
  pushPlayerLog(t('details.playback.starting'));

  const ctl = new AbortController();
  setPlayerAbortController(ctl);

  let unlistenProgress = null;
  try {
    unlistenProgress = await onMediaProgress(msg => {
      if (typeof msg === 'string' && msg) pushPlayerLog(locMsg(msg));
    });
  } catch {}

  try {
    const rawSources = Array.isArray(stream.sources) ? stream.sources : [];
    const tmdbTitle = state.detail?.item?.title || state.detail?.item?.name || '';
    const folderName = (tmdbTitle || titleLine).slice(0, 200);
    const resp = await rdPlay(
      {
        url: stream.infoHash ? undefined : (stream.url || undefined),
        infoHash: stream.infoHash || undefined,
        displayName: folderName,
        fileHint: stream.behaviorHints?.filename || '',
        sources: rawSources.length ? rawSources : undefined,
        addon: stream._addon || undefined,
      },
      { signal: ctl.signal },
    );

    if (ctl.signal.aborted) {
      pushPlayerLog(t('details.playback.cancelled'));
      if (resp?.infoHash) {
        try { await destroyTorrentSession(resp.infoHash); } catch {}
      }
      return;
    }

    pushPlayerLog(t('details.playback.ready'));

    const torrentMeta = resp.infoHash ? { infoHash: resp.infoHash } : null;

    await attachToOpenPlayer(resp.url, {
      probe: resp.probe,
      torrent: torrentMeta,
      ctx,
    });
    streamWatchKey(stream).then(key => saveWatchedStream(ctx, key)).catch(() => {});
    return true;
  } catch (e) {
    if (e?.name === 'AbortError' || ctl.signal.aborted) return;
    const msg = locMsg((e && e.message) || (typeof e === 'string' ? e : String(e)));
    showPlayerError(t('details.errorMsg', { error: msg }));
  } finally {
    setPlayerAbortController(null);
    if (unlistenProgress) {
      try { unlistenProgress(); } catch {}
    }
  }
}

async function startRdPlayback(stream, meta, btn, ctx) {
  const orig = btn.innerHTML;
  btn.disabled = true;
  btn.innerHTML = '…';
  let done = false;
  const reset = () => {
    if (done) return;
    done = true;
    btn.disabled = false;
    btn.innerHTML = orig;
  };
  const onClosed = () => reset();
  window.addEventListener('siiis:player-closed', onClosed, { once: true });
  try {
    return await runPlayback(stream, meta, ctx);
  } finally {
    window.removeEventListener('siiis:player-closed', onClosed);
    reset();
  }
}

function renderStreamsEmpty(root, msg) {
  const body = root.querySelector('[data-stream-body]');
  const counter = root.querySelector('[data-stream-count]');
  if (body) body.innerHTML = `<p class="streams-empty">${escapeHTML(msg)}</p>`;
  if (counter) counter.textContent = '';
}

// Android details: the same data Stremio shows (cover with the title logo,
// runtime · year · rating, overview, genres, director, cast, writers) laid
// out in the app's own style. The streams shell is the shared one.
function mobileDetailsHtml({ item, detail, type, isTV, title, backdrop, director, writers, creators }) {
  const ui = (tmdbLang() || 'en').split('-')[0];
  const logos = detail.images?.logos || [];
  const logo = logos.find(l => l.iso_639_1 === ui)
    || logos.find(l => l.iso_639_1 === 'en')
    || logos.find(l => !l.iso_639_1)
    || null;
  const logoUrl = logo ? `${TMDB_IMG}/w500${logo.file_path}` : '';

  const year = ((isTV ? detail.first_air_date : detail.release_date) || '').slice(0, 4);
  const runtime = isTV
    ? (detail.episode_run_time?.[0] ? `${detail.episode_run_time[0]} min` : '')
    : (detail.runtime ? fmtRuntime(detail.runtime) : '');
  const episodes = isTV && detail.number_of_episodes
    ? `${detail.number_of_episodes} ${t('details.spec.episodes').toLowerCase()}` : '';
  const facts = [runtime, year, episodes].filter(Boolean);
  const vote = Number(detail.vote_average) > 0 ? `★ ${fmtVote(detail.vote_average)}` : '';

  const downloadMode = !!state.detail?.downloadMode;
  const favOn = !downloadMode && isFavorite({ type, tmdbId: item.id ?? state.detail?.id });
  const cast = (detail.credits?.cast || []).slice(0, 12);
  const genres = detail.genres || [];

  const chips = (people, labelKey) => {
    if (!people.length) return '';
    const items = people.map(p => Number.isFinite(p.id)
      ? `<a class="m-chip cast-link" href="https://www.themoviedb.org/person/${p.id}" data-person-id="${p.id}">${escapeHTML(p.name || '')}</a>`
      : `<span class="m-chip">${escapeHTML(p.name || '')}</span>`).join('');
    return `
      <section class="m-people">
        <p class="section-eye">${escapeHTML(t(labelKey))}</p>
        <div class="m-chips">${items}</div>
      </section>`;
  };

  const heroTitle = logoUrl
    ? `<h2 class="dossier-hero-title has-logo" id="detailsTitle"><img class="dossier-hero-logo" src="${escapeHTML(logoUrl)}" alt="${escapeHTML(title)}"></h2>`
    : `<h2 class="dossier-hero-title" id="detailsTitle">${escapeHTML(title)}</h2>`;

  return `
    ${backdrop
      ? `<div class="dossier-hero"><img src="${escapeHTML(backdrop)}" alt="" loading="eager">${heroTitle}</div>`
      : `<h2 class="dossier-title m-title" id="detailsTitle">${escapeHTML(title)}</h2>`}
    <div class="dossier-content m-dossier${backdrop ? '' : ' no-hero'}">
      <div class="m-meta">
        <div class="m-meta-facts">
          ${facts.map(f => `<span>${escapeHTML(f)}</span>`).join('<i class="m-dot"></i>')}
          ${vote ? `${facts.length ? '<i class="m-dot"></i>' : ''}<span class="m-vote">${escapeHTML(vote)}</span>` : ''}
        </div>
        <div class="m-meta-actions">
          ${downloadMode ? `
            <button class="play-btn" type="button" data-play>
              <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M8 5.14v13.72c0 .79.87 1.27 1.54.84l10.54-6.86a1 1 0 0 0 0-1.68L9.54 4.3A1 1 0 0 0 8 5.14Z"/></svg>
              <span>${escapeHTML(t('details.play'))}</span>
            </button>` : ''}
          ${downloadMode ? '' : `
            <button class="m-iconbtn fav-btn" type="button" data-fav aria-pressed="${favOn ? 'true' : 'false'}" aria-label="${escapeHTML(favOn ? t('details.fav.remove') : t('details.fav.add'))}">
              <svg class="ico-heart-outline" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 21s-7.5-4.6-7.5-10.4A4.6 4.6 0 0 1 12 6a4.6 4.6 0 0 1 7.5 4.6C19.5 16.4 12 21 12 21Z"/></svg>
              <svg class="ico-heart-filled" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path fill="currentColor" d="M12 21s-7.5-4.6-7.5-10.4A4.6 4.6 0 0 1 12 6a4.6 4.6 0 0 1 7.5 4.6C19.5 16.4 12 21 12 21Z"/></svg>
            </button>`}
        </div>
      </div>
      ${detail.overview ? `<p class="dossier-overview">${escapeHTML(detail.overview)}</p>` : ''}
      ${genres.length ? `<div class="dossier-genres">${genres.map(g => `<span>${escapeHTML(g.name)}</span>`).join('')}</div>` : ''}
      ${isTV
        ? chips(creators, creators.length > 1 ? 'details.spec.creators' : 'details.spec.creator')
        : chips(director ? [director] : [], 'details.spec.director')}
      ${chips(cast, 'player.sidepane.cast')}
      ${chips(writers, 'details.spec.writers')}
      ${downloadMode ? '' : renderStreamsShell(detail, type)}
    </div>`;
}
