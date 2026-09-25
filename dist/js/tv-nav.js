// Android TV: the remote drives the app. The D-pad moves the selection with
// the navigation of the phone remote (spatial-nav.js, which also places it
// as pages, menus and dialogs open and close) and OK activates it; in the
// player, player.js decides (seek, play/pause, controls). Back is the system
// Back button (android-back.js). In a text field left, right and OK stay with
// the field and its keyboard; up and down leave it.
import { FORM_TAGS } from './dom.js';
import { IS_TV } from './platform.js';
import { spatialNav } from './spatial-nav.js';
import { isPlayerOpen, playerRemoteKey, playerMediaKey } from './player.js';

const DIRS = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', Enter: 'ok' };
// The same by key code, for a WebView that names them "Unidentified" (the
// D-pad centre is 23 on Android).
const DIR_CODES = { 38: 'up', 40: 'down', 37: 'left', 39: 'right', 13: 'ok', 23: 'ok' };

if (IS_TV) {
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

  window.addEventListener('keydown', (e) => {
    if (e.key.startsWith('Media')) {
      if (playerMediaKey(e.key)) {
        e.preventDefault();
        e.stopImmediatePropagation();
      }
      return;
    }
    const dir = DIRS[e.key] || DIR_CODES[e.keyCode];
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
