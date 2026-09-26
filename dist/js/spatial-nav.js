// D-pad navigation, for the phone remote (player.js) and Android TV
// (tv-nav.js): it moves the selection (`.snav-focus`) the way a TV interface
// does. The open layer (activeScope) is divided into zones (ZONES). In a row
// (the tabs, a rail of posters, the buttons of a stream) left and right go
// along it; a box (the grid of posters, a list, a section of the settings)
// is made of rows, and up and down go to the next row keeping the column. At
// the edge of a zone the move carries on in the zone around it, to its next
// part that way. Coming into a zone, the selection goes back to what it had
// selected (REMEMBER), or to its current choice (PREFERRED), or to its first
// item (FROM_TOP), else to the item nearest the one left behind.
import { $, $$ } from './dom.js';
import { IS_TV } from './platform.js';

const FOCUS_CLASS = 'snav-focus';
// On the ancestors of the selection, as :focus-within (without :has, which
// older Fire TV and Android TV WebViews lack).
const WITHIN_CLASS = 'snav-within';

const FOCUSABLE = [
  'a.card:not(.skeleton)',
  '.trending-card:not(.is-skeleton)',
  '.tab',
  '.seg-btn',
  '.brand',
  '.genre-trigger',
  '.dl-row-btn',
  '.alert-select-item',
  'button:not([disabled])',
  'a[href]',
  '[role="option"]',
  '[role="slider"]',
  'input:not([type="hidden"]):not([disabled])',
  'textarea:not([disabled])',
  'select:not([disabled])',
].join(',');

// Never selected: the side pane of the player; the arrows of the rail, which
// the selection scrolls itself; the window buttons; the checkbox of a line in
// a list, the whole line being selected instead.
const EXCLUDED = '#playerSidePane, .trending-nav, .winctl, .alert-select-item input';

// Zones and how their parts are laid out.
const ZONES = [
  // The page
  ['.topbar', 'box'],
  ['.tabs', 'row'],
  ['.searchbox', 'row'],
  ['.trending-rail', 'row'],
  ['.filter-bar', 'row'],
  ['.seg', 'row'],
  ['.dl-magnet-bar-inner', 'box'],
  ['.dl-magnet-row', 'row'],
  ['#grid', 'box'],
  ['.dl-rows', 'box'],
  ['.dl-row', 'row'],
  ['a.card[data-group-key]', 'row'],
  // A film or a series
  ['.dossier-actions', 'row'],
  ['.dossier-cast', 'row'],
  ['.streams-picker', 'row'],
  ['.streams-filter-slot', 'row'],
  ['.stream-list', 'box'],
  ['.stream-actions', 'row'],
  // The settings
  ['.settings-nav-list', 'box'],
  ['.settings-card .pane', 'box'],
  ['.field-row', 'row'],
  ['.player-langs-row', 'row'],
  ['.actions', 'row'],
  ['.theme-grid', 'box'],
  ['.addon-add', 'row'],
  ['.addon-list', 'box'],
  ['.addon-item', 'row'],
  ['.remote-ifaces', 'box'],
  ['.remote-iface', 'row'],
  ['.remote-devices', 'box'],
  ['.remote-device', 'row'],
  ['.account-card', 'row'],
  ['.account-actions', 'row'],
  // Dialogs and menus
  ['.alert-form', 'box'],
  ['.alert-select-list', 'box'],
  ['.alert-actions', 'row'],
  ['[data-pick-options]', 'box'],
  ['.genre-menu', 'box'],
  // The player
  ['.player-top', 'row'],
  ['.player-pickers', 'row'],
  ['.player-controls', 'box'],
  ['.player-row', 'row'],
  ['.player-volume', 'row'],
  ['.player-settings-tabs', 'row'],
  ['.player-settings-body', 'box'],
  ['.player-settings-extra', 'row'],
  ['.player-settings-imdb-wrap', 'row'],
];
const ZONE_SELECTOR = ZONES.map(([sel]) => sel).join(',');

// Zones that give the selection back to what it had selected in them: the
// topbar, the rails, the grid, the parts of a details page.
const REMEMBER = '.topbar, .trending-rail, #grid, .dossier-actions, .dossier-cast, .streams-picker, .streams-filter-slot, .stream-list';

