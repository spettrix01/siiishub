import { $, $$, escapeHTML, FORM_TAGS, openExternal } from './dom.js';
import { IS_ANDROID } from './platform.js';
import { t as tx } from './i18n.js';
import { fmtTime, fmtBytes, fmtSpeed, fmtFullDate, fmtMoney, fmtRuntime, fmtVote } from './format.js';
import {
  destroyTorrentSession,
  fetchTorrentStats,
  fetchSubtitles,
  mpvLoad,
  mpvCommand,
  mpvSet,
  mpvObserve,
  mpvSetGeometry,
  mpvSetVisible,
  windowSetFullscreen,
  onMpvEvent,
  remotePushState,
  onRemoteCmd,
} from './api.js';
import { saveResume, getResume } from './resume.js';
import { state } from './state.js';
import { spatialNav } from './spatial-nav.js';

const TAURI = window.__TAURI__;

const playerModal = $('#playerModal');
const playerVideoBox = $('#playerVideo');
const playerStatus = $('#playerStatus');
const playerLogPane = $('#playerLogPane');
const playerLogBody = $('#playerLogBody');
const playerLogClose = $('#playerLogClose');
const playerEyebrow = $('#playerEyebrow');
const playerTitleEl = $('#playerTitle');
const playerPickers = $('#playerPickers');
const playerFrame = $('#playerFrame');
const playerTopEl = $('#playerTop');
const playerControlsEl = $('#playerControls');
const playerTorrentStats = $('#playerTorrentStats');

const playerBigPlay = $('#playerBigPlay');
const playerPlayBtn = $('#playerPlayBtn');
const playerMuteBtn = $('#playerMuteBtn');
const playerVol = $('#playerVol');
const playerVolPct = $('#playerVolPct');
const playerCurTime = $('#playerCurTime');
const playerTotalTime = $('#playerTotalTime');
const playerProgress = $('#playerProgress');
const playerProgressBuffered = $('#playerProgressBuffered');
const playerProgressPlayed = $('#playerProgressPlayed');
const playerProgressThumb = $('#playerProgressThumb');
const playerProgressTip = $('#playerProgressTip');
const playerSettingsBtn = $('#playerSettingsBtn');
const playerSettings = $('#playerSettings');
const playerSettingsBody = $('#playerSettingsBody');
const playerFsBtn = $('#playerFsBtn');
const playerSplitBtn = $('#playerSplitBtn');
const playerSidePane = $('#playerSidePane');
const playerSidePaneTitle = $('#playerSidePaneTitle');
const playerSidePaneBody = $('#playerSidePaneBody');

const detailsModal = $('#detailsModal');
const settingsModal = $('#settingsModal');

let playerCtx = null;
let playerTorrent = null;
let playerTorrentTimer = null;
let playerOnEnded = null;
let playerLogs = [];
let playerHlsUrl = null;
let mpvGeometryObs = null;
let mpvGeometryRaf = 0;

let pendingResumeTime = 0;

const mpvState = {
  paused: true,
  duration: 0,
  timePos: 0,
  cacheEnd: 0,
  volume: 100,
  mute: false,
  trackList: [],
  aid: null,
  sid: null,
  speed: 1,
  subDelay: 0,
  fullscreen: false,
  seeking: false,
};

const SPEED_PRESETS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];
const VOLUME_MAX = 200;
let observersInstalled = false;
let mpvUnlistenPromise = null;
let resumeSaveTimer = 0;

let playerFailed = false;
let logPaneOpen = false;

function plClock() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function friendlyReason(msg) {
  const s = String(msg || '');
  if (/add_torrent failed|error opening/i.test(s)) return tx('player.reason.invalidFilename');
  if (/HTTP\s*5\d\d|bad gateway|gateway time/i.test(s)) return tx('player.reason.sourceUnreachable');
  if (/no peers|nessun peer|noPeers/i.test(s)) return tx('player.reason.noPeers');
  return s;
}

function renderPlayerStatus() {
  playerStatus.hidden = false;
  playerStatus.classList.remove('player-status--log');
  const last = playerLogs[playerLogs.length - 1];
  const summary = playerFailed
    ? friendlyReason(last && last.text)
    : (last ? last.text : '');
  playerStatus.innerHTML = `
    <div class="pl-box${playerFailed ? ' is-error' : ''}">
      <div class="pl-icon" aria-hidden="true"></div>
      <div class="pl-summary">${escapeHTML(summary)}</div>
      <button class="pl-logbtn" type="button" data-pl-logtoggle>${escapeHTML(logPaneOpen ? tx('player.log.hide') : tx('player.log.show'))}</button>
    </div>`;
}

function renderLogPane() {
  if (!playerLogPane) return;
  playerFrame.classList.toggle('is-logopen', logPaneOpen);
  if (!logPaneOpen) return;
  const total = playerLogs.length;
  const lines = playerLogs.map((l, i) => {
    const isLast = i === total - 1;
    let cls, ico;
    if (l.kind === 'error') { cls = 'is-error'; ico = '✕'; }
    else if (isLast && !playerFailed) { cls = 'is-current'; ico = '●'; }
    else { cls = 'is-done'; ico = '✓'; }
    return `<div class="plp-line ${cls}"><span class="plp-ico">${ico}</span><span class="plp-time">${escapeHTML(l.time)}</span><span class="plp-text">${escapeHTML(l.text)}</span></div>`;
  }).join('');
  const last = playerLogs[total - 1];
  const outcome = playerFailed
    ? `<div class="plp-outcome is-error">${escapeHTML(tx('player.log.failed', { reason: friendlyReason(last && last.text) }))}</div>`
    : '';
  playerLogBody.innerHTML = lines + outcome;
}

function setPlayerStatus(msg) {
  playerLogs = [];
  playerFailed = false;
  logPaneOpen = false;
  if (msg) {
    playerLogs = [{ text: String(msg), kind: 'step', time: plClock() }];
    renderPlayerStatus();
  } else {
    playerStatus.hidden = true;
    playerStatus.classList.remove('player-status--log');
    playerStatus.textContent = '';
  }
  renderLogPane();
}

export function showPlayerError(msg) {
  playerLogs = [...playerLogs, { text: String(msg), kind: 'error', time: plClock() }];
  playerFailed = true;
  renderPlayerStatus();
  renderLogPane();
}

export function pushPlayerLog(msg) {
  playerLogs = [...playerLogs, { text: String(msg), kind: 'step', time: plClock() }];
  renderPlayerStatus();
  renderLogPane();
}

function clearPlayerLog() {
  playerLogs = [];
  playerFailed = false;
  logPaneOpen = false;
  playerStatus.hidden = true;
  playerStatus.classList.remove('player-status--log');
  playerStatus.textContent = '';
  renderLogPane();
}

playerStatus.addEventListener('click', (e) => {
  if (e.target.closest('[data-pl-logtoggle]')) {
    logPaneOpen = !logPaneOpen;
    renderPlayerStatus();
    renderLogPane();
  }
});
playerLogClose?.addEventListener('click', () => {
  logPaneOpen = false;
  renderPlayerStatus();
  renderLogPane();
});

export function isPlayerOpen() {
  return !playerModal.hidden;
}

function renderTorrentStats(stats) {
  if (!stats) {
    playerTorrentStats.hidden = true;
    return;
  }
  const stalled = stats.download_speed < 1024 && stats.progress < 1;
  const pct = (stats.progress * 100).toFixed(1);
  playerTorrentStats.hidden = false;
  playerTorrentStats.classList.toggle('is-stalled', stalled);
  playerTorrentStats.innerHTML = `
    <span class="pts-dot" aria-hidden="true"></span>
    <span>${stats.peers} peer</span>
    <span class="pts-sep">·</span>
    <span>${fmtSpeed(stats.download_speed)}</span>
    <span class="pts-sep">·</span>
    <span>${pct}%</span>
    <span class="pts-sep">·</span>
    <span>${fmtBytes(stats.downloaded)}</span>
  `;
}
async function pollTorrentStatsOnce() {
  if (!playerTorrent?.infoHash) return;
  const ih = playerTorrent.infoHash;
  try {
    const stats = await fetchTorrentStats(ih);
    if (playerTorrent?.infoHash !== ih) return;
    renderTorrentStats(stats);
  } catch {}
}
function startTorrentStatsPolling() {
  stopTorrentStatsPolling();
  if (!playerTorrent?.infoHash) return;
  pollTorrentStatsOnce();
  playerTorrentTimer = setInterval(pollTorrentStatsOnce, 2000);
}
function stopTorrentStatsPolling() {
  if (playerTorrentTimer) {
    clearInterval(playerTorrentTimer);
    playerTorrentTimer = null;
  }
  playerTorrentStats.hidden = true;
  playerTorrentStats.textContent = '';
  playerTorrentStats.classList.remove('is-stalled');
}

