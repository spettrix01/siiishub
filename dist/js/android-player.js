// Android player tweaks.
// - The phone turns to landscape (immersive) as soon as the player overlay
//   opens, while the stream is still being prepared, instead of when the
//   video surface appears at the end of the preparation.
// - The settings panel is a popup centred on the screen over a dimmed
//   backdrop, like the app's other popups. It moves out of the controls bar:
//   the bar's auto-hide transform would otherwise anchor a fixed element to
//   the bar instead of the screen.
// - Pinch on the video, like YouTube: spreading two fingers zooms the picture
//   to fill the screen (mpv crops it with panscan, no black bars), pinching
//   them together fits it back. A short hint names the mode; closing the
//   player goes back to fit.
// - A tap anywhere on the player that is not a control shows the overlay
//   when it is hidden and hides it when it is shown, like YouTube.
// (Holding a finger on the video does not play at 2x on the phone: player.js
// skips that on Android.)
import { $ } from './dom.js';
import { IS_ANDROID } from './platform.js';
import { mpvCommand } from './api.js';
import { t } from './i18n.js';

const modal = $('#playerModal');
const frame = $('#playerFrame');
const settings = $('#playerSettings');
const video = $('#playerVideo');

let zoomFill = false;
let zoomHint = null;
let zoomHintTimer = 0;
let lastPinchAt = -Infinity;

function setZoomFill(on, { hint = true } = {}) {
  zoomFill = on;
  mpvCommand(['set', 'panscan', on ? '1.0' : '0.0']).catch(e => console.warn('panscan failed', e));
  if (!hint || !frame) return;
  if (!zoomHint) {
    // Same look as the 2x speed hint of the desktop player.
    zoomHint = document.createElement('div');
    zoomHint.className = 'player-speed-hint player-zoom-hint';
    zoomHint.hidden = true;
    frame.appendChild(zoomHint);
  }
  zoomHint.textContent = t(on ? 'player.zoomFill' : 'player.zoomFit');
  zoomHint.hidden = false;
  requestAnimationFrame(() => zoomHint.classList.add('is-visible'));
  clearTimeout(zoomHintTimer);
  zoomHintTimer = setTimeout(() => {
    zoomHint.classList.remove('is-visible');
    setTimeout(() => {
      if (!zoomHint.classList.contains('is-visible')) zoomHint.hidden = true;
    }, 140);
  }, 900);
}

if (IS_ANDROID && modal) {
  let landscape = false;
  const syncMode = () => {
    const open = !modal.hidden;
    if (open === landscape) return;
    landscape = open;
    const invoke = window.__TAURI__?.core?.invoke;
    if (invoke) invoke('player_mode', { on: open }).catch(e => console.warn('player_mode failed', e));
    if (!open && zoomFill) setZoomFill(false, { hint: false });
  };
  new MutationObserver(syncMode).observe(modal, { attributes: true, attributeFilter: ['hidden'] });
  syncMode();
}

if (IS_ANDROID && frame && settings) {
  const backdrop = document.createElement('div');
  backdrop.className = 'player-settings-backdrop';
  backdrop.hidden = true;
  frame.append(backdrop, settings);
  // A tap on the backdrop closes the panel through player.js' document click
  // handler; the press itself stays away from the player underneath.
  backdrop.addEventListener('pointerdown', e => e.stopPropagation());
  const syncBackdrop = () => { backdrop.hidden = settings.hidden; };
  new MutationObserver(syncBackdrop).observe(settings, { attributes: true, attributeFilter: ['hidden'] });
  syncBackdrop();
}

if (IS_ANDROID && video) {
  let startSpread = 0;
  let done = false;
  const spread = touches => Math.hypot(
    touches[0].clientX - touches[1].clientX,
    touches[0].clientY - touches[1].clientY,
  );
  video.addEventListener('touchstart', e => {
    if (e.touches.length !== 2) return;
    startSpread = spread(e.touches);
    done = false;
    lastPinchAt = e.timeStamp;
  }, { passive: true });
  video.addEventListener('touchmove', e => {
    if (e.touches.length !== 2 || !startSpread) return;
    lastPinchAt = e.timeStamp;
    if (done) return;
    const ratio = spread(e.touches) / startSpread;
    if (ratio > 1.15 && !zoomFill) { setZoomFill(true); done = true; }
    else if (ratio < 0.87 && zoomFill) { setZoomFill(false); done = true; }
  }, { passive: true });
  const release = e => {
    if (e.touches.length < 2 && startSpread) { startSpread = 0; lastPinchAt = e.timeStamp; }
  };
  video.addEventListener('touchend', release, { passive: true });
  video.addEventListener('touchcancel', release, { passive: true });
}

if (IS_ANDROID && frame) {
  // player.js shows the overlay on every press (wakePlayer), so whether it
  // was hidden is read before that, in the capture phase. Presses on
  // controls, long presses and pinches are not taps.
  const NOT_A_TAP = 'button, a, input, [role="slider"], .player-progress, .player-pickers, '
    + '.player-settings, .player-settings-backdrop, .player-side-pane, .player-log-pane, .player-status';
  const LONG_PRESS_MS = 600;
  let hiddenAtPress = false;
  let pressAt = 0;
  frame.addEventListener('pointerdown', e => {
    hiddenAtPress = frame.classList.contains('is-idle');
    pressAt = e.timeStamp;
  }, true);
  frame.addEventListener('click', e => {
    if (e.target.closest(NOT_A_TAP)) return;
    if (e.timeStamp - pressAt > LONG_PRESS_MS) return;
    if (e.timeStamp - lastPinchAt < 500) return;
    if (frame.classList.contains('is-logopen') || frame.classList.contains('is-split')) return;
    // Hidden before the press: the press has already shown it. Shown: hide.
    if (!hiddenAtPress) frame.classList.add('is-idle');
  });
}
