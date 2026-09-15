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
  const openMenu = $('[data-pick-menu]:not([hidden])');
  if (openMenu) return openMenu;
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

function setFocus(el) {
  if (current && current !== el) current.classList.remove(FOCUS_CLASS);
  current = el || null;
  if (!current) return;
  current.classList.add(FOCUS_CLASS);
  if (activeScope() === document.body) lastBaseFocus = current;
  try {
    current.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'smooth' });
  } catch {
    current.scrollIntoView();
  }
}

function gap(aMin, aMax, bMin, bMax) {
  if (bMax < aMin) return aMin - bMax;
  if (bMin > aMax) return bMin - aMax;
  return 0;
}

function pickInitial(list) {
  const vh = window.innerHeight || 800;
  const inView = list.filter(el => {
    const r = el.getBoundingClientRect();
    return r.bottom > 0 && r.top < vh;
  });
  const pool = inView.length ? inView : list;
  return pool.find(el => el.matches('.streams-pick-option.is-active, #playerProgress, .play-btn, [data-play], a.card, .trending-card'))
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

  for (const el of list) {
    if (el === current) continue;
    const r = el.getBoundingClientRect();
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
  if (best) setFocus(best);
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
    clearFocus();
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
  const obs = new MutationObserver(() => {
    if (detailsModal.hidden) return;
    if (current && detailsModal.contains(current) && isVisible(current)) return;
    const playBtn = detailsBody.querySelector('.play-btn');
    if (playBtn && isVisible(playBtn)) setFocus(playBtn);
  });
  obs.observe(detailsBody, { childList: true, subtree: true });
}

document.addEventListener('pointerdown', () => {
  if (current) clearFocus();
}, true);

export const spatialNav = { move, activate, back, home };