function pushMpvGeometry() {
  const v = playerVideoBox.getBoundingClientRect();
  if (v.width < 1 || v.height < 1) return Promise.resolve();
  const dpr = window.devicePixelRatio || 1;
  return mpvSetGeometry(
    v.left * dpr, v.top * dpr, v.width * dpr, v.height * dpr
  ).catch(() => {});
}
function scheduleMpvGeometry() {
  if (mpvGeometryRaf) return;
  mpvGeometryRaf = requestAnimationFrame(() => {
    mpvGeometryRaf = 0;
    pushMpvGeometry();
  });
}
function startMpvGeometry() {
  stopMpvGeometry();
  mpvGeometryObs = new ResizeObserver(scheduleMpvGeometry);
  mpvGeometryObs.observe(playerVideoBox);
  window.addEventListener('resize', scheduleMpvGeometry);
  return pushMpvGeometry();
}
function stopMpvGeometry() {
  if (mpvGeometryObs) {
    mpvGeometryObs.disconnect();
    mpvGeometryObs = null;
  }
  window.removeEventListener('resize', scheduleMpvGeometry);
  if (mpvGeometryRaf) {
    cancelAnimationFrame(mpvGeometryRaf);
    mpvGeometryRaf = 0;
  }
}

let idleTimer = 0;
const IDLE_MS = 2500;
function wakePlayer() {
  if (playerFrame.classList.contains('is-idle')) {
    playerFrame.classList.remove('is-idle');
  }
  clearTimeout(idleTimer);
  idleTimer = setTimeout(() => {
    if (mpvState.paused) return;
    if (!playerSettings.hidden) return;
    if (document.querySelector('[data-pick-menu]:not([hidden])')) return;
    if (mpvState.seeking) return;
    if (FORM_TAGS.includes(document.activeElement?.tagName)) return;
    playerFrame.classList.add('is-idle');
  }, IDLE_MS);
}
function stopIdle() {
  clearTimeout(idleTimer);
  idleTimer = 0;
  playerFrame.classList.remove('is-idle');
}

const OBSERVED_PROPS = [
  'pause',
  'duration',
  'time-pos',
  'volume',
  'mute',
  'demuxer-cache-time',
  'track-list',
  'aid',
  'sid',
  'speed',
  'sub-delay',
  'eof-reached',
  'video-params',
];

async function ensureObservers() {
  if (observersInstalled) return;
  observersInstalled = true;

  if (!mpvUnlistenPromise && typeof onMpvEvent === 'function' && TAURI?.event?.listen) {
    mpvUnlistenPromise = onMpvEvent(handleMpvEvent);
    try { await mpvUnlistenPromise; } catch {}
  }

  await Promise.all(OBSERVED_PROPS.map(name => mpvObserve(name).catch(() => {})));

  // mpv's default volume-max is 130; align it with the slider's 0–200 range.
  mpvSet('volume-max', VOLUME_MAX).catch(() => {});
}

function fireOnEnded() {
  if (!playerOnEnded) return;
  const fn = playerOnEnded;
  playerOnEnded = null;
  try { fn(); } catch {}
}

function handleMpvEvent(payload) {
  if (!payload || typeof payload !== 'object') return;
  switch (payload.kind) {
    case 'property_changed':
      applyProperty(payload.name, payload.value);
      break;
    case 'end_file':
      if (payload.reason === 'eof') {
        fireOnEnded();
      } else if (payload.reason === 'error') {
        showPlayerError(tx('player.errFileRead'));
      } else if (payload.reason === 'unknown') {
        pushPlayerLog(tx('player.errLoadAborted'));
      }
      break;
    case 'file_loaded':
      if (pendingResumeTime > 5) {
        const t = pendingResumeTime;
        pendingResumeTime = 0;
        mpvCommand(['seek', String(t), 'absolute']).catch(() => {});
      } else {
        clearPlayerLog();
      }
      wakePlayer();
      break;
    case 'playback_restart':
      clearPlayerLog();
      break;
    default:
      break;
  }
}
function applyProperty(name, value) {
  switch (name) {
    case 'pause': {
      mpvState.paused = !!value;
      playerFrame.classList.toggle('is-paused', mpvState.paused);
      wakePlayer();
      pushRemoteStateNow();
      break;
    }
    case 'duration': {
      mpvState.duration = Number(value) || 0;
      playerTotalTime.textContent = fmtTime(mpvState.duration);
      playerProgress.setAttribute('aria-valuemax', String(Math.round(mpvState.duration)));
      pushRemoteStateNow();
      break;
    }
    case 'time-pos': {
      const t = Number(value) || 0;
      mpvState.timePos = t;
      if (!mpvState.seeking) {
        playerCurTime.textContent = fmtTime(t);
        playerProgress.setAttribute('aria-valuenow', String(Math.round(t)));
        const pct = mpvState.duration > 0 ? (t / mpvState.duration) * 100 : 0;
        playerProgressPlayed.style.width = pct + '%';
        playerProgressThumb.style.left = pct + '%';
      }
      if (!resumeSaveTimer) {
        resumeSaveTimer = setTimeout(() => {
          resumeSaveTimer = 0;
          if (playerCtx && mpvState.duration > 0) {
            saveResume(playerCtx, mpvState.timePos, mpvState.duration);
          }
        }, 5000);
      }
      pushRemoteStateThrottled();
      break;
    }
    case 'volume': {
      const prev = Math.round(mpvState.volume);
      mpvState.volume = Number(value) || 0;
      if (Number(playerVol.value) !== Math.round(mpvState.volume)) {
        playerVol.value = String(Math.round(mpvState.volume));
      }
      paintVolumeSlider(mpvState.volume);
      if (Math.round(mpvState.volume) !== prev) {
        showVolumePct();
      }
      if (Math.round(mpvState.volume) === 100 && prev !== 100) {
        pulseVolumeSnap();
      }
      updateMuteIcon();
      pushRemoteStateNow();
      break;
    }
    case 'mute': {
      mpvState.mute = !!value;
      updateMuteIcon();
      pushRemoteStateNow();
      break;
    }
    case 'demuxer-cache-time': {
      const ahead = Number(value) || 0;
      mpvState.cacheEnd = mpvState.timePos + ahead;
      if (mpvState.duration > 0) {
        const pct = Math.min(100, (mpvState.cacheEnd / mpvState.duration) * 100);
        playerProgressBuffered.style.width = pct + '%';
      }
      break;
    }
    case 'track-list': {
      mpvState.trackList = Array.isArray(value) ? value : [];
      renderSettingsBody();
      pushRemoteStateNow();
      break;
    }
    case 'aid': {
      mpvState.aid = value;
      renderSettingsBody();
      pushRemoteStateNow();
      break;
    }
    case 'sid': {
      mpvState.sid = value;
      renderSettingsBody();
      pushRemoteStateNow();
      break;
    }
    case 'speed': {
      const s = Number(value);
      if (Number.isFinite(s) && s > 0) {
        mpvState.speed = s;
        renderSettingsBody();
      }
      break;
    }
    case 'sub-delay': {
      mpvState.subDelay = Number(value) || 0;
      const el = playerSettingsBody.querySelector('[data-subdelay-value]');
      if (el) el.textContent = fmtSubDelay(mpvState.subDelay);
      break;
    }
    case 'eof-reached': {
      if (value === true) fireOnEnded();
      break;
    }
    case 'video-params': {
      if (value && typeof value === 'object') {
        const dw = Number(value.dw) || Number(value.w);
        const dh = Number(value.dh) || Number(value.h);
        let aspect = Number(value.aspect);
        if (!Number.isFinite(aspect) || aspect <= 0) {
          aspect = dw > 0 && dh > 0 ? dw / dh : NaN;
        }
        if (Number.isFinite(aspect) && aspect > 0.5 && aspect < 4) {
          scheduleMpvGeometry();
        }
      }
      break;
    }
    default:
      break;
  }
}
function updateMuteIcon() {
  let state;
  if (mpvState.mute || mpvState.volume <= 0) state = 'mute';
  else if (mpvState.volume < 50) state = 'mid';
  else state = 'high';
  playerMuteBtn.dataset.state = state;
}

