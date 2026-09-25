// Android back button. Tauri's activity calls WebView.goBack() when the
// page has history and closes the app otherwise, so every overlay pushes a
// history entry while it is open: Back then pops it and we close the
// overlay instead of leaving the app. With nothing open, Back exits as usual.
// With the D-pad (Android TV) Back first goes from a section of the settings
// to their menu, and from the page up to its tabs.
import { $ } from './dom.js';
import { IS_ANDROID, IS_TV } from './platform.js';
import { closePlayer } from './player.js';
import { closeModal } from './modal.js';
import { closeRemoteOverlay } from './remote-client.js';
import { spatialNav } from './spatial-nav.js';

const OVERLAYS = ['#genreMenu', '#alertModal', '#playerSettings', '#playerModal', '#remoteOverlay', '#settingsModal', '#detailsModal'];
const STATE_KEY = 'siiisOverlay';

function closeOverlay(id) {
  const el = $(id);
  if (!el || el.hidden) return;
  if (id === '#playerModal') closePlayer();
  else if (id === '#remoteOverlay') closeRemoteOverlay(); // also releases the camera
  else if (id === '#genreMenu') $('#genreTrigger')?.click(); // toggles the open menu shut
  else if (id === '#playerSettings') $('#playerSettingsBtn')?.click(); // toggles the panel shut
  else if (id === '#alertModal') {
    // Let the dialog settle its promise through its own buttons.
    const cancel = el.querySelector('[data-alert-cancel]') || el.querySelector('[data-alert-ok]');
    cancel?.click();
  } else closeModal(id);
}

function topOpenOverlay() {
  return OVERLAYS.find(id => $(id) && !$(id).hidden) || null;
}

if (IS_ANDROID) {
  let suppressPop = 0;

  // Keep exactly one pushed entry per open overlay.
  const tracked = new Set();
  const sync = () => {
    for (const id of OVERLAYS) {
      const el = $(id);
      if (!el) continue;
      if (!el.hidden && !tracked.has(id)) {
        tracked.add(id);
        history.pushState({ [STATE_KEY]: id }, '');
      } else if (el.hidden && tracked.has(id)) {
        // The details hide under the player and come back when it closes:
        // they keep their entry.
        if (id === '#detailsModal' && !$('#playerModal').hidden) continue;
        tracked.delete(id);
        if (history.state && history.state[STATE_KEY] === id) {
          suppressPop++;
          history.back();
        }
      }
    }
  };
  const obs = new MutationObserver(sync);
  for (const id of OVERLAYS) {
    const el = $(id);
    if (el) obs.observe(el, { attributes: true, attributeFilter: ['hidden'] });
  }

  // Popups (the settings dropdowns and text editors) hold one history entry
  // while open, like the overlays above; switching from one popup to another
  // reuses the entry.
  let openPopup = null; // { source, close }
  const popupOpened = (source, close) => {
    if (!openPopup) history.pushState({ [STATE_KEY]: '#popup' }, '');
    openPopup = { source, close };
  };
  const popupClosed = (source) => {
    if (!openPopup || openPopup.source !== source) return;
    openPopup = null;
    if (history.state && history.state[STATE_KEY] === '#popup') {
      suppressPop++;
      history.back();
    }
  };
  document.addEventListener('streampicker:open', e => {
    if (!e.detail?.popup) return;
    const root = e.detail.source;
    popupOpened(root, () => root.querySelector('[data-pick-trigger]')?.click()); // toggles the open menu shut
  });
  document.addEventListener('streampicker:close', e => {
    if (e.detail?.popup) popupClosed(e.detail.source);
  });
  document.addEventListener('siiis:popup-open', e => popupOpened(e.detail.source, e.detail.close));
  document.addEventListener('siiis:popup-close', e => popupClosed(e.detail.source));

  // TV: the page holds one entry while the selection is in it, below the
  // topbar; Back then brings the selection up to the tabs, and the next Back
  // leaves the app.
  let pageEntry = false;
  if (IS_TV) {
    document.addEventListener('snav:focus', e => {
      const where = e.detail?.where;
      if (tracked.size || openPopup || where === 'layer') return;
      if (where === 'page' && !pageEntry) {
        pageEntry = true;
        history.pushState({ [STATE_KEY]: '#page' }, '');
      } else if (where === 'topbar' && pageEntry && history.state && history.state[STATE_KEY] === '#page') {
        pageEntry = false;
        suppressPop++;
        history.back();
      }
    });
  }

  window.addEventListener('popstate', () => {
    if (suppressPop > 0) { suppressPop--; return; }
    if (openPopup) {
      const popup = openPopup;
      openPopup = null;
      popup.close();
      return;
    }
    const id = topOpenOverlay();
    if (id) {
      // From a section of the settings, their menu first: the settings keep
      // their entry.
      if (id === '#settingsModal' && spatialNav.backInside()) {
        history.pushState({ [STATE_KEY]: id }, '');
        return;
      }
      tracked.delete(id);
      closeOverlay(id);
      return;
    }
    if (pageEntry) {
      pageEntry = false;
      spatialNav.backToTabs();
    }
  });
}
