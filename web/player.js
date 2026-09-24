// The browser's player (siiishub-server): stands in for the app's embedded
// mpv. The app's player interface (js/player.js) drives mpv with `mpv_*`
// commands and listens to `mpv://event`; web/bridge.js hands those commands
// here, and this answers with the same properties and events over an HTML5
// video in #playerVideo.
//
// The server publishes each stream at /media/<id> (src-tauri/src/server/
// media.rs). What the browser can play goes as it is, byte ranges and all.
// The rest goes through HLS (server/hls.rs), as Stremio does: a playlist of
// the whole film in 6-second segments that ffmpeg makes on demand, the video
// copied or turned into H.264 and the audio into AAC. hls.js plays it (Safari
// by itself), so seeking is the video element's own; every text subtitle of
// the file comes with the session, and switching them restarts nothing.
// Without hls.js (offline CDN) the older way remains: ffmpeg's fragmented
// MP4 as one stream from a given second, restarted at every seek.
import { state } from '../js/state.js';
import { t } from '../js/i18n.js';

const EVENT = 'mpv://event';
const VOLUME_MAX = 200;
const HLS_JS = 'https://cdn.jsdelivr.net/npm/hls.js@1.7.3/dist/hls.min.mjs';
// A progressive stream that stops this far from the end did not reach the
// end of the film: it is resumed instead of ending the playback.
const EOF_MARGIN = 30;
const SUB_POLL_MS = 2000;
const TEXT_SUBTITLES = ['subrip', 'ass', 'ssa', 'webvtt', 'mov_text', 'text'];

const box = document.getElementById('playerVideo');
const video = document.createElement('video');
video.className = 'web-video';
video.playsInline = true;
video.preload = 'auto';
box.appendChild(video);
const subtitleTrack = video.addTextTrack('subtitles', 'SIIISHUB', '');
subtitleTrack.mode = 'showing';

// ---------- Notice ----------
// A word over the video about what the browser cannot do with the file,
// until closed or for half a minute.
const NOTICE_MS = 30000;
const notice = document.createElement('div');
notice.className = 'web-notice';
notice.setAttribute('role', 'status');
notice.hidden = true;
notice.innerHTML = `
  <svg class="web-notice-icon" viewBox="0 0 24 24" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M12 3 2 20h20L12 3zM12 10v4M12 17.5v.01"/></svg>
  <span class="web-notice-text"></span>
  <button class="web-notice-close" type="button"><svg viewBox="0 0 24 24" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" d="M6 6l12 12M18 6 6 18"/></svg></button>`;
document.getElementById('playerFrame')?.appendChild(notice);
let noticeTimer = 0;

function showNotice(text) {
  notice.querySelector('.web-notice-text').textContent = text;
  notice.querySelector('.web-notice-close').setAttribute('aria-label', t('common.close'));
  notice.hidden = false;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(hideNotice, NOTICE_MS);
}

function hideNotice() {
  clearTimeout(noticeTimer);
  notice.hidden = true;
}

notice.querySelector('.web-notice-close').addEventListener('click', hideNotice);

// ---------- What the browser plays ----------
const tester = document.createElement('video');
const can = (type) => tester.canPlayType(type) !== '';
const CAN = {
  // Chromium reads Matroska with its FFmpeg demuxer when the codecs are
  // ones it plays.
  matroska: /\b(Chrome|Chromium|Edg)\//.test(navigator.userAgent),
  h264: can('video/mp4; codecs="avc1.640028"'),
  hevc10: can('video/mp4; codecs="hvc1.2.4.L153.B0"'),
  hevc8: can('video/mp4; codecs="hvc1.1.6.L150.B0"'),
  av1: can('video/mp4; codecs="av01.0.08M.08"'),
  av1_10: can('video/mp4; codecs="av01.0.08M.10"'),
  vp9: can('video/webm; codecs="vp9"'),
  vp8: can('video/webm; codecs="vp8"'),
  aac: can('audio/mp4; codecs="mp4a.40.2"'),
  mp3: can('audio/mpeg'),
  opus: can('audio/webm; codecs="opus"'),
  vorbis: can('audio/webm; codecs="vorbis"'),
  flac: can('audio/flac'),
  ac3: can('audio/mp4; codecs="ac-3"'),
  eac3: can('audio/mp4; codecs="ec-3"'),
  nativeHls: can('application/vnd.apple.mpegurl'),
};

