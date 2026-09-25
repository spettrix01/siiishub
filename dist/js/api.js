import { state, tmdbType, TMDB_API } from './state.js';
import { tmdbLang } from './i18n.js';

const TAURI = window.__TAURI__;
const invoke = TAURI?.core?.invoke ?? (() => {
  throw new Error('Tauri runtime missing — open this app via the desktop binary, not a browser');
});

function applyPublicSettings(s) {
  state.settings = s;
  state.settings.tmdbKey = s.tmdb_key || '';
  state.settings.rdAvailable = s.rd_available || false;
  state.settings.debridConfigured = !!s.debrid_configured;
  state.settings.debridProvider = s.debrid_provider || '';
  state.settings.debridToken = s.debrid_token || '';
  state.settings.debridTokenSet = !!(s.debrid_token && s.debrid_token.length > 0);
  state.settings.tracker_fallbacks = s.tracker_fallbacks || [];
  state.settings.playerAudioLangs = Array.isArray(s.player_audio_langs) ? s.player_audio_langs : [];
  state.settings.playerSubLangs = Array.isArray(s.player_sub_langs) ? s.player_sub_langs : [];
  state.settings.remotePort = Number(s.remote_port) || 9871;
  state.settings.language = s.language || 'eng';
  state.settings.downloadDir = s.download_dir || '';
  return state.settings;
}

export async function loadSettings() {
  try {
    return applyPublicSettings(await invoke('settings_get'));
  } catch {
    return applyPublicSettings({ tmdb_key: '', addons: [], rd_available: false, tracker_fallbacks: [] });
  }
}

export async function saveSettings(patch) {
  const out = {};
  if ('tmdbKey' in patch) out.tmdb_key = patch.tmdbKey;
  if ('addons' in patch) out.addons = patch.addons;
  if ('rdToken' in patch) out.rd_token = patch.rdToken;
  if ('debridProvider' in patch) out.debrid_provider = patch.debridProvider;
  if ('debridToken' in patch) out.debrid_token = patch.debridToken;
  if ('trackerFallbacks' in patch) out.tracker_fallbacks = patch.trackerFallbacks;
  if ('playerAudioLangs' in patch) out.player_audio_langs = patch.playerAudioLangs;
  if ('playerSubLangs' in patch) out.player_sub_langs = patch.playerSubLangs;
  if ('remotePort' in patch) out.remote_port = Number(patch.remotePort) || 9871;
  if ('language' in patch) out.language = patch.language;
  return applyPublicSettings(await invoke('settings_save', { patch: out }));
}

/** The version of SIIISHUB, empty if the backend does not tell it. */
export async function appVersion() {
  try {
    return (await invoke('app_version')) || '';
  } catch {
    return '';
  }
}

export async function remoteInfo() {
  try {
    return await invoke('remote_info');
  } catch {
    return { running: false, port: 9871, interfaces: [], clients: 0, devices: [] };
  }
}

export async function remotePushState(payload) {
  try {
    await invoke('remote_push_state', { payload });
  } catch {}
}

export function onRemoteCmd(handler) {
  return TAURI.event.listen('remote://cmd', e => handler(e.payload));
}

export function onRemoteClientCount(handler) {
  return TAURI.event.listen('remote://client-count', e => handler(e.payload));
}

export async function remoteSetApproval(id, approved) {
  try {
    await invoke('remote_set_approval', { args: { id, approved: !!approved } });
  } catch (e) {
    console.warn('remote_set_approval failed', e);
  }
}

export async function remoteRememberDevice(id) {
  try {
    await invoke('remote_remember_device', { args: { id } });
  } catch (e) {
    console.warn('remote_remember_device failed', e);
  }
}

export async function remoteForgetDevice(id, key) {
  try {
    await invoke('remote_forget_device', { args: { id: id ?? null, key: key ?? null } });
  } catch (e) {
    console.warn('remote_forget_device failed', e);
  }
}

export function onRemotePending(handler) {
  return TAURI.event.listen('remote://pending', e => handler(e.payload));
}

