// Android TV: the remote drives the app. The D-pad moves the selection with
// the spatial navigation of the phone remote and OK activates it; in the
// player, player.js decides (seek, play/pause, controls). Back is the system
// Back button (android-back.js). In a text field left, right and OK stay with
// the field and its keyboard; up and down leave it.
import { $, FORM_TAGS } from './dom.js';
import { IS_TV } from './platform.js';
import { spatialNav } from './spatial-nav.js';
import { isPlayerOpen, playerRemoteKey, playerMediaKey } from './player.js';

const DIRS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', Enter: 'ok' };

if (IS_TV) {
  // A details page opens with its first action selected (trailer, favourite);
  // what the page redraws behind the player leaves the selection alone, and
  // so does the menu of one of its pickers, which opens outside the page.
  const details = $('#detailsModal');
  const body = $('#detailsBody');
  new MutationObserver(() => {
    if (details.hidden || isPlayerOpen()) return;
    const sel = document.querySelector('.snav-focus');
    if (sel && (details.contains(sel) || sel.closest('[data-pick-menu], #alertModal'))) return;
    const first = body.querySelector('.dossier-actions button') || body.querySelector('[data-rd-play]');
    if (first) spatialNav.focus(first);
  }).observe(body, { childList: true, subtree: true });

  // Pickers and the genre menu open on their current value (a picker's menu
  // shows once its open event is handled).
  const selectOption = (menu, option) => {
    const el = !menu.hidden && (menu.querySelector(`${option}.is-active`) || menu.querySelector(option));
    if (el) spatialNav.focus(el);
  };
  document.addEventListener('streampicker:open', (e) => {
    const menu = e.detail?.source?.querySelector('[data-pick-menu]');
    if (menu) queueMicrotask(() => selectOption(menu, '.streams-pick-option'));
  });
  const genreMenu = $('#genreMenu');
  new MutationObserver(() => selectOption(genreMenu, '.genre-option'))
    .observe(genreMenu, { attributes: true, attributeFilter: ['hidden'] });

  // The popup editing a text field (android-inputs.js) gives the selection
  // back to the field when it closes.
  let beforePopup = null;
  document.addEventListener('siiis:popup-open', (e) => {
    const sel = document.querySelector('.snav-focus');
    beforePopup = sel && !e.detail?.source?.contains(sel) ? sel : null;
  });
  document.addEventListener('siiis:popup-close', () => {
    const el = beforePopup;
    beforePopup = null;
    if (el?.isConnected) spatialNav.focus(el);
  });

  // Settings open on their current section in the menu.
  const settings = $('#settingsModal');
  new MutationObserver(() => {
    const item = !settings.hidden && settings.querySelector('.settings-nav-item.is-active');
    if (item) spatialNav.focus(item);
  }).observe(settings, { attributes: true, attributeFilter: ['hidden'] });

  // A dialog (a confirmation, a phone asking to be the remote) opens on the
  // button it focuses, its confirmation, whatever was picked underneath; the
  // selection goes back there when it closes.
  const dialog = $('#alertModal');
  let beforeDialog = null;
  new MutationObserver(() => {
    if (dialog.hidden) {
      const el = beforeDialog;
      beforeDialog = null;
      if (el?.isConnected) spatialNav.focus(el);
      return;
    }
    const sel = document.querySelector('.snav-focus');
    if (sel && !dialog.contains(sel)) beforeDialog = sel;
    // The dialog focuses its button once it is shown.
    requestAnimationFrame(() => {
      if (dialog.hidden) return;
      const focused = dialog.contains(document.activeElement) ? document.activeElement : null;
      const button = focused?.matches('button') ? focused : dialog.querySelector('button');
      if (button) spatialNav.focus(button);
    });
  }).observe(dialog, { attributes: true, attributeFilter: ['hidden'] });

  // Closing the player gives the selection back to what started it; a stream
  // list redrawn meanwhile (the stream just played goes first, as watched)
  // gives it to that stream.
  const player = $('#playerModal');
  let beforePlayer = null;
  new MutationObserver(() => {
    if (isPlayerOpen()) {
      const sel = document.querySelector('.snav-focus');
      if (sel && !player.contains(sel)) beforePlayer = sel;
      return;
    }
    const el = beforePlayer?.isConnected ? beforePlayer
      : !details.hidden && body.querySelector('.stream.is-watched [data-rd-play]');
    beforePlayer = null;
    if (el) spatialNav.focus(el);
  }).observe(player, { attributes: true, attributeFilter: ['hidden'] });

  window.addEventListener('keydown', (e) => {
    if (e.key.startsWith('Media')) {
      if (playerMediaKey(e.key)) {
        e.preventDefault();
        e.stopImmediatePropagation();
      }
      return;
    }
    const dir = DIRS[e.key];
    if (!dir) return;
    const field = FORM_TAGS.includes(document.activeElement?.tagName) ? document.activeElement : null;
    if (field) {
      if (dir === 'left' || dir === 'right' || dir === 'ok') return;
      // The move starts from the field, even one focused without the D-pad
      // (the popup editing a text field).
      field.blur();
      if (!field.classList.contains('snav-focus')) spatialNav.focus(field);
    } else if (dir === 'ok' && !isPlayerOpen() && !document.querySelector('.snav-focus')
        && document.activeElement?.matches('button, a[href]')) {
      // Nothing picked with the D-pad yet: OK keeps meaning the focused
      // button, like the confirm button of a dialog.
      return;
    }
    e.preventDefault();
    e.stopImmediatePropagation();
    if (isPlayerOpen()) playerRemoteKey(dir);
    else if (dir === 'ok') spatialNav.activate();
    else spatialNav.move(dir);
  }, true);
}
