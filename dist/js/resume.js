import { safeJsonParse } from './dom.js';
import { userStore } from './userstore.js';

const KEY_PREFIX = 'siiis:resume:';
const TTL_MS = 30 * 24 * 60 * 60 * 1000;
const MIN_SAVE_TIME = 5;
const MAX_SAVE_PROGRESS = 0.95;

function notify(ctx) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent('siiis:resume', { detail: ctx || null }));
}

function keyOf(ctx) {
  if (!ctx) return null;
  if (ctx.type === 'tv') {
    if (ctx.tmdbId == null || ctx.season == null || ctx.episode == null) return null;
    return `${KEY_PREFIX}tv:${ctx.tmdbId}:${ctx.season}:${ctx.episode}`;
  }
  if (ctx.type === 'movie') {
    if (ctx.tmdbId == null) return null;
    return `${KEY_PREFIX}movie:${ctx.tmdbId}`;
  }
  // Local files / manual downloads have no TMDB id: key on the stable
  // download id (a hash of the file path) so playback still resumes.
  if (ctx.type === 'local') {
    if (ctx.id == null || ctx.id === '') return null;
    return `${KEY_PREFIX}local:${ctx.id}`;
  }
  return null;
}

export function saveResume(ctx, time, duration) {
  const k = keyOf(ctx);
  if (!k) return;
  if (!Number.isFinite(time) || !Number.isFinite(duration) || duration <= 0) return;
  if (time < MIN_SAVE_TIME) return;
  if (time / duration > MAX_SAVE_PROGRESS) {
    userStore.removeItem(k);
    notify(ctx);
    return;
  }
  userStore.setItem(k, JSON.stringify({ t: time, d: duration, ts: Date.now() }));
  notify(ctx);
}

export function getResume(ctx) {
  const k = keyOf(ctx);
  if (!k) return null;
  const data = safeJsonParse(userStore.getItem(k));
  if (!data || !Number.isFinite(data.t) || !Number.isFinite(data.d)) return null;
  if (Date.now() - (data.ts || 0) > TTL_MS) {
    userStore.removeItem(k);
    return null;
  }
  if (data.t / data.d > MAX_SAVE_PROGRESS) {
    userStore.removeItem(k);
    return null;
  }
  return { time: data.t, duration: data.d };
}

export function listResume(limit = 20) {
  const movies = [];
  const tvByShow = new Map();
  const count = userStore.length;
  for (let i = 0; i < count; i++) {
    const k = userStore.key(i);
    if (!k || !k.startsWith(KEY_PREFIX)) continue;
    const data = safeJsonParse(userStore.getItem(k));
    if (!data || !Number.isFinite(data.t) || !Number.isFinite(data.d) || data.d <= 0) continue;
    if (Date.now() - (data.ts || 0) > TTL_MS) continue;
    if (data.t / data.d > MAX_SAVE_PROGRESS) continue;
    const rest = k.slice(KEY_PREFIX.length);
    const parts = rest.split(':');
    const ts = data.ts || 0;
    if (parts[0] === 'movie' && parts.length === 2) {
      movies.push({ type: 'movie', tmdbId: Number(parts[1]), time: data.t, duration: data.d, ts });
    } else if (parts[0] === 'tv' && parts.length === 4) {
      const tmdbId = Number(parts[1]);
      const cur = tvByShow.get(tmdbId);
      if (!cur || ts > cur.ts) {
        tvByShow.set(tmdbId, {
          type: 'tv',
          tmdbId,
          season: Number(parts[2]),
          episode: Number(parts[3]),
          time: data.t,
          duration: data.d,
          ts,
        });
      }
    }
  }
  const all = movies.concat([...tvByShow.values()]);
  all.sort((a, b) => b.ts - a.ts);
  return all.slice(0, limit);
}

export function getCardProgress(type, tmdbId) {
  if (tmdbId == null) return null;
  if (type === 'movie') {
    const r = getResume({ type: 'movie', tmdbId });
    return r ? r.time / r.duration : null;
  }
  if (type === 'tv') {
    const prefix = `${KEY_PREFIX}tv:${tmdbId}:`;
    let latest = null;
    let latestTs = 0;
    const count = userStore.length;
    for (let i = 0; i < count; i++) {
      const k = userStore.key(i);
      if (!k || !k.startsWith(prefix)) continue;
      const data = safeJsonParse(userStore.getItem(k));
      if (!data || !Number.isFinite(data.t) || !Number.isFinite(data.d) || data.d <= 0) continue;
      if (Date.now() - (data.ts || 0) > TTL_MS) continue;
      if (data.t / data.d > MAX_SAVE_PROGRESS) continue;
      if ((data.ts || 0) > latestTs) {
        latestTs = data.ts || 0;
        latest = data;
      }
    }
    return latest ? latest.t / latest.d : null;
  }
  return null;
}