// The apps' account: the SIIISHUB server they sync with (account.rs).
export const syncStatus = () => invoke('sync_status');
export const syncSignIn = (server, username, password, device) =>
  invoke('sync_sign_in', { server, username, password, device });
export const syncSignOut = () => invoke('sync_sign_out');

export function onSyncChanged(handler) {
  return TAURI.event.listen('sync://changed', e => handler(e.payload));
}

// The browser version's accounts (server/accounts.rs).
export const accountStatus = () => invoke('account_status');
export const accountsList = () => invoke('accounts_list');
export const accountCreate = (username, password) => invoke('account_create', { username, password });
export const accountDelete = (id) => invoke('account_delete', { id });
export const accountPassword = (current, next) => invoke('account_password', { current, next });

export async function openDownloadDir() {
  return invoke('open_download_dir');
}

export async function tmdb(path, params = {}) {
  if (!state.settings.tmdbKey) throw new Error('NO_KEY');
  const url = new URL(TMDB_API + path);
  url.searchParams.set('api_key', state.settings.tmdbKey);
  url.searchParams.set('language', tmdbLang());
  for (const [k, v] of Object.entries(params)) {
    if (v != null && v !== '') url.searchParams.set(k, v);
  }
  const r = await fetch(url);
  if (!r.ok) throw new Error(`TMDB ${r.status}`);
  return r.json();
}

export async function loadGenres() {
  if (!state.settings.tmdbKey) return;
  try {
    const [m, t] = await Promise.all([
      tmdb('/genre/movie/list'),
      tmdb('/genre/tv/list'),
    ]);
    state.genres.movie = m.genres || [];
    state.genres.tv = t.genres || [];
  } catch (e) {
    console.warn('genres failed', e);
  }
}

export async function fetchTrending(type, window = 'day') {
  return tmdb(`/trending/${type}/${window}`);
}

export async function fetchPage(page) {
  const type = tmdbType();
  if (state.query) {
    return tmdb(`/search/${type}`, { query: state.query, page, include_adult: 'false' });
  }
  const today = new Date().toISOString().slice(0, 10);
  const params = {
    page,
    sort_by: type === 'movie' ? 'primary_release_date.desc' : 'first_air_date.desc',
    include_adult: 'false',
    'vote_count.gte': 5,
  };
  if (type === 'movie') params['primary_release_date.lte'] = today;
  else params['first_air_date.lte'] = today;
  if (typeof state.selectedGenre === 'string' && state.selectedGenre.startsWith('lang:')) {
    params.with_original_language = state.selectedGenre.slice(5);
  } else if (state.selectedGenre) {
    params.with_genres = state.selectedGenre;
  }
  return tmdb(`/discover/${type}`, params);
}

export async function fetchStreams(type, id) {
  return invoke('streams_fetch', { kind: type, id });
}

export async function fetchSubtitles(type, id) {
  return invoke('subtitles_fetch', { kind: type, id });
}

const tvSeasonCache = new Map();
export function fetchTvSeason(tvId, seasonNumber) {
  const key = `${tvId}:${seasonNumber}`;
  let p = tvSeasonCache.get(key);
  if (!p) {
    p = tmdb(`/tv/${tvId}/season/${seasonNumber}`).catch(e => {
      tvSeasonCache.delete(key);
      throw e;
    });
    tvSeasonCache.set(key, p);
  }
  return p;
}

function makeRequestId() {
  if (crypto.randomUUID) return crypto.randomUUID();
  return `${Date.now().toString(16)}-${Math.random().toString(16).slice(2)}`;
}

export async function rdPlay({ url, infoHash, displayName, fileHint, sources, addon, useDebrid = false }, { signal } = {}) {
  if (signal?.aborted) throw new DOMException('The operation was aborted.', 'AbortError');
  const requestId = makeRequestId();
  const onAbort = () => { invoke('media_cancel', { requestId }).catch(() => {}); };
  signal?.addEventListener('abort', onAbort, { once: true });
  try {
    return await invoke('media_resolve', {
      args: {
        url: url || null,
        infoHash: infoHash || null,
        displayName: displayName || null,
        fileHint: fileHint || null,
        sources: sources || [],
        addon: addon || null,
        useDebrid: !!useDebrid,
        requestId,
      },
    });
  } finally {
    signal?.removeEventListener('abort', onAbort);
  }
}