const VOL_TRACK_BG = 'rgba(255,255,255,0.22)';
let volSnapTimer = 0;

function paintVolumeSlider(v) {
  const vol = Math.max(0, Math.min(VOLUME_MAX, Number(v) || 0));
  const pct = (vol / VOLUME_MAX) * 100;
  const hue = 120 - (vol / VOLUME_MAX) * 120;
  playerVol.style.background =
    `linear-gradient(to right, hsl(120 60% 42%) 0%, hsl(${hue} 75% 48%) ${pct}%, ${VOL_TRACK_BG} ${pct}%)`;
  playerVol.style.setProperty('--vol-hue', String(hue));
  if (playerVolPct) playerVolPct.textContent = `${Math.round(vol)}%`;
}

function pulseVolumeSnap() {
  const wrap = playerVol.closest('.player-volume');
  if (!wrap) return;
  wrap.classList.remove('vol-snap');
  void wrap.offsetWidth;
  wrap.classList.add('vol-snap');
  clearTimeout(volSnapTimer);
  volSnapTimer = setTimeout(() => wrap.classList.remove('vol-snap'), 200);
}

let volPctTimer = 0;

function positionVolumePct() {
  const wrap = playerVol.closest('.player-volume');
  if (!wrap || !playerVolPct) return;
  const wrapRect = wrap.getBoundingClientRect();
  const inRect = playerVol.getBoundingClientRect();
  let x = wrapRect.width / 2;
  // With the bar expanded, follow the thumb (12px wide) along the track;
  // collapsed (keyboard/remote changes) fall back to the wrapper centre.
  if (inRect.width >= 20) {
    const v = Math.max(0, Math.min(VOLUME_MAX, Number(playerVol.value) || 0));
    const half = 6;
    x = (inRect.left - wrapRect.left) + half + (v / VOLUME_MAX) * (inRect.width - half * 2);
  }
  playerVolPct.style.left = `${Math.round(x)}px`;
}

function showVolumePct() {
  if (!playerVolPct) return;
  positionVolumePct();
  playerVolPct.classList.add('is-visible');
  clearTimeout(volPctTimer);
  volPctTimer = setTimeout(() => playerVolPct.classList.remove('is-visible'), 900);
}

paintVolumeSlider(mpvState.volume);

let lastRemotePushTs = 0;
function remoteTracksOf(kind) {
  const currentId = kind === 'audio' ? mpvState.aid : mpvState.sid;
  return mpvState.trackList
    .filter(t => t && t.type === kind)
    .map(t => ({
      id: t.id,
      label: trackLabel(t),
      lang: t.lang || '',
      active: Number(currentId) === Number(t.id),
    }));
}
function pushRemoteStateNow() {
  lastRemotePushTs = Date.now();
  const audioTracks = remoteTracksOf('audio');
  const subTracks = remoteTracksOf('sub');
  const audioDisabled = mpvState.aid === false || mpvState.aid === 'no' || mpvState.aid == null;
  const subDisabled = mpvState.sid === false || mpvState.sid === 'no' || mpvState.sid == null;
  remotePushState({
    paused: !!mpvState.paused,
    time: mpvState.timePos || 0,
    duration: mpvState.duration || 0,
    volume: mpvState.volume || 0,
    mute: !!mpvState.mute,
    fullscreen: !!mpvState.fullscreen,
    title: playerTitleEl.textContent || '',
    eyebrow: playerEyebrow.textContent || '',
    playing: !playerModal.hidden,
    audio_tracks: audioTracks,
    sub_tracks: subTracks,
    audio_disabled: audioDisabled,
    sub_disabled: subDisabled,
  });
}
function pushRemoteStateThrottled() {
  const now = Date.now();
  if (now - lastRemotePushTs < 900) return;
  pushRemoteStateNow();
}

let settingsTab = 'audio';

const trackFilter = { audio: '', sub: '' };
function tracksOf(type) {
  return mpvState.trackList.filter(t => t && t.type === type);
}

function trackLabel(t) {
  if (t.title) return t.title;
  const parts = [];
  if (t.lang) parts.push(t.lang.toUpperCase());
  if (t.codec) parts.push(t.codec);
  return parts.join(' · ') || tx('player.trackN', { id: t.id });
}

function trackMatchesPrefix(t, prefix) {
  if (!prefix) return true;
  const label = trackLabel(t);
  const primary = label.split('[')[0].trim().toLowerCase();
  const lang = (t.lang || '').toLowerCase();
  return primary.startsWith(prefix) || lang.startsWith(prefix);
}
function speedLabel(s) {
  return s === 1 ? tx('player.speedNormal') : `${s}x`;
}
function renderSettingsBody() {
  if (!playerSettingsBody) return;
  if (playerSettings.hidden) return;

  if (settingsTab === 'speed') {
    delete playerSettingsBody.dataset.shellTab;
    renderSpeedList();
    return;
  }

  if (settingsTab === 'opensubtitles') {
    delete playerSettingsBody.dataset.shellTab;
    renderOpenSubtitlesList();
    return;
  }

  ensureTrackShell();
  renderTrackList();
}

let openSubsCache = null;
let openSubsCacheKey = null;
let openSubsLoading = false;
let openSubsAppliedId = null;
let openSubsFilter = '';
let openSubsManualImdb = '';

function parseImdbInput(raw) {
  const s = (raw || '').trim();
  if (!s) return null;
  const m = s.match(/^(tt\d{6,10})(?:[:\s\-_]+(\d{1,3})[:\sEex\-_]+(\d{1,3}))?$/i);
  if (!m) return null;
  const imdb = m[1].toLowerCase();
  if (m[2] && m[3]) {
    return { kind: 'series', id: `${imdb}:${Number(m[2])}:${Number(m[3])}` };
  }
  return { kind: 'movie', id: imdb };
}

function imdbInputValueForCtx() {
  if (openSubsManualImdb) return openSubsManualImdb;
  if (!playerCtx?.imdb) return '';
  if (playerCtx.type === 'tv') {
    return `${playerCtx.imdb}:${playerCtx.season}:${playerCtx.episode}`;
  }
  return playerCtx.imdb;
}

function getOpenSubsCtx() {
  if (openSubsManualImdb) return parseImdbInput(openSubsManualImdb);
  if (!playerCtx?.imdb) return null;
  if (playerCtx.type === 'tv') {
    return { kind: 'series', id: `${playerCtx.imdb}:${playerCtx.season}:${playerCtx.episode}` };
  }
  return { kind: 'movie', id: playerCtx.imdb };
}

function ensureOpenSubsShell() {
  if (playerSettingsBody.dataset.shellTab === 'opensubtitles') return;
  playerSettingsBody.dataset.shellTab = 'opensubtitles';
  playerSettingsBody.innerHTML = `
    <div class="player-settings-imdb-wrap">
      <input type="text" class="player-settings-imdb-input" data-os-imdb placeholder="${escapeHTML(tx('player.imdbPlaceholder'))}" autocomplete="off" spellcheck="false" />
      <button type="button" class="player-settings-imdb-btn" data-os-imdb-go>${escapeHTML(tx('player.searchBtn'))}</button>
    </div>
    <div class="player-settings-search-wrap">
      <input type="search" class="player-settings-search" placeholder="${escapeHTML(tx('player.filterByLang'))}" autocomplete="off" spellcheck="false" />
    </div>
    <div class="player-settings-list" data-os-list></div>
  `;

  const imdbInput = playerSettingsBody.querySelector('[data-os-imdb]');
  const imdbBtn = playerSettingsBody.querySelector('[data-os-imdb-go]');
  imdbInput.value = imdbInputValueForCtx();
  const submitImdb = () => {
    const raw = imdbInput.value.trim();
    if (!raw) {
      openSubsManualImdb = '';
      openSubsCache = null;
      openSubsCacheKey = null;
      openSubsAppliedId = null;
      renderOpenSubtitlesList();
      return;
    }
    const parsed = parseImdbInput(raw);
    if (!parsed) {
      const target = playerSettingsBody.querySelector('[data-os-list]');
      if (target) {
        target.innerHTML = `<p class="player-settings-empty">${tx('player.imdbInvalid')}</p>`;
      }
      return;
    }
    openSubsManualImdb = raw;
    openSubsCache = null;
    openSubsCacheKey = null;
    openSubsAppliedId = null;
    renderOpenSubtitlesList();
  };
  imdbBtn.addEventListener('click', submitImdb);
  imdbInput.addEventListener('keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); submitImdb(); }
  });

  const filter = playerSettingsBody.querySelector('.player-settings-search');
  filter.value = openSubsFilter;
  filter.addEventListener('input', () => {
    openSubsFilter = filter.value;
    paintOpenSubtitlesList(openSubsCache || []);
  });
}