// What a zone selects when the selection comes into it (and it remembers
// nothing): the tab of the section shown, its current choice, the button
// that confirms.
const PREFERRED = [
  ['.topbar', '.tab.is-active'],
  ['.settings-nav-list', '.settings-nav-item.is-active'],
  ['.actions', 'button.primary'],
  ['.alert-actions', '[data-alert-ok]'],
  ['.player-row', '#playerPlayBtn'],
  ['.player-volume', '#playerMuteBtn'],
  ['.player-settings-tabs', '.is-active'],
  ['.player-settings-body', '.player-settings-opt.is-active'],
];

// Zones read from the top unless the selection comes up into them: a
// section of the settings, the fields and the lines of a dialog.
const FROM_TOP = '.settings-card .pane, .alert-form, .alert-select-list';

// Tabs that show their content as soon as the selection reaches them: the
// sections of the settings, the tabs of the player's settings.
const AUTO_OPEN = '.settings-nav-item, .player-settings-tabs [data-pst-tab]';

let current = null;
let lastBaseFocus = null;
// The item selected last in each zone.
let remembered = new WeakMap();
// Whether the D-pad is in use: always on Android TV; with the phone remote,
// until the mouse is used.
let dpad = false;
const inUse = () => IS_TV || dpad;
// Home closes everything at once: nothing gives the selection back.
let homing = false;

function activeScope() {
  // The Android popup editing a text field (android-inputs.js).
  const inputPopup = $('.input-popup');
  if (inputPopup) return inputPopup;
  const qr = $('#qrOverlay');
  if (qr && !qr.hidden) return qr;
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
  return $$(FOCUSABLE, scope).filter(el => isVisible(el) && !el.closest(EXCLUDED));
}

// Geometry of one move, measured once.
let cache = null;

function isZone(el) {
  let zone = cache.zone.get(el);
  if (zone === undefined) {
    zone = el.nodeType === 1 && el.matches(ZONE_SELECTOR);
    cache.zone.set(el, zone);
  }
  return zone;
}

function zoneType(el) {
  const zone = ZONES.find(([sel]) => el.matches(sel));
  return zone ? zone[1] : 'box';
}

// How far the scrollers around `el` have moved it up.
function scrolledBy(el) {
  let dy = cache.shift.get(el);
  if (dy === undefined) {
    const parent = el.parentElement;
    dy = !parent || getComputedStyle(el).position === 'fixed'
      ? 0
      : parent.scrollTop + scrolledBy(parent);
    cache.shift.set(el, dy);
  }
  return dy;
}

// Where `el` is laid out, as if nothing were scrolled vertically: rows keep
// their order however far a page or a list has scrolled, and what scrolled
// up out of view stays below the topbar it went under. Across, it is where
// it shows, so a rail scrolled sideways lines up with what it shows.
function rectOf(el) {
  let r = cache.rect.get(el);
  if (!r) {
    const b = el.getBoundingClientRect();
    const dy = scrolledBy(el);
    r = { left: b.left, right: b.right, top: b.top + dy, bottom: b.bottom + dy };
    cache.rect.set(el, r);
  }
  return r;
}

function unionRect(els) {
  let left = Infinity, top = Infinity, right = -Infinity, bottom = -Infinity;
  for (const el of els) {
    const r = rectOf(el);
    left = Math.min(left, r.left);
    top = Math.min(top, r.top);
    right = Math.max(right, r.right);
    bottom = Math.max(bottom, r.bottom);
  }
  return { left, top, right, bottom };
}

// The innermost zone holding `el` (itself if it is one), or the scope.
function zoneAround(el, scope) {
  for (let p = el; p && p !== scope; p = p.parentElement) {
    if (isZone(p)) return p;
  }
  return scope;
}

