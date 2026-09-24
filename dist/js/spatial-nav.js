import { $, $$ } from './dom.js';

const FOCUS_CLASS = 'snav-focus';

const FOCUSABLE = [
  'a.card:not(.skeleton)',
  '.trending-card:not(.is-skeleton)',
  '.tab',
  '.seg-btn',
  '.brand',
  '.genre-trigger',
  '.dl-row-btn',
  'button:not([disabled])',
  'a[href]',
  '[role="option"]',
  '[role="slider"]',
  'input:not([type="hidden"]):not([disabled])',
  'textarea:not([disabled])',
  'select:not([disabled])',
].join(',');

let current = null;
let lastBaseFocus = null;

function topModal() {
  for (const sel of ['#alertModal', '#detailsModal', '#settingsModal']) {
    const m = $(sel);
    if (m && !m.hidden) return m;
  }
  return null;
}

function activeScope() {
  // The Android popup editing a text field (android-inputs.js).
  const inputPopup = $('.input-popup');
  if (inputPopup) return inputPopup;
  const openMenu = $('[data-pick-menu]:not([hidden])');
  if (openMenu) return openMenu;
  const genreMenu = $('#genreMenu');
  if (genreMenu && !genreMenu.hidden) return genreMenu;
  const alert = $('#alertModal');
  if (alert && !alert.hidden) return alert;
  const player = $('#playerModal');
  if (player && !player.hidden) {
    const psettings = $('#playerSettings');
    if (psettings && !psettings.hidden) return psettings;
    return player;
  }
  const settings = $('#settingsModal');
  if (settings && !settings.hidden) return settings;
  const details = $('#detailsModal');
  if (details && !details.hidden) return details;
  return document.body;
}

function isVisible(el) {
  if (!el || !el.isConnected) return false;
  if (el.closest('[hidden]')) return false;
  const r = el.getBoundingClientRect();
  if (r.width < 3 || r.height < 3) return false;
  const cs = getComputedStyle(el);
  if (cs.visibility === 'hidden' || cs.display === 'none') return false;
  return true;
}

function candidates(scope) {
  return $$(FOCUSABLE, scope).filter(el => isVisible(el) && !el.closest('#playerSidePane'));
}

function clearFocus() {
  if (current) current.classList.remove(FOCUS_CLASS);
  current = null;
}

// A move keeps the focus in the middle of its scroller along the direction
// of travel (rows centred going up and down, cards centred in a rail going
// sideways), so what comes next is always in view; focusing without a move
// scrolls as little as possible.
function setFocus(el, dir = null) {
  if (current && current !== el) current.classList.remove(FOCUS_CLASS);
  current = el || null;
  if (!current) return;
  current.classList.add(FOCUS_CLASS);
  if (activeScope() === document.body) lastBaseFocus = current;
  const vertical = dir === 'up' || dir === 'down';
  const horizontal = dir === 'left' || dir === 'right';
  try {
    current.scrollIntoView({
      block: vertical ? 'center' : 'nearest',
      inline: horizontal ? 'center' : 'nearest',
      behavior: 'smooth',
    });
  } catch {
    current.scrollIntoView();
  }
}

function gap(aMin, aMax, bMin, bMax) {
  if (bMax < aMin) return aMin - bMax;
  if (bMin > aMax) return bMin - aMax;
  return 0;
}

// Whether `r` lies entirely in the direction of the move from `cr`, edge to
// edge. Comparing centres alone let a neighbour in the same row win a move up
// or down: cards whose titles wrap on a different number of lines have
// centres a few pixels apart.
const EDGE_SLACK = 2;
function isAhead(dir, cr, r) {
  if (dir === 'up') return r.bottom <= cr.top + EDGE_SLACK;
  if (dir === 'down') return r.top >= cr.bottom - EDGE_SLACK;
  if (dir === 'left') return r.right <= cr.left + EDGE_SLACK;
  return r.left >= cr.right - EDGE_SLACK;
}

// Scrollers around `el` within `scope`, innermost first. A move stays in the
// innermost one that has something in its direction: otherwise the row above,
// scrolled out of view, lost to the topbar that is always on screen.
function scrollersOf(el, scope) {
  const out = [];
  for (let p = el.parentElement; p && p !== scope && p !== document.body; p = p.parentElement) {
    const cs = getComputedStyle(p);
    if (/auto|scroll/.test(cs.overflowY + cs.overflowX)) out.push(p);
  }
  return out;
}

function pickInitial(list) {
  const vh = window.innerHeight || 800;
  const inView = list.filter(el => {
    const r = el.getBoundingClientRect();
    return r.bottom > 0 && r.top < vh;
  });
  const pool = inView.length ? inView : list;
  return pool.find(el => el.matches('.streams-pick-option.is-active, .settings-nav-item.is-active, #playerProgress, .play-btn, [data-play], .dossier-actions button, [data-rd-play], a.card, .trending-card'))
    || pool[0]
    || null;
}