async function renderOpenSubtitlesList() {
  ensureOpenSubsShell();
  // The shell persists across player sessions; re-sync the IMDb input with the
  // current context unless the user typed their own id or is typing right now.
  const imdbInput = playerSettingsBody.querySelector('[data-os-imdb]');
  if (imdbInput && !openSubsManualImdb && document.activeElement !== imdbInput) {
    imdbInput.value = imdbInputValueForCtx();
  }
  const ctx = getOpenSubsCtx();
  if (!ctx) {
    const target = playerSettingsBody.querySelector('[data-os-list]');
    if (target) {
      target.innerHTML = `<p class="player-settings-empty">${escapeHTML(tx('player.imdbPrompt'))}</p>`;
    }
    return;
  }

  const key = `${ctx.kind}:${ctx.id}`;
  if (openSubsCacheKey !== key) {
    openSubsCache = null;
    openSubsCacheKey = key;
    openSubsAppliedId = null;
  }

  if (!openSubsCache && !openSubsLoading) {
    openSubsLoading = true;
    const target = playerSettingsBody.querySelector('[data-os-list]');
    if (target) target.innerHTML = `<p class="player-settings-empty">${escapeHTML(tx('player.searchingSubs'))}</p>`;
    try {
      const data = await fetchSubtitles(ctx.kind, ctx.id);
      openSubsCache = data?.subtitles || [];
    } catch (e) {
      console.warn('subtitles fetch failed', e);
      openSubsCache = [];
    } finally {
      openSubsLoading = false;
    }
    if (settingsTab !== 'opensubtitles') return;
  }

  paintOpenSubtitlesList(openSubsCache || []);
}

function paintOpenSubtitlesList(subs) {
  const target = playerSettingsBody.querySelector('[data-os-list]');
  if (!target) return;

  if (!subs.length) {
    target.innerHTML = `<p class="player-settings-empty">${escapeHTML(tx('player.noSubsFound'))}</p>`;
    return;
  }

  const filter = (openSubsFilter || '').trim().toLowerCase();
  const filtered = filter
    ? subs.filter(s => {
        const label = subEntryLabel(s).toLowerCase();
        const lang = (s.lang || '').toLowerCase();
        const langName = langLabel(lang).toLowerCase();
        return label.startsWith(filter) || lang.startsWith(filter) || langName.startsWith(filter);
      })
    : subs;

  if (!filtered.length) {
    target.innerHTML = `<p class="player-settings-empty">${tx('player.noResultsFor', { q: escapeHTML(openSubsFilter) })}</p>`;
    return;
  }

  const grouped = new Map();
  for (const s of filtered) {
    const lang = (s.lang || 'unk').toLowerCase();
    if (!grouped.has(lang)) grouped.set(lang, []);
    grouped.get(lang).push(s);
  }

  const langOrder = [...grouped.keys()].sort((a, b) => {
    const pa = LANG_PRIORITY[a] ?? 99;
    const pb = LANG_PRIORITY[b] ?? 99;
    if (pa !== pb) return pa - pb;
    return a.localeCompare(b);
  });

  let html = '';
  for (const lang of langOrder) {
    const items = grouped.get(lang);
    html += `<div class="player-settings-sub-lang">${escapeHTML(langLabel(lang))} <span class="player-settings-sub-count">${items.length}</span></div>`;
    for (const s of items) {
      const id = s.id || s.url;
      const active = openSubsAppliedId === id;
      const label = subEntryLabel(s);
      html += `
        <button class="player-settings-opt${active ? ' is-active' : ''}" data-os-id="${escapeHTML(id)}" data-os-url="${escapeHTML(s.url)}" data-os-lang="${escapeHTML(s.lang || '')}" data-os-title="${escapeHTML(label)}">
          <span>${escapeHTML(label)}</span>
          <svg class="check" viewBox="0 0 24 24" width="14" height="14"><path fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" d="m5 12 5 5L20 7"/></svg>
        </button>
      `;
    }
  }
  target.innerHTML = html;
}

const LANG_PRIORITY = { ita: 0, it: 0, eng: 1, en: 1 };
const LANG_NAME_KEYS = {
  ita: 'player.langIt', it: 'player.langIt',
  eng: 'player.langEn', en: 'player.langEn',
  spa: 'player.langEs', es: 'player.langEs',
  fre: 'player.langFr', fra: 'player.langFr', fr: 'player.langFr',
  ger: 'player.langDe', deu: 'player.langDe', de: 'player.langDe',
  por: 'player.langPt', pt: 'player.langPt',
  rus: 'player.langRu', ru: 'player.langRu',
  jpn: 'player.langJa', ja: 'player.langJa',
  chi: 'player.langZh', zho: 'player.langZh', zh: 'player.langZh',
  unk: 'player.langUnknown',
};

function langLabel(lang) {
  const key = LANG_NAME_KEYS[lang];
  return key ? tx(key) : lang.toUpperCase();
}