// The parts of a zone a move goes between: its own items, and the zones
// inside it taken whole, each where its items are.
function unitsOf(zone, items) {
  const parts = new Map();
  for (const item of items) {
    if (!zone.contains(item)) continue;
    let unit = item;
    if (item !== zone) {
      for (let p = item.parentElement; p && p !== zone; p = p.parentElement) {
        if (isZone(p)) unit = p;
      }
    }
    if (!parts.has(unit)) parts.set(unit, []);
    parts.get(unit).push(item);
  }
  return [...parts].map(([el, its]) => ({ el, r: unionRect(its) }));
}

// Parts side by side, overlapping for at least half the height of the
// smaller one, make a row; the rows top to bottom, each left to right.
function rowsOf(units) {
  const sorted = [...units].sort((a, b) => a.r.top - b.r.top || a.r.left - b.r.left);
  const rows = [];
  for (const u of sorted) {
    const row = rows[rows.length - 1];
    const height = u.r.bottom - u.r.top;
    if (row && Math.min(row.bottom, u.r.bottom) - u.r.top > Math.min(height, row.bottom - row.top) / 2) {
      row.units.push(u);
      row.bottom = Math.max(row.bottom, u.r.bottom);
    } else {
      rows.push({ units: [u], top: u.r.top, bottom: u.r.bottom });
    }
  }
  return rows.map(row => row.units.sort((a, b) => a.r.left - b.r.left || a.r.top - b.r.top));
}

// The part of a row under `x`, or the nearest to it.
function nearestX(row, x) {
  let best = null;
  let bestScore = Infinity;
  for (const u of row) {
    const off = x < u.r.left ? u.r.left - x : x > u.r.right ? x - u.r.right : 0;
    const score = off * 1000 + Math.abs((u.r.left + u.r.right) / 2 - x);
    if (score < bestScore) { bestScore = score; best = u; }
  }
  return best;
}

// The part on one side of a box nearest `y`.
function nearestSide(units, side, y) {
  const edgeOf = u => (side === 'left' ? u.r.left : u.r.right);
  const edge = side === 'left'
    ? Math.min(...units.map(edgeOf))
    : Math.max(...units.map(edgeOf));
  let best = null;
  let bestScore = Infinity;
  for (const u of units) {
    if (Math.abs(edgeOf(u) - edge) > 12) continue;
    const score = y < u.r.top ? u.r.top - y : y > u.r.bottom ? y - u.r.bottom : 0;
    if (score < bestScore) { bestScore = score; best = u; }
  }
  return best;
}

// The next part after `fromEl` going `dir` inside `zone`; null at its edge.
function stepIn(zone, fromEl, dir, ref, items, scope) {
  const units = unitsOf(zone, items);
  const from = units.find(u => u.el === fromEl);
  if (!from) return null;
  const type = zone === scope && !isZone(scope) ? 'box' : zoneType(zone);
  const rows = rowsOf(units);
  if (type === 'row') {
    if (dir === 'up' || dir === 'down') return null;
    const order = rows.flat();
    const i = order.indexOf(from);
    return order[dir === 'right' ? i + 1 : i - 1] || null;
  }
  const ri = rows.findIndex(row => row.includes(from));
  if (dir === 'left' || dir === 'right') {
    const row = rows[ri];
    const i = row.indexOf(from);
    return row[dir === 'right' ? i + 1 : i - 1] || null;
  }
  const next = rows[dir === 'down' ? ri + 1 : ri - 1];
  return next ? nearestX(next, ref.x) : null;
}

// The item the selection lands on coming into `el` (an item, or a zone)
// going `dir`.
function enter(el, dir, ref, items, fromTop = false) {
  // A zone that is an item itself (a download card) is entered on itself.
  if (!isZone(el) || items.includes(el)) return el;
  if (el.matches(REMEMBER)) {
    const mem = remembered.get(el);
    if (mem && items.includes(mem) && el.contains(mem)) return mem;
  }
  for (const [zoneSel, itemSel] of PREFERRED) {
    if (!el.matches(zoneSel)) continue;
    const pick = el.querySelector(itemSel);
    if (pick && items.includes(pick)) return pick;
  }
  const units = unitsOf(el, items);
  if (!units.length) return null;
  const rows = rowsOf(units);
  const top = fromTop || (dir !== 'up' && el.matches(FROM_TOP));
  let unit;
  if (top) unit = rows[0][0];
  else if (dir === 'down') unit = nearestX(rows[0], ref.x);
  else if (dir === 'up') unit = nearestX(rows[rows.length - 1], ref.x);
  else if (zoneType(el) === 'row') {
    const order = rows.flat();
    unit = order[dir === 'right' ? 0 : order.length - 1];
  }
  else unit = nearestSide(units, dir === 'right' ? 'left' : 'right', ref.y);
  return unit ? enter(unit.el, dir, ref, items, top) : null;
}