function videoPlays(v) {
  if (!v) return false;
  const deep = (v.bit_depth || 8) > 8 || /10/.test(v.pix_fmt || '');
  switch (v.codec) {
    case 'h264': return CAN.h264 && !deep;
    case 'hevc': return deep ? CAN.hevc10 : (CAN.hevc8 || CAN.hevc10);
    case 'av1': return deep ? CAN.av1_10 : CAN.av1;
    case 'vp9': return CAN.vp9;
    case 'vp8': return CAN.vp8;
    default: return false;
  }
}

function audioPlays(a) {
  return !!a && !!CAN[a.codec];
}

// As it is: one audio track at most (a video element cannot switch tracks),
// codecs and container the browser knows.
function playsAsItIs(info) {
  const v = info.video;
  if (!videoPlays(v)) return false;
  if (info.audios.length > 1) return false;
  if (info.audios.length === 1 && !audioPlays(info.audios[0])) return false;
  if (info.container === 'mov') return true;
  if (info.container === 'matroska') {
    return CAN.matroska && ['h264', 'vp8', 'vp9', 'av1'].includes(v.codec)
      && info.audios.every(a => ['aac', 'mp3', 'opus', 'vorbis', 'flac'].includes(a.codec));
  }
  return false;
}

// hls.js from the CDN, once; `false` when it cannot run here.
let HlsJs;
async function loadHlsJs() {
  if (HlsJs !== undefined) return HlsJs;
  try {
    const mod = await import(HLS_JS);
    HlsJs = mod.default?.isSupported?.() ? mod.default : false;
  } catch (err) {
    console.warn('[player] hls.js unavailable', err);
    HlsJs = false;
  }
  return HlsJs;
}

// ---------- State ----------
let generation = 0;
// { url, info, mode: 'direct' | 'copy' | 'h264', transport: 'direct' | 'hls' | 'progressive' }
let media = null;
let opening = false;    // file_loaded sent, the stream not started yet
let pendingStart = 0;   // where to start it (a resume seek lands here)
let offset = 0;         // where a progressive stream starts, in seconds
let restarting = false; // a new stream: playback_restart when it plays
let stopped = true;
let hls = null;         // the hls.js instance
let session = null;     // { id, subtitles } of the HLS session
let mediaRecovered = false;

let aid = 1;
let sid = 'no';
let subDelay = 0;
let volume = 100;
let speed = 1;
let trackList = [];
// sid -> { kind: 'embedded' | 'external', index?, title, lang, codec, cues, seen }
// with the cues on the film's timeline. Embedded ones come first, numbered
// as mpv numbers them.
const subs = new Map();
let nextSubId = 1;
let subFeed = null;     // cues of an embedded subtitle being fetched

const observed = new Set();
let lastTimeEmit = 0;

function emit(payload) {
  window.__TAURI__.__emitLocal(EVENT, payload);
}

function prop(name, value) {
  if (observed.has(name)) emit({ kind: 'property_changed', name, value });
}

function duration() {
  const d = media?.info?.duration || 0;
  if (d > 0) return d;
  return Number.isFinite(video.duration) ? video.duration : 0;
}

function position() {
  if (opening) return pendingStart;
  return offset + (video.currentTime || 0);
}

function current(name) {
  switch (name) {
    case 'pause': return video.paused;
    case 'duration': return duration();
    case 'time-pos': return position();
    case 'volume': return volume;
    case 'mute': return video.muted;
    case 'demuxer-cache-time': return bufferedAhead();
    case 'track-list': return trackList;
    case 'aid': return aid;
    case 'sid': return sid;
    case 'speed': return speed;
    case 'sub-delay': return subDelay;
    case 'eof-reached': return false;
    case 'video-params': return videoParams();
    default: return null;
  }
}

function bufferedAhead() {
  const t = video.currentTime;
  for (let i = 0; i < video.buffered.length; i++) {
    if (video.buffered.start(i) <= t + 0.5 && t <= video.buffered.end(i)) {
      return Math.max(0, video.buffered.end(i) - t);
    }
  }
  return 0;
}

function videoParams() {
  const v = media?.info?.video;
  if (!v || !v.width || !v.height) return null;
  return { w: v.width, h: v.height, dw: v.width, dh: v.height, aspect: v.width / v.height };
}