function move(dir) {
  const scope = activeScope();
  if (!scope) return;
  const list = candidates(scope);
  if (!list.length) return;

  if (!current || !list.includes(current) || !isVisible(current)) {
    let start = null;
    if (scope === document.body && lastBaseFocus &&
        list.includes(lastBaseFocus) && isVisible(lastBaseFocus)) {
      start = lastBaseFocus;
    }
    setFocus(start || pickInitial(list));
    return;
  }

  const axis = current.getAttribute('data-snav-axis');
  if ((axis === 'x' && (dir === 'left' || dir === 'right')) ||
      (axis === 'y' && (dir === 'up' || dir === 'down'))) {
    current.dispatchEvent(new CustomEvent('snav-adjust', { detail: dir }));
    return;
  }

  const cr = current.getBoundingClientRect();
  const cx = cr.left + cr.width / 2;
  const cy = cr.top + cr.height / 2;
  let best = null;
  let bestScore = Infinity;

  // Elements entirely ahead, in the innermost scroller that has some; with
  // nothing entirely ahead (overlapping layouts) the centres decide. Sideways
  // moves stay in the row the focused element shows on screen (a card half
  // scrolled under the topbar only counts for its part in view): at the end
  // of the row the focus stays put instead of reaching the topbar or the
  // window controls.
  const scrollers = scrollersOf(current, scope);
  const sideways = dir === 'left' || dir === 'right';
  let rowTop = cr.top;
  let rowBottom = cr.bottom;
  for (const box of scrollers) {
    const b = box.getBoundingClientRect();
    rowTop = Math.max(rowTop, b.top);
    rowBottom = Math.min(rowBottom, b.bottom);
  }
  if (rowBottom - rowTop < 1) {
    rowTop = cr.top;
    rowBottom = cr.bottom;
  }
  const others = list.filter(el => el !== current)
    .map(el => ({ el, r: el.getBoundingClientRect() }))
    .filter(({ r }) => !sideways || Math.min(rowBottom, r.bottom) - Math.max(rowTop, r.top) > 1);
  const ahead = others.filter(({ r }) => isAhead(dir, cr, r));
  let pool = ahead;
  for (const box of scrollers) {
    const inside = ahead.filter(({ el }) => box.contains(el));
    if (inside.length) {
      pool = inside;
      break;
    }
  }
  for (const { el, r } of pool.length ? pool : others) {
    const ex = r.left + r.width / 2;
    const ey = r.top + r.height / 2;
    let primary, offset;
    if (dir === 'right') {
      if (ex - cx <= 1) continue;
      primary = ex - cx;
      offset = gap(cr.top, cr.bottom, r.top, r.bottom);
    } else if (dir === 'left') {
      if (cx - ex <= 1) continue;
      primary = cx - ex;
      offset = gap(cr.top, cr.bottom, r.top, r.bottom);
    } else if (dir === 'down') {
      if (ey - cy <= 1) continue;
      primary = ey - cy;
      offset = gap(cr.left, cr.right, r.left, r.right);
    } else if (dir === 'up') {
      if (cy - ey <= 1) continue;
      primary = cy - ey;
      offset = gap(cr.left, cr.right, r.left, r.right);
    } else {
      return;
    }
    const score = primary + offset * 3;
    if (score < bestScore) {
      bestScore = score;
      best = el;
    }
  }
  if (best) setFocus(best, dir);
}

function activate() {
  const scope = activeScope();
  if (!scope) return;
  if (!current || !isVisible(current) || !scope.contains(current)) {
    move('down');
    return;
  }
  if (current.matches('input, textarea, select')) {
    current.focus();
    return;
  }
  current.click();
}

function back() {
  const openPickTrigger = $('.streams-pick.is-open [data-pick-trigger]');
  if (openPickTrigger) {
    openPickTrigger.click();
    return true;
  }
  const m = topModal();
  if (!m) return false;
  const closer = m.querySelector('.modal-close, .dossier-close, [data-alert-ok]');
  if (closer) closer.click();
  else m.hidden = true;
  clearFocus();
  return true;
}

function home() {
  const openPickTrigger = $('.streams-pick.is-open [data-pick-trigger]');
  if (openPickTrigger) openPickTrigger.click();
  for (const sel of ['#alertModal', '#detailsModal', '#settingsModal']) {
    const m = $(sel);
    if (m && !m.hidden) {
      const closer = m.querySelector('.modal-close, .dossier-close, [data-alert-ok]');
      if (closer) closer.click();
      else m.hidden = true;
    }
  }
  $('.tab[data-section="movie"]')?.click();
  const scroller = $('#page-scroll') || document.scrollingElement;
  try {
    scroller?.scrollTo({ top: 0, behavior: 'smooth' });
  } catch {
    if (scroller) scroller.scrollTop = 0;
  }
  clearFocus();
}

const detailsModal = $('#detailsModal');
const detailsBody = $('#detailsBody');
if (detailsModal && detailsBody) {
  const player = $('#playerModal');
  const obs = new MutationObserver(() => {
    if (detailsModal.hidden || (player && !player.hidden)) return;
    if (current && (detailsModal.contains(current) || current.closest('[data-pick-menu], #alertModal'))
        && isVisible(current)) return;
    const playBtn = detailsBody.querySelector('.play-btn');
    if (playBtn && isVisible(playBtn)) setFocus(playBtn);
  });
  obs.observe(detailsBody, { childList: true, subtree: true });
}

document.addEventListener('pointerdown', () => {
  if (current) clearFocus();
}, true);

// A picker closed with the D-pad gives the selection back to its button: the
// options are redrawn on a choice, and the selection would be lost.
document.addEventListener('streampicker:close', (e) => {
  const source = e.detail?.source;
  if (!current || !source || (current.isConnected && !source.contains(current))) return;
  const trigger = source.querySelector('[data-pick-trigger]');
  if (trigger && isVisible(trigger)) setFocus(trigger);
});

// The same for the genre menu, whose options are redrawn on a choice too.
const genreMenu = $('#genreMenu');
if (genreMenu) {
  new MutationObserver(() => {
    if (!genreMenu.hidden || !current || (current.isConnected && !genreMenu.contains(current))) return;
    const trigger = $('#genreTrigger');
    if (trigger && isVisible(trigger)) setFocus(trigger);
  }).observe(genreMenu, { attributes: true, attributeFilter: ['hidden'] });
}

export const spatialNav = { move, activate, back, home, clear: clearFocus, focus: setFocus };
