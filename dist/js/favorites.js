import { safeJsonParse } from './dom.js';
import { userStore } from './userstore.js';

const KEY_PREFIX = 'siiis:favorite:';

function notify(ctx) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent('siiis:favorite', { detail: ctx || null }));
}

function keyOf(ctx) {
  if (!ctx || ctx.tmdbId == null) return null;
  if (ctx.type !== 'movie' && ctx.type !== 'tv') return null;
  return `${KEY_PREFIX}${ctx.type}:${ctx.tmdbId}`;
}

export function isFavorite(ctx) {
  const k = keyOf(ctx);
  if (!k) return false;
  return userStore.getItem(k) != null;
}

function addFavorite(ctx) {
  const k = keyOf(ctx);
  if (!k) return;
  userStore.setItem(k, JSON.stringify({ ts: Date.now() }));
  notify(ctx);
}

function removeFavorite(ctx) {
  const k = keyOf(ctx);
  if (!k) return;
  userStore.removeItem(k);
  notify(ctx);
}

export function toggleFavorite(ctx) {
  if (isFavorite(ctx)) {
    removeFavorite(ctx);
    return false;
  }
  addFavorite(ctx);
  return true;
}

export function listFavorites(limit = 100) {
  const out = [];
  const count = userStore.length;
  for (let i = 0; i < count; i++) {
    const k = userStore.key(i);
    if (!k || !k.startsWith(KEY_PREFIX)) continue;
    const data = safeJsonParse(userStore.getItem(k));
    const rest = k.slice(KEY_PREFIX.length);
    const sep = rest.indexOf(':');
    if (sep < 0) continue;
    const type = rest.slice(0, sep);
    const tmdbId = Number(rest.slice(sep + 1));
    if ((type !== 'movie' && type !== 'tv') || !Number.isFinite(tmdbId)) continue;
    out.push({ type, tmdbId, ts: data?.ts || 0 });
  }
  out.sort((a, b) => b.ts - a.ts);
  return out.slice(0, limit);
}