let within = [];

function markWithin(el) {
  for (const p of within) p.classList.remove(WITHIN_CLASS);
  within = [];
  for (let p = el?.parentElement; p && p !== document.body; p = p.parentElement) {
    p.classList.add(WITHIN_CLASS);
    within.push(p);
  }
}

function clearFocus() {
  if (current) current.classList.remove(FOCUS_CLASS);
  current = null;
  markWithin(null);
}

// How far a scroller has to go to show `a`-`b` within `lo`-`hi`: centred,
// or else just enough, with a margin.
function scrollDelta(a, b, lo, hi, centre) {
  if (centre) return (a + b) / 2 - (lo + hi) / 2;
  const margin = Math.min(48, (hi - lo) / 6);
  if (a < lo + margin) return a - lo - margin;
  if (b > hi - margin) return Math.min(b - hi + margin, a - lo - margin);
  return 0;
}

// A move keeps the selection in the middle of its scrollers along the
// direction of travel (rows centred going up and down, cards centred in a
// rail going sideways), so what comes next is always in view; the other way,
// and without a move, they scroll as little as possible. Only what scrolls
// does: a box that just hides its overflow (the player's frame) stays put.
function reveal(el, dir) {
  const r = el.getBoundingClientRect();
  let { top, bottom, left, right } = r;
  for (let node = el; node.parentElement; node = node.parentElement) {
    if (getComputedStyle(node).position === 'fixed') break;
    const p = node.parentElement;
    if (p === document.body || p === document.documentElement) break;
    const cs = getComputedStyle(p);
    const scrollsY = /auto|scroll/.test(cs.overflowY) && p.scrollHeight > p.clientHeight;
    const scrollsX = /auto|scroll/.test(cs.overflowX) && p.scrollWidth > p.clientWidth;
    if (!scrollsY && !scrollsX) continue;
    const box = p.getBoundingClientRect();
    const boxTop = box.top + p.clientTop;
    const boxLeft = box.left + p.clientLeft;
    let dy = 0;
    let dx = 0;
    if (scrollsY) {
      dy = scrollDelta(top, bottom, boxTop, boxTop + p.clientHeight, dir === 'up' || dir === 'down');
      dy = Math.max(-p.scrollTop, Math.min(p.scrollHeight - p.clientHeight - p.scrollTop, dy));
    }
    if (scrollsX) {
      dx = scrollDelta(left, right, boxLeft, boxLeft + p.clientWidth, dir === 'left' || dir === 'right');
      dx = Math.max(-p.scrollLeft, Math.min(p.scrollWidth - p.clientWidth - p.scrollLeft, dx));
    }
    if (Math.abs(dy) < 1 && Math.abs(dx) < 1) continue;
    p.scrollBy({ top: dy, left: dx, behavior: 'smooth' });
    top -= dy;
    bottom -= dy;
    left -= dx;
    right -= dx;
  }
}

function setFocus(el, dir = null) {
  if (current && current !== el) current.classList.remove(FOCUS_CLASS);
  current = el || null;
  markWithin(current);
  if (!current) return;
  current.classList.add(FOCUS_CLASS);
  const scope = activeScope();
  if (scope === document.body) lastBaseFocus = current;
  for (let p = current.parentElement; p && p !== scope; p = p.parentElement) {
    if (p.matches(REMEMBER)) remembered.set(p, current);
  }
  if (dir && current.matches(AUTO_OPEN) && !current.classList.contains('is-active')) current.click();
  // What has the page's own focus is left behind: a field being typed in,
  // and on a TV a button its WebView focused with the D-pad while the
  // interface was loading, which would stay outlined.
  const focused = document.activeElement;
  if (dir && focused && focused !== current && focused !== document.body
      && (IS_TV || focused.matches('input, textarea, select'))) focused.blur();
  reveal(current, dir);
  // Android TV keeps Back for the page and for a section of the settings
  // while the selection is in them (android-back.js).
  const where = scope === document.body
    ? (current.closest('.topbar') ? 'topbar' : 'page')
    : (current.closest('#settingsModal .pane.is-active') ? 'pane' : 'layer');
  document.dispatchEvent(new CustomEvent('snav:focus', { detail: { where } }));
}

