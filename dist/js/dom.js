import { IS_ANDROID } from './platform.js';

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

const HTML_ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
export function escapeHTML(s) {
  return String(s ?? '').replace(/[&<>"']/g, c => HTML_ESC[c]);
}

export const FORM_TAGS = ['INPUT', 'TEXTAREA', 'SELECT'];

export function safeJsonParse(raw, fallback = null) {
  if (raw == null) return fallback;
  try { return JSON.parse(raw); } catch { return fallback; }
}

export function openExternal(url) {
  if (!url) return;
  // Android: the shell plugin's open spawns a desktop opener that does not
  // exist on the phone; the app command hands the URL to the system browser.
  if (IS_ANDROID && window.__TAURI__?.core?.invoke) {
    window.__TAURI__.core.invoke('open_url', { url }).catch(e => console.warn('open_url failed', e));
    return;
  }
  const opener = window.__TAURI__?.shell?.open || window.__TAURI__?.opener?.openUrl;
  if (opener) opener(url).catch(() => {});
  else window.open(url, '_blank', 'noreferrer');
}

// Android: the WebView cannot open windows of its own, so plain links meant
// for a new window (target="_blank", like the TMDB "find your key" link) go
// to the system browser as well. Capture phase, and the click stops here:
// the shell plugin also handles these links (on body, before any document
// listener), but its opener does not exist on the phone.
if (IS_ANDROID) {
  document.addEventListener('click', e => {
    const a = e.target instanceof Element ? e.target.closest('a[target="_blank"][href]') : null;
    const href = a?.getAttribute('href') || '';
    if (!/^https?:/i.test(href)) return;
    e.preventDefault();
    e.stopPropagation();
    openExternal(href);
  }, true);
}

export const CHECK_SVG = `<svg class="check" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="m5 12 5 5L20 7"/></svg>`;
export const COPY_SVG = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="9" y="9" width="11" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M5 15V6a2 2 0 0 1 2-2h9" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>`;

export const DOTS_HTML = `<span></span><span></span><span></span>`;

export async function copyToClipboard(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try { ok = document.execCommand('copy'); } catch {}
    document.body.removeChild(ta);
    return ok;
  }
}