// ---------- Languages ----------
// ISO 639-1 codes and their 639-2 forms, B and T (the app's settings and the
// files use either).
const LANGS = {
  en: ['eng'], it: ['ita'], es: ['spa'], fr: ['fre', 'fra'], de: ['ger', 'deu'], pt: ['por'],
  nl: ['dut', 'nld'], pl: ['pol'], ru: ['rus'], uk: ['ukr'], cs: ['cze', 'ces'], hu: ['hun'],
  ro: ['rum', 'ron'], el: ['gre', 'ell'], tr: ['tur'], sv: ['swe'], da: ['dan'], fi: ['fin'],
  nb: ['nob', 'nor'], no: ['nor', 'nob'], ja: ['jpn'], ko: ['kor'], zh: ['chi', 'zho'],
  ar: ['ara'], he: ['heb'], hi: ['hin'],
};
function langKey(code) {
  const c = (code || '').toLowerCase().split(/[-_]/)[0];
  if (c.length === 2) return c;
  for (const [two, three] of Object.entries(LANGS)) if (three.includes(c)) return two;
  return c;
}
function sameLang(a, b) {
  return !!a && !!b && langKey(a) === langKey(b);
}

// The audio track the app's settings prefer, else the default one.
function preferredAudio(info) {
  for (const lang of state.settings?.playerAudioLangs || []) {
    const i = info.audios.findIndex(a => sameLang(a.lang, lang));
    if (i >= 0) return i;
  }
  const def = info.audios.findIndex(a => a.default);
  return def >= 0 ? def : 0;
}

// The subtitle the settings prefer among the file's, as mpv's `slang` picks.
function preferredSub() {
  for (const lang of state.settings?.playerSubLangs || []) {
    for (const [id, sub] of subs) {
      if (sub.kind === 'embedded' && sameLang(sub.lang, lang)) return id;
    }
  }
  return 'no';
}

// ---------- Tracks ----------
function buildTrackList() {
  const info = media?.info;
  const list = [];
  if (info?.video) {
    list.push({ id: 1, type: 'video', codec: info.video.codec, 'demux-w': info.video.width, 'demux-h': info.video.height, selected: true });
  }
  (info?.audios || []).forEach((a, i) => {
    list.push({
      id: i + 1, type: 'audio', lang: a.lang || '', title: a.title || '', codec: a.codec,
      'audio-channels': a.channels, default: !!a.default, selected: aid === i + 1,
    });
  });
  for (const [id, sub] of subs) {
    list.push({
      id, type: 'sub', lang: sub.lang, title: sub.title, codec: sub.codec,
      external: sub.kind === 'external', selected: sid === id,
    });
  }
  trackList = list;
  prop('track-list', trackList);
}

// ---------- Subtitles ----------
function parseTimestamp(s) {
  let secs = 0;
  for (const part of s.trim().split(':')) secs = secs * 60 + parseFloat(part.replace(',', '.'));
  return secs;
}

function parseVtt(text) {
  const cues = [];
  for (const block of text.replace(/\r/g, '').split(/\n\n+/)) {
    const lines = block.split('\n');
    const i = lines.findIndex(l => l.includes('-->'));
    if (i < 0) continue;
    const [from, rest] = lines[i].split('-->');
    const start = parseTimestamp(from);
    const end = parseTimestamp(rest.trim().split(/\s+/)[0]);
    const body = lines.slice(i + 1).join('\n').trim();
    if (Number.isFinite(start) && Number.isFinite(end) && body) cues.push({ start, end, text: body });
  }
  return cues;
}

// A cue of the film on the video's timeline: shifted by the subtitle delay,
// and by where a progressive stream started.
function toVideoCue(c) {
  const shift = subDelay - offset;
  const end = c.end + shift;
  if (end <= 0) return null;
  return new VTTCue(Math.max(0, c.start + shift), end, c.text);
}

function renderCues() {
  for (const cue of [...(subtitleTrack.cues || [])]) subtitleTrack.removeCue(cue);
  const sub = typeof sid === 'number' ? subs.get(sid) : null;
  for (const c of sub?.cues || []) {
    const cue = toVideoCue(c);
    if (cue) subtitleTrack.addCue(cue);
  }
}

function addCues(sub, cues) {
  const shown = typeof sid === 'number' && subs.get(sid) === sub;
  for (const c of cues) {
    const key = `${c.start.toFixed(2)}|${c.text}`;
    if (sub.seen.has(key)) continue;
    sub.seen.add(key);
    sub.cues.push(c);
    if (shown) {
      const cue = toVideoCue(c);
      if (cue) subtitleTrack.addCue(cue);
    }
  }
}