function pickInitial(list) {
  const vh = window.innerHeight || 800;
  const inView = list.filter(el => {
    const r = el.getBoundingClientRect();
    return r.bottom > 0 && r.top < vh;
  });
  const pool = inView.length ? inView : list;
  return pool.find(el => el.matches('.streams-pick-option.is-active, .genre-option.is-active, .settings-nav-item.is-active, #playerProgress, .play-btn, [data-play], .dossier-actions button, [data-rd-play], .empty button, a.card, .trending-card'))
    || pool.find(el => el.matches('.tab.is-active'))
    || pool[0]
    || null;
}

// Where a TV starts: the selection shows at once, on the welcome page's
// button or on the first title. Until the titles have loaded it waits on the
// tab of the section, and moves to the first one as it shows, unless the
// remote moved it meanwhile.
function begin() {
  if (current && isVisible(current)) return;
  // The WebView's own D-pad focus, from keys pressed while loading.
  if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur();
  const start = pickInitial(candidates(activeScope()));
  if (!start) return;
  setFocus(start);
  if (!start.matches('.tab')) return;
  const titles = new MutationObserver(() => {
    if (current !== start) { titles.disconnect(); return; }
    const first = pickInitial(candidates(activeScope()));
    if (first && !first.matches('.tab')) {
      titles.disconnect();
      setFocus(first);
    }
  });
  titles.observe($('#page-scroll') || document.body, { childList: true, subtree: true });
  setTimeout(() => titles.disconnect(), 30000);
}

function move(dir) {
  dpad = true;
  const scope = activeScope();
  if (!scope) return;
  cache = { zone: new Map(), shift: new Map(), rect: new Map() };
  try {
    const items = candidates(scope);
    if (!items.length) return;

    if (!current || !items.includes(current)) {
      let start = null;
      if (scope === document.body && lastBaseFocus && items.includes(lastBaseFocus)) start = lastBaseFocus;
      setFocus(start || pickInitial(items));
      return;
    }

    // A slider takes its own axis (seek, volume).
    const axis = current.getAttribute('data-snav-axis');
    if ((axis === 'x' && (dir === 'left' || dir === 'right')) ||
        (axis === 'y' && (dir === 'up' || dir === 'down'))) {
      current.dispatchEvent(new CustomEvent('snav-adjust', { detail: dir }));
      return;
    }

    const cr = rectOf(current);
    const ref = { x: (cr.left + cr.right) / 2, y: (cr.top + cr.bottom) / 2 };
    // From the innermost zone outwards, until one has something that way.
    let from = current;
    let zone = zoneAround(current, scope);
    for (;;) {
      const target = stepIn(zone, from, dir, ref, items, scope);
      if (target) {
        const el = enter(target.el, dir, ref, items);
        if (el) setFocus(el, dir);
        return;
      }
      if (zone === scope) return;
      from = zone;
      zone = zoneAround(zone.parentElement, scope);
    }
  } finally {
    cache = null;
  }
}

function activate() {
  dpad = true;
  const scope = activeScope();
  if (!scope) return;
  if (scope.id === 'qrOverlay') {
    scope.hidden = true;
    return;
  }
  if (!current || !isVisible(current) || !scope.contains(current)) {
    move('down');
    return;
  }
  // A slider: its own OK (play/pause on the time bar, mute on the volume).
  if (current.hasAttribute('data-snav-axis')) {
    current.dispatchEvent(new CustomEvent('snav-activate'));
    return;
  }
  if (current.matches('input[type="checkbox"], input[type="radio"]')) {
    current.click();
    return;
  }
  if (current.matches('input, textarea, select')) {
    current.focus();
    return;
  }
  // A menu of several choices stays open and redraws its options: the
  // selection stays on the option just ticked.
  const menu = current.closest('[data-pick-menu]');
  const value = current.dataset.value;
  current.click();
  if (menu && value != null && !current.isConnected && !menu.hidden) {
    const option = [...menu.querySelectorAll('[role="option"]')].find(o => o.dataset.value === value);
    if (option) setFocus(option);
  }
}