export async function rdResolveAll({ infoHash, displayName, sources }, { signal } = {}) {
  if (signal?.aborted) throw new DOMException('The operation was aborted.', 'AbortError');
  const requestId = makeRequestId();
  const onAbort = () => { invoke('media_cancel', { requestId }).catch(() => {}); };
  signal?.addEventListener('abort', onAbort, { once: true });
  try {
    return await invoke('media_resolve_all', {
      args: {
        url: null,
        infoHash: infoHash || null,
        displayName: displayName || null,
        fileHint: null,
        sources: sources || [],
        useDebrid: true,
        requestId,
      },
    });
  } finally {
    signal?.removeEventListener('abort', onAbort);
  }
}

export async function fetchTorrentStats(infoHash) {
  if (!infoHash) return null;
  try {
    return await invoke('torrent_stats', { infoHash });
  } catch {
    return null;
  }
}

export async function destroyTorrentSession(infoHash) {
  if (!infoHash) return;
  try {
    await invoke('session_destroy', { infoHash });
  } catch {}
}

export async function downloadStart(args) {
  return invoke('download_start', { args });
}

export async function downloadStartGroup(args) {
  return invoke('download_start_group', { args });
}

export async function downloadAddLocal(path) {
  return invoke('download_add_local', { path });
}

export async function downloadList() {
  try {
    return await invoke('download_list');
  } catch {
    return [];
  }
}

export async function downloadRemove(id) {
  if (!id) return;
  try {
    await invoke('download_remove', { id });
  } catch {}
}

export async function downloadPause(id) {
  if (!id) return;
  try { await invoke('download_pause', { id }); } catch (e) { console.warn('download_pause failed', e); }
}

export async function downloadResume(id) {
  if (!id) return;
  try { await invoke('download_resume', { id }); } catch (e) { console.warn('download_resume failed', e); }
}

export async function downloadPlay(id, fileIndex = null) {
  const args = { id };
  if (fileIndex != null) args.fileIndex = fileIndex;
  return invoke('download_play', args);
}

export async function downloadFiles(id) {
  try {
    return await invoke('download_files', { id });
  } catch {
    return [];
  }
}

export async function downloadOpenFolder(id) {
  if (!id) return;
  try {
    await invoke('download_open_folder', { id });
  } catch (e) {
    console.warn('download_open_folder failed', e);
  }
}

export async function torrentParseFile(bytes) {
  return invoke('torrent_parse_file', { bytes: Array.from(bytes) });
}

export async function fetchAddonMeta(url) {
  return invoke('addon_meta', { url });
}

export async function mpvLoad(url, options) {
  return invoke('mpv_load', { url, options: options || null });
}

export async function mpvCommand(args) {
  return invoke('mpv_command', { args });
}

export async function mpvSet(name, value) {
  return invoke('mpv_set_property', { name, value });
}

export async function mpvObserve(name) {
  return invoke('mpv_observe', { name });
}

export async function mpvSetGeometry(x, y, w, h) {
  return invoke('mpv_set_geometry', {
    x: Math.round(x),
    y: Math.round(y),
    w: Math.round(w),
    h: Math.round(h),
  });
}

export async function mpvSetVisible(visible) {
  return invoke('mpv_set_visible', { visible: !!visible });
}

export async function windowSetFullscreen(fullscreen) {
  return invoke('window_set_fullscreen', { fullscreen: !!fullscreen });
}

export function onMpvEvent(handler) {
  return TAURI.event.listen('mpv://event', e => handler(e.payload));
}

export function onMediaProgress(handler) {
  return TAURI.event.listen('media://progress', e => handler(e.payload));
}