async function addSubtitle(url, title, lang) {
  const res = await fetch(`/media/subtitle?url=${encodeURIComponent(url)}`, { credentials: 'same-origin' });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const sub = { kind: 'external', title: title || '', lang: lang || '', codec: 'webvtt', cues: [], seen: new Set() };
  addCues(sub, parseVtt(await res.text()));
  const id = nextSubId++;
  subs.set(id, sub);
  selectSub(id);
}

function stopSubFeed() {
  if (subFeed) clearInterval(subFeed.timer);
  subFeed = null;
}

// The cues of the selected embedded subtitle, as the stream brings them: the
// HLS session gathers them per track (film times); a progressive stream
// writes one growing WebVTT file (its times start where it started).
function followSubtitle(token) {
  stopSubFeed();
  const sub = typeof sid === 'number' ? subs.get(sid) : null;
  if (!media || sub?.kind !== 'embedded') return;
  if (media.transport === 'hls') {
    const n = session?.subtitles.indexOf(sub.index) ?? -1;
    if (n < 0) return;
    subFeed = { url: `/hls/${session.id}/subtitles/${n}`, from: 0, sub, busy: false };
  } else if (token) {
    subFeed = {
      live: `/media/live-sub/${token}`, sub, base: offset, bytes: 0, tail: '',
      decoder: new TextDecoder(), busy: false,
    };
  } else {
    return;
  }
  subFeed.timer = setInterval(pollSubtitle, SUB_POLL_MS);
  pollSubtitle();
}

async function pollSubtitle() {
  const feed = subFeed;
  if (!feed || feed.busy) return;
  feed.busy = true;
  try {
    if (feed.url) {
      const res = await fetch(`${feed.url}?from=${feed.from}`, { credentials: 'same-origin' });
      if (!res.ok) return;
      const { cues, next } = await res.json();
      feed.from = next;
      if (feed === subFeed) addCues(feed.sub, cues);
      return;
    }
    const res = await fetch(feed.live, { credentials: 'same-origin', headers: { Range: `bytes=${feed.bytes}-` } });
    if (res.status !== 206 && res.status !== 200) return;
    let chunk = new Uint8Array(await res.arrayBuffer());
    if (res.status === 200) chunk = chunk.subarray(feed.bytes);
    feed.bytes += chunk.byteLength;
    // Only whole cues: the last block may still be being written.
    const text = feed.tail + feed.decoder.decode(chunk, { stream: true });
    const cut = text.lastIndexOf('\n\n');
    if (cut < 0) {
      feed.tail = text;
      return;
    }
    feed.tail = text.slice(cut + 2);
    const cues = parseVtt(text.slice(0, cut)).map(c => ({ ...c, start: c.start + feed.base, end: c.end + feed.base }));
    if (feed === subFeed) addCues(feed.sub, cues);
  } catch {
    // Next round.
  } finally {
    feed.busy = false;
  }
}

function selectSub(next) {
  sid = next;
  buildTrackList();
  renderCues();
  prop('sid', sid);
  const sub = typeof sid === 'number' ? subs.get(sid) : null;
  if (sub?.kind !== 'embedded') {
    stopSubFeed();
  } else if (media?.transport === 'hls') {
    followSubtitle();
  } else if (media) {
    // Out of the file only through ffmpeg: a stream that goes through it.
    if (media.mode === 'direct') media.mode = videoPlays(media.info.video) ? 'copy' : 'h264';
    restartAt(position());
  }
}

// ---------- Playback ----------
function randomToken() {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return [...bytes].map(b => b.toString(16).padStart(2, '0')).join('');
}

function destroyHls() {
  if (hls) {
    hls.destroy();
    hls = null;
  }
}

function closeSession() {
  if (!session) return;
  fetch(`/hls/${session.id}`, { method: 'DELETE', credentials: 'same-origin' }).catch(() => {});
  session = null;
}

function startProgressive(at) {
  offset = Math.max(0, at);
  const q = new URLSearchParams({
    start: offset.toFixed(3),
    audio: String(Math.max(0, aid - 1)),
    video: media.mode,
  });
  const sub = typeof sid === 'number' ? subs.get(sid) : null;
  let token = null;
  if (sub?.kind === 'embedded') {
    token = randomToken();
    q.set('sub', String(sub.index));
    q.set('subfile', token);
  }
  video.src = `${media.url}/remux.mp4?${q}`;
  followSubtitle(token);
}