// Back inside the settings before closing them: from a section to their
// menu. True when it did.
function backInside() {
  const settings = $('#settingsModal');
  if (!settings || settings.hidden || !current) return false;
  if (!settings.querySelector('.pane.is-active')?.contains(current)) return false;
  const item = settings.querySelector('.settings-nav-item.is-active');
  if (!item || !isVisible(item)) return false;
  setFocus(item);
  return true;
}

// Back on the page: the selection goes up to the tab of the section, the
// page back to its top. True when it did.
function backToTabs() {
  const el = current && isVisible(current) ? current : lastBaseFocus;
  if (activeScope() !== document.body || !el?.isConnected || el.closest('.topbar')) return false;
  const tab = $('.tab.is-active');
  if (!tab || !isVisible(tab)) return false;
  remembered = new WeakMap();
  setFocus(tab);
  scrollPageTop();
  return true;
}

// Back with the phone remote: the innermost thing open closes (a menu, the
// QR code, a dialog, which is cancelled when it can be); the settings go
// from a section back to their menu first; on the page the selection goes
// back up to the tabs. False when there was nothing to go back from.
function back() {
  dpad = true;
  const openPickTrigger = $('.streams-pick.is-open [data-pick-trigger]');
  if (openPickTrigger) {
    openPickTrigger.click();
    return true;
  }
  const genreMenu = $('#genreMenu');
  if (genreMenu && !genreMenu.hidden) {
    $('#genreTrigger')?.click();
    return true;
  }
  const qr = $('#qrOverlay');
  if (qr && !qr.hidden) {
    qr.hidden = true;
    return true;
  }
  const alert = $('#alertModal');
  if (alert && !alert.hidden) {
    (alert.querySelector('[data-alert-cancel]') || alert.querySelector('[data-alert-ok]'))?.click();
    return true;
  }
  const settings = $('#settingsModal');
  if (settings && !settings.hidden) {
    if (!backInside()) {
      settings.querySelector('.settings-close')?.click();
      clearFocus();
    }
    return true;
  }
  const details = $('#detailsModal');
  if (details && !details.hidden) {
    details.querySelector('.dossier-close')?.click();
    clearFocus();
    return true;
  }
  return backToTabs();
}

function scrollPageTop() {
  const scroller = $('#page-scroll') || document.scrollingElement;
  try {
    scroller?.scrollTo({ top: 0, behavior: 'smooth' });
  } catch {
    if (scroller) scroller.scrollTop = 0;
  }
}

function home() {
  dpad = true;
  const openPickTrigger = $('.streams-pick.is-open [data-pick-trigger]');
  if (openPickTrigger) openPickTrigger.click();
  const genreMenu = $('#genreMenu');
  if (genreMenu && !genreMenu.hidden) $('#genreTrigger')?.click();
  const qr = $('#qrOverlay');
  if (qr) qr.hidden = true;
  const alert = $('#alertModal');
  if (alert && !alert.hidden) {
    (alert.querySelector('[data-alert-cancel]') || alert.querySelector('[data-alert-ok]'))?.click();
  }
  for (const sel of ['#settingsModal', '#detailsModal']) {
    const m = $(sel);
    if (m && !m.hidden) m.querySelector('.settings-close, .dossier-close')?.click();
  }
  const tab = $('.tab[data-section="movie"]');
  tab?.click();
  scrollPageTop();
  remembered = new WeakMap();
  clearFocus();
  if (tab && isVisible(tab)) setFocus(tab);
  homing = true;
  setTimeout(() => { homing = false; }, 0);
}

document.addEventListener('pointerdown', () => {
  dpad = false;
  if (current) clearFocus();
}, true);