function subEntryLabel(s) {
  const addon = s._addon || '';
  const id = s.id || '';
  const fname = id.split('/').pop()?.split('?')[0] || id;
  const cleaned = fname
    .replace(/\.[a-z0-9]{1,4}$/i, '')
    .replace(/[._]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (cleaned) return cleaned.length > 80 ? cleaned.slice(0, 80) + '…' : cleaned;
  return addon || tx('player.subtitle');
}

function renderSpeedList() {
  const cur = mpvState.speed || 1;
  let html = '';
  for (const s of SPEED_PRESETS) {
    const active = Math.abs(s - cur) < 0.01;
    html += `
      <button class="player-settings-opt${active ? ' is-active' : ''}" data-speed="${s}">
        <span>${speedLabel(s)}</span>
        <svg class="check" viewBox="0 0 24 24" width="14" height="14"><path fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" d="m5 12 5 5L20 7"/></svg>
      </button>
    `;
  }
  playerSettingsBody.innerHTML = html;
}

function fmtSubDelay(d) {
  const v = Number(d) || 0;
  return `${v > 0 ? '+' : ''}${v.toFixed(1)}s`;
}

function trackShellExtraHtml() {
  if (settingsTab === 'sub') {
    return `
      <div class="player-settings-extra">
        <span class="pse-label">${escapeHTML(tx('player.subDelay'))}</span>
        <div class="pse-controls">
          <button type="button" class="pse-btn" data-subdelay="-0.1">−0.1s</button>
          <span class="pse-value" data-subdelay-value>${escapeHTML(fmtSubDelay(mpvState.subDelay))}</span>
          <button type="button" class="pse-btn" data-subdelay="0.1">+0.1s</button>
          <button type="button" class="pse-btn" data-subdelay-reset>0s</button>
        </div>
      </div>`;
  }
  return '';
}

function ensureTrackShell() {
  if (playerSettingsBody.dataset.shellTab === settingsTab) return;
  playerSettingsBody.dataset.shellTab = settingsTab;
  const filter = trackFilter[settingsTab] || '';
  playerSettingsBody.innerHTML = `
    <div class="player-settings-search-wrap">
      <input type="search" class="player-settings-search" placeholder="${escapeHTML(tx('player.searchTrack'))}" autocomplete="off" spellcheck="false" />
    </div>
    <div class="player-settings-list" data-track-list></div>
    ${trackShellExtraHtml()}
  `;
  const input = playerSettingsBody.querySelector('.player-settings-search');
  input.value = filter;
  input.addEventListener('input', () => {
    trackFilter[settingsTab] = input.value;
    renderTrackList();
  });
}

function renderTrackList() {
  const target = playerSettingsBody.querySelector('[data-track-list]');
  if (!target) return;
  const list = tracksOf(settingsTab);
  const filter = (trackFilter[settingsTab] || '').trim().toLowerCase();
  const filtered = filter
    ? list.filter(t => trackMatchesPrefix(t, filter))
    : list;
  const currentId = settingsTab === 'audio' ? mpvState.aid : mpvState.sid;
  const noLabel = settingsTab === 'sub' ? tx('player.disable') : tx('common.none');
  const noActive = currentId === false || currentId === 'no' || currentId == null;

  let html = `
    <button class="player-settings-opt${noActive ? ' is-active' : ''}" data-track-id="no">
      <span>${escapeHTML(noLabel)}</span>
      <svg class="check" viewBox="0 0 24 24" width="14" height="14"><path fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" d="m5 12 5 5L20 7"/></svg>
    </button>
  `;
  if (!list.length) {
    html += `<p class="player-settings-empty">${escapeHTML(settingsTab === 'audio' ? tx('player.noTracksAudio') : tx('player.noTracksSub'))}</p>`;
  } else if (!filtered.length) {
    html += `<p class="player-settings-empty">${tx('player.noResultsFor', { q: escapeHTML(filter) })}</p>`;
  } else {
    for (const t of filtered) {
      const active = Number(currentId) === Number(t.id);
      html += `
        <button class="player-settings-opt${active ? ' is-active' : ''}" data-track-id="${t.id}">
          <span>${escapeHTML(trackLabel(t))}</span>
          <svg class="check" viewBox="0 0 24 24" width="14" height="14"><path fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" d="m5 12 5 5L20 7"/></svg>
        </button>
      `;
    }
  }
  target.innerHTML = html;
}
function hasSubtitlesAddon() {
  return (state.settings.addons || []).some(a =>
    a.enabled !== false && Array.isArray(a.resources) && a.resources.includes('subtitles'),
  );
}

function updateOpenSubsTabVisibility() {
  const tab = playerSettings.querySelector('[data-pst-tab="opensubtitles"]');
  if (!tab) return;
  const available = hasSubtitlesAddon();
  tab.hidden = !available;
  if (!available && settingsTab === 'opensubtitles') {
    settingsTab = 'sub';
    $$('[data-pst-tab]', playerSettings).forEach(b =>
      b.classList.toggle('is-active', b.dataset.pstTab === settingsTab),
    );
  }
}

function openSettings() {
  playerSettings.hidden = false;
  playerSettingsBtn.setAttribute('aria-expanded', 'true');
  updateOpenSubsTabVisibility();
  renderSettingsBody();
  wakePlayer();
}
function closeSettings() {
  playerSettings.hidden = true;
  playerSettingsBtn.setAttribute('aria-expanded', 'false');
  wakePlayer();
}
function toggleSettings() {
  playerSettings.hidden ? openSettings() : closeSettings();
}

function detachPlayer({ keepTorrent = false, sendStop = false } = {}) {
  if (playerCtx && mpvState.duration > 0 && mpvState.timePos > 0) {
    saveResume(playerCtx, mpvState.timePos, mpvState.duration);
  }

  if (sendStop) {
    mpvCommand(['stop']).catch(() => {});
  }
  stopMpvGeometry();
  stopIdle();
  if (resumeSaveTimer) {
    clearTimeout(resumeSaveTimer);
    resumeSaveTimer = 0;
  }

  if (splitEnabled) setSplitEnabled(false);
  mpvState.paused = true;
  mpvState.duration = 0;
  mpvState.timePos = 0;
  mpvState.cacheEnd = 0;
  mpvState.trackList = [];
  playerProgressPlayed.style.width = '0%';
  playerProgressBuffered.style.width = '0%';
  playerProgressThumb.style.left = '0%';
  playerCurTime.textContent = '0:00';
  playerTotalTime.textContent = '0:00';
  closeSettings();
  playerHlsUrl = null;
  playerCtx = null;
  playerOnEnded = null;
  pendingResumeTime = 0;
  trackFilter.audio = '';
  trackFilter.sub = '';
  openSubsFilter = '';
  if (!keepTorrent) {
    if (playerTorrent?.infoHash) destroyTorrentSession(playerTorrent.infoHash);
    playerTorrent = null;
    stopTorrentStatsPolling();
  }
}

async function attachStream(playlistUrl, opts = {}) {
  const newInfoHash = opts.torrent?.infoHash || null;
  const sameTorrent = !!newInfoHash && newInfoHash === playerTorrent?.infoHash;
  detachPlayer({ keepTorrent: sameTorrent });
  playerHlsUrl = playlistUrl;
  if (!sameTorrent) {
    playerTorrent = opts.torrent || null;
    if (playerTorrent?.infoHash) startTorrentStatsPolling();
  }
  playerCtx = opts.ctx || null;
  playerOnEnded = opts.onEnded || null;
  openSubsCache = null;
  openSubsCacheKey = null;
  openSubsAppliedId = null;
  openSubsFilter = '';
  openSubsManualImdb = '';

  try {
    await startMpvGeometry();
    await mpvSetVisible(true);
    await ensureObservers();

    // Any subtitle sync tweak is specific to the previous file.
    mpvState.subDelay = 0;
    mpvSet('sub-delay', 0).catch(() => {});

    const r = playerCtx ? getResume(playerCtx) : null;
    if (r && Number.isFinite(r.time) && r.time > 5) {
      pendingResumeTime = r.time;
      pushPlayerLog(tx('player.resumingFrom', { time: fmtTime(r.time) }));
    } else {
      pendingResumeTime = 0;
    }
    await mpvLoad(playlistUrl, null);
    mpvSet('pause', false).catch(() => {});

    mpvState.paused = false;
    wakePlayer();
  } catch (e) {
    showPlayerError(tx('player.errPlayer', { error: e.message || e }));
  }
}

function setPlayerEyebrow(eyebrow, eyebrowKey) {
  if (eyebrowKey) {
    playerEyebrow.dataset.i18n = eyebrowKey;
    playerEyebrow.textContent = tx(eyebrowKey);
  } else {
    delete playerEyebrow.dataset.i18n;
    playerEyebrow.textContent = eyebrow || '';
  }
}

export function openPlayerLoading({ title, eyebrow, eyebrowKey, torrent = null }) {
  playerModal.hidden = false;
  document.body.style.overflow = 'hidden';
  document.body.classList.add('is-player-open');

  if (detailsModal && !detailsModal.hidden) {
    detailsWasVisible = true;
    detailsModal.hidden = true;
  }
  setPlayerEyebrow(eyebrow, eyebrowKey);
  playerTitleEl.textContent = title || '';
  playerPickers.innerHTML = '';
  clearPlayerLog();
  playerTorrent = torrent || null;
  if (playerTorrent?.infoHash) startTorrentStatsPolling();
  updateOpenSubsTabVisibility();
}

export async function attachToOpenPlayer(url, opts = {}) {
  await attachStream(url, opts);
}

export async function openPlayerWithUrl({ url, title, eyebrow, eyebrowKey, status, probe = null, ctx = null }) {
  playerModal.hidden = false;
  document.body.style.overflow = 'hidden';
  document.body.classList.add('is-player-open');

  if (detailsModal && !detailsModal.hidden) {
    detailsWasVisible = true;
    detailsModal.hidden = true;
  }
  setPlayerEyebrow(eyebrow, eyebrowKey);
  playerTitleEl.textContent = title || '';
  playerPickers.innerHTML = '';
  setPlayerStatus(status || null);
  updateOpenSubsTabVisibility();
  await attachStream(url, { probe, ctx });
}

let playerAbort = null;
let detailsWasVisible = false;
export function setPlayerAbortController(ctl) {
  playerAbort = ctl;
}

export function closePlayer() {
  if (playerAbort) {
    try { playerAbort.abort(); } catch {}
    playerAbort = null;
  }

  if (mpvState.fullscreen) {
    windowSetFullscreen(false).catch(() => {});
    mpvState.fullscreen = false;
    playerFrame.classList.remove('is-fs');
  }

  if (detailsWasVisible && detailsModal) {
    detailsModal.hidden = false;
    detailsWasVisible = false;
  }

  playerModal.hidden = true;
  detachPlayer({ sendStop: true });

  mpvSetVisible(false).catch(() => {});
  playerPickers.innerHTML = '';
  setPlayerStatus(null);

  if (detailsModal.hidden && settingsModal.hidden) {
    document.body.style.overflow = '';
  } else {
    document.body.style.overflow = 'hidden';
  }
  document.body.classList.remove('is-player-open');

  window.dispatchEvent(new CustomEvent('siiis:player-closed'));
}

let splitEnabled = false;

const TMDB_IMG_FACE = 'https://image.tmdb.org/t/p/w138_and_h175_face';

function specRow(label, value) {
  if (value == null || value === '') return '';
  return `<div class="psp-spec"><dt>${escapeHTML(label)}</dt><dd>${value}</dd></div>`;
}

function renderSidePane() {
  if (!splitEnabled) return;
  const d = state.detail?.fullDetail;
  const der = state.detail?.derived || {};
  const cast = Array.isArray(state.detail?.cast) ? state.detail.cast : [];
  const title = state.detail?.title || '';
  const isTV = !!state.detail?.isTV;

  playerSidePaneTitle.textContent = title || tx('player.details');

  if (!d) {
    playerSidePaneBody.innerHTML = `<p class="psp-empty">${escapeHTML(tx('player.noInfo'))}</p>`;
    return;
  }

  const dateLabel = isTV ? tx('player.specPremiere') : tx('player.specRelease');
  const dateValue = isTV && d.last_air_date && d.last_air_date !== d.first_air_date
    ? `${fmtFullDate(d.first_air_date)} – ${fmtFullDate(d.last_air_date)}`
    : fmtFullDate(isTV ? d.first_air_date : d.release_date);

  const runtime = isTV
    ? (d.episode_run_time?.[0] ? `${d.episode_run_time[0]} min` : null)
    : (d.runtime ? fmtRuntime(d.runtime) : null);

  const genres = (d.genres || []).map(g => `<span class="psp-genre">${escapeHTML(g.name)}</span>`).join('');

  const specs = [
    specRow(dateLabel, dateValue || null),
    specRow(isTV ? tx('player.specRuntimeEp') : tx('player.specRuntime'), runtime),
    !isTV ? specRow(tx('player.specDirector'), der.director ? escapeHTML(der.director.name) : null) : '',
    !isTV && der.writers?.length ? specRow(tx('player.specWriters'), der.writers.map(w => escapeHTML(w.name)).join(', ')) : '',
    !isTV ? specRow(tx('player.specCinematography'), der.dop ? escapeHTML(der.dop.name) : null) : '',
    der.composer ? specRow(tx('player.specMusic'), escapeHTML(der.composer.name)) : '',
    isTV && der.creators?.length ? specRow(der.creators.length > 1 ? tx('player.specCreators') : tx('player.specCreator'), der.creators.map(c => escapeHTML(c.name)).join(', ')) : '',
    isTV && der.networks?.length ? specRow(tx('player.specNetwork'), der.networks.map(n => escapeHTML(n.name)).join(' · ')) : '',
    isTV ? specRow(tx('player.specSeasons'), d.number_of_seasons || null) : '',
    isTV ? specRow(tx('player.specEpisodes'), d.number_of_episodes || null) : '',
    isTV ? specRow(tx('player.specStatus'), der.statusLabel ? escapeHTML(der.statusLabel) : null) : '',
    specRow(tx('player.specVote'), d.vote_average ? `★ ${fmtVote(d.vote_average)} <small>(${d.vote_count || 0})</small>` : null),
    specRow(tx('player.specLanguage'), d.original_language ? escapeHTML(d.original_language.toUpperCase()) : null),
    specRow(tx('player.specCountries'), der.countries ? escapeHTML(der.countries) : null),
    !isTV && d.budget ? specRow(tx('player.specBudget'), fmtMoney(d.budget)) : '',
    !isTV && d.revenue ? specRow(tx('player.specRevenue'), fmtMoney(d.revenue)) : '',
  ].filter(Boolean).join('');

  const overviewSection = d.overview
    ? `<section class="psp-section"><h4 class="psp-section-title">${escapeHTML(tx('player.sectionSynopsis'))}</h4>${d.tagline ? `<p class="psp-tagline">${escapeHTML(d.tagline)}</p>` : ''}<p class="psp-overview">${escapeHTML(d.overview)}</p></section>`
    : '';

  const specsSection = specs
    ? `<section class="psp-section"><h4 class="psp-section-title">${escapeHTML(tx('player.sectionInfo'))}</h4><dl class="psp-grid">${specs}</dl></section>`
    : '';

  const genresSection = genres
    ? `<section class="psp-section"><h4 class="psp-section-title">${escapeHTML(tx('player.sectionGenres'))}</h4><div class="psp-genres">${genres}</div></section>`
    : '';

  const castItems = cast.map(c => {
    const portrait = c.profile_path ? `${TMDB_IMG_FACE}${c.profile_path}` : '';
    const hasId = Number.isFinite(c.id);
    const linkAttrs = hasId
      ? ` data-person-id="${c.id}" role="link" tabindex="0" title="${escapeHTML(tx('player.openPersonTmdb', { name: c.name || '' }))}"`
      : '';
    return `
      <li class="psp-cast-item${hasId ? ' is-clickable' : ''}"${linkAttrs}>
        <div class="psp-cast-img">${portrait ? `<img src="${escapeHTML(portrait)}" alt="" loading="lazy">` : '<span>👤</span>'}</div>
        <div class="psp-cast-meta">
          <div class="psp-cast-name">${escapeHTML(c.name || '')}</div>
          ${c.character ? `<div class="psp-cast-char">${escapeHTML(c.character)}</div>` : ''}
        </div>
      </li>`;
  }).join('');

  const castSection = cast.length
    ? `<section class="psp-section"><h4 class="psp-section-title">${escapeHTML(tx('player.sectionCast'))} <span class="psp-section-count">${cast.length}</span></h4><ul class="psp-cast">${castItems}</ul></section>`
    : '';

  playerSidePaneBody.innerHTML = `${overviewSection}${specsSection}${genresSection}${castSection}`;
}

function setSplitEnabled(on) {
  if (on === splitEnabled) return;
  splitEnabled = !!on;
  playerFrame.classList.toggle('is-split', splitEnabled);
  playerSplitBtn.setAttribute('aria-pressed', splitEnabled ? 'true' : 'false');
  if (splitEnabled) {
    renderSidePane();
    playerSidePaneBody.scrollTop = 0;
  }
  scheduleMpvGeometry();
  wakePlayer();
}

function toggleSplit() {
  setSplitEnabled(!splitEnabled);
}

function togglePlay() {
  mpvCommand(['cycle', 'pause']).catch(() => {});
}
function toggleMute() {
  mpvCommand(['cycle', 'mute']).catch(() => {});
}
async function toggleFullscreen() {
  try {
    const next = !mpvState.fullscreen;

    if (next && TAURI?.window) {
      try {
        const win = TAURI.window.getCurrentWindow();
        mpvState.wasMaximized = await win.isMaximized();
      } catch { mpvState.wasMaximized = false; }
    }

    await windowSetFullscreen(next);
    mpvState.fullscreen = next;
    playerFrame.classList.toggle('is-fs', next);

    if (!next && mpvState.wasMaximized && TAURI?.window) {
      try {
        const win = TAURI.window.getCurrentWindow();
        await win.unmaximize();
        await win.maximize();
      } catch {}
      mpvState.wasMaximized = false;
    }

    scheduleMpvGeometry();
  } catch (e) {
    pushPlayerLog(tx('player.errFullscreen', { error: e?.message || e }));
  }
}
function seekToFraction(frac) {
  if (!Number.isFinite(frac) || mpvState.duration <= 0) return;
  const target = Math.max(0, Math.min(1, frac)) * mpvState.duration;
  mpvSet('time-pos', target).catch(() => {});
  mpvState.timePos = target;
  playerCurTime.textContent = fmtTime(target);
  const pct = (target / mpvState.duration) * 100;
  playerProgressPlayed.style.width = pct + '%';
  playerProgressThumb.style.left = pct + '%';
}

playerPlayBtn.addEventListener('click', togglePlay);
playerBigPlay.addEventListener('click', togglePlay);
playerMuteBtn.addEventListener('click', toggleMute);
playerFsBtn.addEventListener('click', toggleFullscreen);
playerSplitBtn.addEventListener('click', toggleSplit);
playerSidePane.addEventListener('click', e => e.stopPropagation());

function openPersonFrom(target) {
  const item = target.closest('[data-person-id]');
  if (!item) return false;
  const id = item.dataset.personId;
  if (!id) return false;
  openExternal(`https://www.themoviedb.org/person/${id}`);
  return true;
}
playerSidePaneBody.addEventListener('click', e => {
  openPersonFrom(e.target);
});
playerSidePaneBody.addEventListener('keydown', e => {
  if (e.key !== 'Enter' && e.key !== ' ') return;
  if (openPersonFrom(e.target)) e.preventDefault();
});
playerSettingsBtn.addEventListener('click', e => {
  e.stopPropagation();
  toggleSettings();
});

playerVol.addEventListener('input', () => {
  let v = Number(playerVol.value);
  // 0–200 range: snap to exactly 100 so neutral volume stays easy to hit.
  if (v > 94 && v < 106 && v !== 100) {
    v = 100;
    playerVol.value = '100';
  }
  paintVolumeSlider(v);
  showVolumePct();
  mpvSet('volume', v).catch(() => {});
  if (mpvState.mute && v > 0) {
    mpvSet('mute', false).catch(() => {});
  }
});

function fractionFromPointer(ev) {
  const rect = playerProgress.getBoundingClientRect();
  if (rect.width <= 0) return 0;
  return Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
}
playerProgress.addEventListener('pointerdown', ev => {
  ev.preventDefault();
  mpvState.seeking = true;
  playerProgress.setPointerCapture(ev.pointerId);
  seekToFraction(fractionFromPointer(ev));
});
playerProgress.addEventListener('pointermove', ev => {
  if (mpvState.duration <= 0) return;
  const frac = fractionFromPointer(ev);
  const t = frac * mpvState.duration;
  const rect = playerProgress.getBoundingClientRect();
  playerProgressTip.hidden = false;
  playerProgressTip.textContent = fmtTime(t);
  playerProgressTip.style.left = (ev.clientX - rect.left) + 'px';
  if (mpvState.seeking) {
    seekToFraction(frac);
  }
});
playerProgress.addEventListener('pointerleave', () => {
  if (!mpvState.seeking) playerProgressTip.hidden = true;
});
playerProgress.addEventListener('pointerup', ev => {
  if (!mpvState.seeking) return;
  mpvState.seeking = false;
  try { playerProgress.releasePointerCapture(ev.pointerId); } catch {}
  playerProgressTip.hidden = true;
});
playerProgress.addEventListener('pointercancel', () => {
  mpvState.seeking = false;
  playerProgressTip.hidden = true;
});

$$('[data-pst-tab]', playerSettings).forEach(btn => {
  btn.addEventListener('click', () => {
    settingsTab = btn.dataset.pstTab;
    $$('[data-pst-tab]', playerSettings).forEach(b => b.classList.toggle('is-active', b === btn));
    trackFilter.audio = '';
    trackFilter.sub = '';
    openSubsFilter = '';
    renderSettingsBody();
  });
});
playerSettingsBody.addEventListener('click', e => {
  const speedOpt = e.target.closest('[data-speed]');
  if (speedOpt) {
    const v = Number(speedOpt.dataset.speed);
    if (Number.isFinite(v) && v > 0) {
      mpvSet('speed', v).catch(() => {});
      mpvState.speed = v;
      renderSettingsBody();
    }
    return;
  }
  const sdReset = e.target.closest('[data-subdelay-reset]');
  if (sdReset) {
    mpvSet('sub-delay', 0).catch(() => {});
    return;
  }
  const sdBtn = e.target.closest('[data-subdelay]');
  if (sdBtn) {
    const step = Number(sdBtn.dataset.subdelay);
    if (Number.isFinite(step) && step) {
      mpvCommand(['add', 'sub-delay', String(step)]).catch(() => {});
    }
    return;
  }
  const osOpt = e.target.closest('[data-os-id]');
  if (osOpt) {
    const url = osOpt.dataset.osUrl;
    const lang = osOpt.dataset.osLang || '';
    const title = osOpt.dataset.osTitle || 'OpenSubtitles';
    const id = osOpt.dataset.osId;
    if (!url) return;
    const args = ['sub-add', url, 'select', title];
    if (lang) args.push(lang);
    mpvCommand(args)
      .then(() => {
        openSubsAppliedId = id;
        if (settingsTab === 'opensubtitles') paintOpenSubtitlesList(openSubsCache || []);
      })
      .catch(err => {
        console.warn('sub-add failed', err);
        pushPlayerLog(tx('player.errSubLoad'));
      });
    return;
  }
  const opt = e.target.closest('[data-track-id]');
  if (!opt) return;
  const id = opt.dataset.trackId;
  const prop = settingsTab === 'audio' ? 'aid' : 'sid';
  const value = id === 'no' ? 'no' : Number(id);
  mpvSet(prop, value).catch(() => {});
});

document.addEventListener('click', e => {
  if (playerSettings.hidden) return;
  if (e.target.closest('#playerSettings')) return;
  if (e.target.closest('#playerSettingsBtn')) return;
  // Popups opened from the panel (text editor, dropdown backdrop) sit outside it.
  if (e.target.closest('.input-popup, .pick-backdrop')) return;
  closeSettings();
});

['mousemove', 'pointermove', 'pointerdown', 'wheel'].forEach(ev => {
  playerFrame.addEventListener(ev, () => wakePlayer());
});

const playerSpeedHint = $('#playerSpeedHint');
let holdTimer = 0;
let holdActive = false;
let holdJustEnded = false;
let savedSpeedBeforeHold = 1;
let holdPointerId = null;

function startHoldFastForward() {
  if (holdActive) return;
  // Nothing to fast-forward while paused: no 2x, no hint.
  if (mpvState.paused) return;
  holdActive = true;
  savedSpeedBeforeHold = mpvState.speed || 1;
  mpvSet('speed', 2).catch(() => {});
  if (playerSpeedHint) {
    playerSpeedHint.hidden = false;
    requestAnimationFrame(() => playerSpeedHint.classList.add('is-visible'));
  }
}

function endHoldFastForward() {
  if (holdTimer) {
    clearTimeout(holdTimer);
    holdTimer = 0;
  }
  if (holdActive) {
    holdActive = false;
    holdJustEnded = true;
    mpvSet('speed', savedSpeedBeforeHold || 1).catch(() => {});
  }
  if (playerSpeedHint) {
    playerSpeedHint.classList.remove('is-visible');
    setTimeout(() => {
      if (!holdActive && playerSpeedHint) playerSpeedHint.hidden = true;
    }, 140);
  }
  holdPointerId = null;
}

playerFrame.addEventListener('dblclick', e => {
  if (IS_ANDROID) return; // no fullscreen toggle on the phone, it already fills the screen
  if (e.target.closest('.player-controls, .player-top, .player-pickers, input')) return;
  if (videoClickTimer) {
    clearTimeout(videoClickTimer);
    videoClickTimer = 0;
  }
  toggleFullscreen();
});

playerVideoBox.addEventListener('pointerdown', e => {
  if (IS_ANDROID) return; // no hold for 2x speed on the phone
  if (e.button !== 0) return;
  if (e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
  holdPointerId = e.pointerId;
  if (holdTimer) clearTimeout(holdTimer);
  holdTimer = setTimeout(() => {
    holdTimer = 0;
    if (mpvState.paused) {
      // Long press while paused: no 2x, and the release is not a click either.
      holdJustEnded = true;
      return;
    }
    startHoldFastForward();
  }, 250);
});

document.addEventListener('pointerup', e => {
  if (holdPointerId !== null && e.pointerId !== holdPointerId) return;
  endHoldFastForward();
});

// A plain click on the video closes whichever panel is open on the side
// (the info pane, the log, the settings popup); with nothing open it toggles
// play/pause. Releasing a hold-to-fast-forward also ends with a click: that
// one is ignored, so the 2x hold keeps working. The toggle waits briefly so a
// double click (fullscreen) does not flip playback twice.
let videoClickTimer = 0;
playerVideoBox.addEventListener('click', e => {
  if (holdJustEnded) { holdJustEnded = false; return; }
  if (e.target.closest('.player-controls, .player-top, .player-pickers, button, input')) return;
  if (!playerSettings.hidden) return;
  let closedPanel = false;
  if (logPaneOpen) {
    logPaneOpen = false;
    renderPlayerStatus();
    renderLogPane();
    closedPanel = true;
  }
  if (splitEnabled) {
    setSplitEnabled(false);
    closedPanel = true;
  }
  if (closedPanel) return;
  // Android: play and pause only from the button in the controls bar.
  if (IS_ANDROID) return;
  if (videoClickTimer) clearTimeout(videoClickTimer);
  videoClickTimer = setTimeout(() => {
    videoClickTimer = 0;
    togglePlay();
  }, 220);
});
document.addEventListener('pointercancel', () => endHoldFastForward());

playerModal.addEventListener('transitionend', () => {
  if (playerModal.hidden && holdActive) endHoldFastForward();
});

playerModal.addEventListener('click', e => {
  const t = e.target;
  if (t && (t.matches('[data-close]') || t.closest('[data-close]'))) {
    closePlayer();
  }
});

document.addEventListener('keydown', e => {
  if (playerModal.hidden) return;
  if (FORM_TAGS.includes(document.activeElement?.tagName)) {
    if (e.key !== 'Escape') return;
  }
  switch (e.key) {
    case 'Escape':
      e.preventDefault();
      if (mpvState.fullscreen) {
        toggleFullscreen();
      } else if (!playerSettings.hidden) {
        closeSettings();
      } else {
        closePlayer();
      }
      break;
    case 'k':
    case 'K':
      e.preventDefault();
      togglePlay();
      break;
    case 'm':
    case 'M':
      e.preventDefault();
      toggleMute();
      break;
    case 'f':
    case 'F':
      e.preventDefault();
      toggleFullscreen();
      break;
    case 'ArrowLeft':
      e.preventDefault();
      mpvCommand(['seek', '-5', 'relative']).catch(() => {});
      wakePlayer();
      break;
    case 'ArrowRight':
      e.preventDefault();
      mpvCommand(['seek', '5', 'relative']).catch(() => {});
      wakePlayer();
      break;
    case 'ArrowUp':
      e.preventDefault();
      mpvSet('volume', Math.min(VOLUME_MAX, mpvState.volume + 5)).catch(() => {});
      wakePlayer();
      break;
    case 'ArrowDown':
      e.preventDefault();
      mpvSet('volume', Math.max(0, mpvState.volume - 5)).catch(() => {});
      wakePlayer();
      break;
    case 'z':
    case 'Z':
      e.preventDefault();
      mpvCommand(['add', 'sub-delay', '0.1']).catch(() => {});
      wakePlayer();
      break;
    case 'x':
    case 'X':
      e.preventDefault();
      mpvCommand(['add', 'sub-delay', '-0.1']).catch(() => {});
      wakePlayer();
      break;
  }
});

let spacePressedAt = 0;
let spaceHoldTimer = 0;
let swallowNextClick = false;
let swallowClickTimer = 0;
const SPACE_HOLD_MS = 200;

document.addEventListener('click', e => {
  if (!swallowNextClick) return;
  swallowNextClick = false;
  if (swallowClickTimer) { clearTimeout(swallowClickTimer); swallowClickTimer = 0; }
  e.preventDefault();
  e.stopImmediatePropagation();
}, true);

document.addEventListener('keydown', e => {
  if (e.key !== ' ') return;
  if (playerModal.hidden) return;
  if (FORM_TAGS.includes(document.activeElement?.tagName)) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  if (spacePressedAt > 0) return;
  spacePressedAt = Date.now();
  swallowNextClick = true;
  if (swallowClickTimer) clearTimeout(swallowClickTimer);
  swallowClickTimer = setTimeout(() => { swallowNextClick = false; swallowClickTimer = 0; }, 500);
  if (spaceHoldTimer) clearTimeout(spaceHoldTimer);
  spaceHoldTimer = setTimeout(() => {
    spaceHoldTimer = 0;
    startHoldFastForward();
  }, SPACE_HOLD_MS);
}, true);

document.addEventListener('keyup', e => {
  if (e.key !== ' ') return;
  if (spacePressedAt === 0) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  const dur = Date.now() - spacePressedAt;
  spacePressedAt = 0;
  if (spaceHoldTimer) {
    clearTimeout(spaceHoldTimer);
    spaceHoldTimer = 0;
  }
  if (dur < SPACE_HOLD_MS) {
    togglePlay();
  } else if (holdActive) {
    endHoldFastForward();
  }
}, true);

window.addEventListener('pagehide', () => {
  if (playerTorrent?.infoHash) destroyTorrentSession(playerTorrent.infoHash);
});

function handleNavCommand(dir) {
  if (!dir) return;
  const playerOpen = !playerModal.hidden;

  if (dir === 'home') {
    if (playerOpen) closePlayer();
    spatialNav.home();
    return;
  }
  if (dir === 'back') {
    if (playerOpen) {
      if (!playerSettings.hidden) closeSettings();
      else if (mpvState.fullscreen) toggleFullscreen();
      else closePlayer();
    } else {
      spatialNav.back();
    }
    return;
  }
  if (playerOpen) wakePlayer();
  if (dir === 'ok') spatialNav.activate();
  else spatialNav.move(dir);
}

playerProgress.addEventListener('snav-adjust', (e) => {
  const step = e.detail === 'right' ? 10 : -10;
  mpvCommand(['seek', String(step), 'relative']).catch(() => {});
  wakePlayer();
});
playerVol.addEventListener('snav-adjust', (e) => {
  const delta = e.detail === 'up' ? 5 : -5;
  const v = Math.max(0, Math.min(VOLUME_MAX, Math.round((mpvState.volume || 0) + delta)));
  mpvSet('volume', v).catch(() => {});
  if (mpvState.mute && v > 0) mpvSet('mute', false).catch(() => {});
  wakePlayer();
});

function handleRemoteCmd(cmd) {
  if (!cmd || typeof cmd !== 'object') return;
  const name = cmd.name;
  const value = cmd.value;
  switch (name) {
    case 'request_state':
      pushRemoteStateNow();
      break;
    case 'toggle':
      togglePlay();
      break;
    case 'play':
      mpvSet('pause', false).catch(() => {});
      break;
    case 'pause':
      mpvSet('pause', true).catch(() => {});
      break;
    case 'mute_toggle':
      toggleMute();
      break;
    case 'volume':
      if (Number.isFinite(value)) {
        const v = Math.max(0, Math.min(100, Math.round(Number(value))));
        mpvSet('volume', v).catch(() => {});
      }
      break;
    case 'seek':
      if (Number.isFinite(value) && mpvState.duration > 0) {
        const target = Math.max(0, Math.min(mpvState.duration, Number(value)));
        mpvSet('time-pos', target).catch(() => {});
      }
      break;
    case 'skip':
      if (Number.isFinite(value)) {
        mpvCommand(['seek', String(Number(value)), 'relative']).catch(() => {});
      }
      break;
    case 'fullscreen':
      toggleFullscreen();
      break;
    case 'track': {
      const kind = cmd.kind;
      const id = cmd.id;
      if (kind !== 'aid' && kind !== 'sid') break;
      const v = id === 'no' || id == null ? 'no' : Number(id);
      mpvSet(kind, v).catch(() => {});
      break;
    }
    case 'nav':
      if (typeof value === 'string') handleNavCommand(value);
      break;
    case 'search': {
      if (typeof value !== 'string') break;
      if (!playerModal.hidden) closePlayer();
      const searchEl = document.getElementById('search');
      if (searchEl) {
        searchEl.value = value;
        searchEl.dispatchEvent(new Event('input', { bubbles: true }));
      }
      break;
    }
    default:
      break;
  }
}

if (typeof onRemoteCmd === 'function') {
  onRemoteCmd(handleRemoteCmd).catch(() => {});
}