async function startHls(at) {
  const gen = generation;
  const q = new URLSearchParams({ video: media.mode, audio: String(Math.max(0, aid - 1)), start: at.toFixed(3) });
  let created;
  try {
    const res = await fetch(`${media.url}/hls?${q}`, { method: 'POST', credentials: 'same-origin' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    created = await res.json();
  } catch (err) {
    if (gen !== generation) return;
    console.warn('[player] no HLS session, streaming as one file', err);
    media.transport = 'progressive';
    startStream(at);
    return;
  }
  if (gen !== generation || !media) return;
  if (session && session.id !== created.session) closeSession();
  session = { id: created.session, subtitles: created.subtitles || [] };
  offset = 0;
  const Hls = await loadHlsJs();
  if (gen !== generation || !media) return;
  destroyHls();
  if (Hls) {
    mediaRecovered = false;
    hls = new Hls({
      startPosition: at,
      enableWorker: false,
      maxBufferLength: 60,
      maxMaxBufferLength: 120,
      backBufferLength: 90,
      // A segment may wait for ffmpeg to start over there.
      fragLoadPolicy: {
        default: {
          maxTimeToFirstByteMs: 90000,
          maxLoadTimeMs: 180000,
          timeoutRetry: { maxNumRetry: 2, retryDelayMs: 1000, maxRetryDelayMs: 4000 },
          errorRetry: { maxNumRetry: 6, retryDelayMs: 1000, maxRetryDelayMs: 8000 },
        },
      },
    });
    hls.on(Hls.Events.ERROR, (_event, data) => onHlsError(Hls, data));
    hls.loadSource(created.playlist);
    hls.attachMedia(video);
  } else if (CAN.nativeHls) {
    video.src = created.playlist;
    if (at > 0) video.addEventListener('loadedmetadata', () => { video.currentTime = at; }, { once: true });
  } else {
    media.transport = 'progressive';
    startProgressive(at);
    return;
  }
  video.playbackRate = speed;
  renderCues();
  video.play().catch(() => {});
  followSubtitle();
}

function onHlsError(Hls, data) {
  if (!data.fatal) return;
  console.warn('[player] hls.js', data.type, data.details);
  if (data.type === Hls.ErrorTypes.MEDIA_ERROR && !mediaRecovered && hls) {
    mediaRecovered = true;
    hls.recoverMediaError();
    return;
  }
  if (fallBack()) return;
  emit({ kind: 'end_file', reason: 'error' });
}

function startStream(at) {
  if (!media) return;
  opening = false;
  if (media.transport === 'direct') {
    offset = 0;
    video.src = media.url;
    if (at > 0) video.currentTime = at;
    video.playbackRate = speed;
    renderCues();
    video.play().catch(() => {});
  } else if (media.transport === 'hls') {
    startHls(at);
  } else {
    destroyHls();
    startProgressive(at);
    video.playbackRate = speed;
    renderCues();
    video.play().catch(() => {});
  }
}

function restartAt(at) {
  restarting = true;
  const wasPaused = video.paused;
  if (media && media.mode !== 'direct' && media.transport === 'direct') media.transport = 'hls';
  startStream(at);
  if (wasPaused) video.pause();
}

function seekTo(target) {
  if (!media) return;
  const d = duration();
  target = Math.max(0, d > 0 ? Math.min(target, d - 1) : target);
  if (opening) {
    pendingStart = target;
  } else if (media.transport === 'progressive') {
    restartAt(target);
  } else {
    video.currentTime = target;
  }
  prop('time-pos', target);
}

async function load(url) {
  const gen = ++generation;
  stopSubFeed();
  destroyHls();
  closeSession();
  stopped = false;
  opening = false;
  restarting = false;
  media = null;
  offset = 0;
  subs.clear();
  nextSubId = 1;
  sid = 'no';
  renderCues();
  video.style.visibility = '';
  let info;
  try {
    const res = await fetch(`${url}/info`, { credentials: 'same-origin' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    info = await res.json();
  } catch (err) {
    if (gen === generation) {
      console.warn('[player] cannot read the file', err);
      emit({ kind: 'end_file', reason: 'error' });
    }
    return;
  }
  if (gen !== generation) return;
  const direct = playsAsItIs(info);
  media = {
    url, info,
    mode: direct ? 'direct' : (videoPlays(info.video) ? 'copy' : 'h264'),
    transport: direct ? 'direct' : 'hls',
  };
  aid = info.audios.length ? preferredAudio(info) + 1 : 1;
  for (const s of info.subs || []) {
    if (!TEXT_SUBTITLES.includes(s.codec)) continue;
    subs.set(nextSubId++, {
      kind: 'embedded', index: s.sub_idx, title: s.title || '', lang: s.lang || '', codec: s.codec,
      cues: [], seen: new Set(),
    });
  }
  sid = preferredSub();
  if (sid !== 'no' && media.mode === 'direct') {
    media.mode = videoPlays(info.video) ? 'copy' : 'h264';
    media.transport = 'hls';
  }
  console.info(`[player] ${info.container} ${info.video?.codec || '?'}${info.video?.bit_depth > 8 ? ' 10-bit' : ''}`
    + `${info.video?.dv_profile ? ` Dolby Vision ${info.video.dv_profile}` : ''}, `
    + `audio ${info.audios.map(a => a.codec).join('/') || 'none'}: `
    + (media.mode === 'direct' ? 'as it is' : `HLS, video ${media.mode}`));
  // Dolby Vision with nothing a player without it can show (profile 5): the
  // browser decodes the video, with the colours green and purple.
  if (info.video?.dv_profile && !info.video.dv_compat) showNotice(t('web.player.dolbyVision'));
  else hideNotice();
  buildTrackList();
  prop('aid', aid);
  prop('sid', sid);
  prop('duration', duration());
  prop('video-params', videoParams());
  // The file is open: the interface seeks to where it was left, if anywhere,
  // right away; the stream then starts there instead of at the beginning.
  opening = true;
  pendingStart = 0;
  emit({ kind: 'file_loaded' });
  setTimeout(() => {
    if (gen !== generation || !opening) return;
    // Like mpv, report when the picture starts moving.
    restarting = true;
    startStream(pendingStart);
  }, 60);
}

// What did not play gets another way: the video copied, then transcoded to
// H.264, then as one progressive stream.
function fallBack() {
  if (!media || stopped) return false;
  const at = position();
  if (media.mode === 'direct') {
    media.mode = videoPlays(media.info.video) ? 'copy' : 'h264';
    media.transport = 'hls';
  } else if (media.mode === 'copy') {
    media.mode = 'h264';
  } else if (media.transport === 'hls') {
    media.transport = 'progressive';
  } else {
    return false;
  }
  console.warn(`[player] trying ${media.transport}, video ${media.mode}`);
  restarting = true;
  destroyHls();
  closeSession();
  startStream(at);
  return true;
}

function stop() {
  generation++;
  stopped = true;
  opening = false;
  hideNotice();
  stopSubFeed();
  destroyHls();
  closeSession();
  media = null;
  video.pause();
  video.removeAttribute('src');
  video.load();
  subs.clear();
  sid = 'no';
  renderCues();
}

// ---------- Volume ----------
// Above 100% the video element cannot go: a Web Audio gain takes over.
let gain = null;
function applyVolume() {
  const v = volume / 100;
  if (v > 1 && !gain) {
    try {
      const ctx = new AudioContext();
      const source = ctx.createMediaElementSource(video);
      gain = ctx.createGain();
      source.connect(gain).connect(ctx.destination);
      ctx.resume().catch(() => {});
    } catch (err) {
      console.warn('[player] no volume boost', err);
    }
  }
  if (gain) {
    video.volume = 1;
    gain.gain.value = v;
  } else {
    video.volume = Math.min(1, Math.max(0, v));
  }
}

// ---------- Video element events ----------
video.addEventListener('playing', () => {
  if (restarting) {
    restarting = false;
    emit({ kind: 'playback_restart' });
  }
});
// mpv reports every seek done; a new progressive stream reports above.
video.addEventListener('seeked', () => {
  if (media && media.transport !== 'progressive') emit({ kind: 'playback_restart' });
});
video.addEventListener('play', () => prop('pause', false));
video.addEventListener('pause', () => prop('pause', true));
video.addEventListener('timeupdate', () => {
  if (!media || opening) return;
  const now = performance.now();
  if (now - lastTimeEmit < 250) return;
  lastTimeEmit = now;
  prop('time-pos', position());
  prop('demuxer-cache-time', bufferedAhead());
});
video.addEventListener('ended', () => {
  if (!media) return;
  const d = duration();
  if (media.transport === 'progressive' && d > 0 && position() < d - EOF_MARGIN) {
    // The stream broke off (the source stalled): pick it up again.
    restartAt(position());
    return;
  }
  prop('eof-reached', true);
  emit({ kind: 'end_file', reason: 'eof' });
});
video.addEventListener('error', () => {
  // hls.js reports its own errors.
  if (!media || stopped || hls || !video.getAttribute('src')) return;
  console.warn('[player] video error', video.error?.code, video.error?.message);
  if (fallBack()) return;
  emit({ kind: 'end_file', reason: 'error' });
});

// ---------- mpv commands ----------
function runCommand(args) {
  const [name, ...rest] = args.map(a => (a == null ? '' : String(a)));
  switch (name) {
    case 'seek': {
      const value = Number(rest[0]) || 0;
      const mode = rest[1] || 'relative';
      seekTo(mode.startsWith('absolute') ? value : position() + value);
      return null;
    }
    case 'cycle':
      if (rest[0] === 'pause') setProperty('pause', !video.paused);
      else if (rest[0] === 'mute') setProperty('mute', !video.muted);
      return null;
    case 'add': {
      const delta = Number(rest[1]) || 0;
      if (rest[0] === 'volume') setProperty('volume', volume + delta);
      else if (rest[0] === 'sub-delay') setProperty('sub-delay', subDelay + delta);
      else if (rest[0] === 'speed') setProperty('speed', speed + delta);
      return null;
    }
    case 'stop':
      stop();
      return null;
    case 'sub-add': {
      // sub-add <url> [select|auto|cached] [title] [lang]
      const [url, , title, lang] = rest;
      return addSubtitle(url, title, lang);
    }
    default:
      return null;
  }
}

function setProperty(name, value) {
  switch (name) {
    case 'pause':
      if (value && value !== 'no') video.pause();
      else video.play().catch(() => {});
      return null;
    case 'time-pos':
      seekTo(Number(value) || 0);
      return null;
    case 'volume':
      volume = Math.max(0, Math.min(VOLUME_MAX, Number(value) || 0));
      applyVolume();
      prop('volume', volume);
      return null;
    case 'mute':
      video.muted = !!value && value !== 'no';
      prop('mute', video.muted);
      return null;
    case 'speed':
      speed = Math.max(0.25, Math.min(4, Number(value) || 1));
      video.playbackRate = speed;
      prop('speed', speed);
      return null;
    case 'sub-delay':
      subDelay = Math.round((Number(value) || 0) * 1000) / 1000;
      renderCues();
      prop('sub-delay', subDelay);
      return null;
    case 'aid': {
      const next = value === 'no' || value === false ? aid : Number(value);
      if (!media || !Number.isFinite(next) || next === aid) return null;
      aid = next;
      buildTrackList();
      prop('aid', aid);
      // One audio track per stream: another one needs a new stream (for
      // HLS, a new session from the same second).
      if (media.mode !== 'direct' && !opening) restartAt(position());
      return null;
    }
    case 'sid': {
      const next = value === 'no' || value === false ? 'no' : Number(value);
      if (next !== 'no' && !subs.has(next)) return null;
      if (next !== sid) selectSub(next);
      return null;
    }
    default:
      return null;
  }
}

function observe(name) {
  observed.add(name);
  // mpv reports the current value of a property as soon as it is observed.
  emit({ kind: 'property_changed', name, value: current(name) });
}

window.__SIIISHUB_PLAYER__ = {
  async command(command, args = {}) {
    switch (command) {
      case 'mpv_load':
        load(args.url);
        return null;
      case 'mpv_command':
        return runCommand(Array.isArray(args.args) ? args.args : []);
      case 'mpv_set_property':
        return setProperty(args.name, args.value);
      case 'mpv_get_property':
        return current(args.name);
      case 'mpv_observe':
        observe(args.name);
        return null;
      case 'mpv_set_visible':
        video.style.visibility = args.visible ? '' : 'hidden';
        if (!args.visible && !stopped) stop();
        return null;
      case 'mpv_set_geometry':
        // The video lives in #playerVideo and follows its size by itself.
        return null;
      default:
        throw `unsupported player command: ${command}`;
    }
  },
};