// While the D-pad is in use, what opens starts on its current choice or its
// first action, and what closes gives the selection back to what opened it.
function selectOption(menu, option) {
  const el = !menu.hidden && (menu.querySelector(`${option}.is-active`) || menu.querySelector(option));
  if (el && isVisible(el)) setFocus(el);
}

function trackLayer(layer, { open, fallback } = {}) {
  if (!layer) return;
  let shown = !layer.hidden;
  let before = null;
  new MutationObserver(() => {
    // The details hide under the player and come back when it closes.
    if (layer === detailsModal && player && !player.hidden) return;
    if (shown === !layer.hidden) return;
    shown = !layer.hidden;
    if (shown) {
      before = current && current.isConnected && !layer.contains(current) ? current : null;
      if (open && inUse()) open();
      return;
    }
    const el = before?.isConnected ? before : fallback?.();
    before = null;
    if (!homing && inUse() && el && isVisible(el) && activeScope().contains(el)) setFocus(el);
  }).observe(layer, { attributes: true, attributeFilter: ['hidden'] });
}

// A details page: its first action (trailer, favourite, play), once drawn;
// what it redraws later (the streams, behind the player) leaves the
// selection alone, and so does the menu of one of its pickers.
const detailsModal = $('#detailsModal');
const detailsBody = $('#detailsBody');
const player = $('#playerModal');
if (detailsModal && detailsBody) {
  new MutationObserver(() => {
    if (detailsModal.hidden || (player && !player.hidden) || !inUse()) return;
    if (current && (detailsModal.contains(current) || current.closest('[data-pick-menu], #alertModal'))
        && isVisible(current)) return;
    const first = detailsBody.querySelector('.dossier-actions button, [data-rd-play]');
    if (first && isVisible(first)) setFocus(first);
  }).observe(detailsBody, { childList: true, subtree: true });
}
trackLayer(detailsModal);

// The settings: the section shown, in their menu.
const settingsModal = $('#settingsModal');
trackLayer(settingsModal, {
  open: () => {
    const item = settingsModal.querySelector('.settings-nav-item.is-active');
    if (item) setFocus(item);
  },
});

// A dialog (a confirmation, a form, a phone asking to be the remote): the
// button or the field it focuses once shown (modal.js, on a timer queued
// before this one).
const dialog = $('#alertModal');
trackLayer(dialog, {
  open: () => setTimeout(() => {
    if (dialog.hidden) return;
    const items = candidates(dialog);
    const focused = document.activeElement;
    const el = items.includes(focused) ? focused : pickInitial(items);
    if (el) setFocus(el);
  }, 0),
});

// The player gives the selection back to what started it; to the stream
// just played if the list was redrawn meanwhile (it goes first, as watched).
trackLayer(player, {
  fallback: () => !detailsModal.hidden && detailsBody.querySelector('.stream.is-watched [data-rd-play]'),
});

// The player's settings: the track, subtitles or speed in use.
const playerSettings = $('#playerSettings');
trackLayer(playerSettings, {
  open: () => {
    const items = candidates(playerSettings);
    const el = items.find(e => e.matches('.player-settings-opt.is-active'))
      || items.find(e => e.matches('.player-settings-tabs .is-active'))
      || items[0];
    if (el) setFocus(el);
  },
});

// Pickers open on their current value, once their menu shows.
document.addEventListener('streampicker:open', (e) => {
  const menu = e.detail?.source?.querySelector('[data-pick-menu]');
  if (menu && inUse()) queueMicrotask(() => selectOption(menu, '.streams-pick-option'));
});

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
    if (!genreMenu.hidden) {
      if (inUse()) selectOption(genreMenu, '.genre-option');
      return;
    }
    if (!current || (current.isConnected && !genreMenu.contains(current))) return;
    const trigger = $('#genreTrigger');
    if (trigger && isVisible(trigger)) setFocus(trigger);
  }).observe(genreMenu, { attributes: true, attributeFilter: ['hidden'] });
}

export const spatialNav = {
  move, activate, back, backInside, backToTabs, home, begin, clear: clearFocus, focus: setFocus,
};
