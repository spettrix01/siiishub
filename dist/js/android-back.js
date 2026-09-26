// Android back button. Tauri's activity calls WebView.goBack() when the
// page has history and closes the app otherwise, so every overlay pushes a
// history entry while it is open: Back then pops it and we close the
// overlay instead of leaving the app. With nothing open, Back exits as usual.
// With the D-pad (Android TV) Back first goes from a section of the settings
// to their menu, and from the page up to its tabs.
//
// Chromium's Back skips the entries a page adds without a key press or a tap
// in between, so entries are only added right after one (or as an overlay
// opens, which one caused), never while handling Back itself.
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

const topEntry = () => history.state && history.state[STATE_KEY];

// A key was just pressed (or the screen touched): an entry added now stays.
const activated = () => !navigator.userActivation || navigator.userActivation.isActive;

if (IS_ANDROID) {
  let suppressPop = 0;
  // TV: the entry of the page, while the selection is in it below the
  // topbar, and the entry of a section of the settings, while the selection
  // is in it (above the settings' own entry).
  let pageEntry = false;
  let paneEntry = false;

  const popEntries = (count) => {
    suppressPop++;
    if (count === 1) history.back();
    else history.go(-count);
  };

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
        // Settings closed from one of their sections: its entry goes too.
        const withPane = id === '#settingsModal' && paneEntry && topEntry() === '#pane';
        if (id === '#settingsModal') paneEntry = false;
        if (withPane) popEntries(2);
        else if (topEntry() === id) popEntries(1);
      }
    }
  };
  const obs = new MutationObserver(sync);
  for (const id of OVERLAYS) {
    const el = $(id);
    if (el) obs.observe(el, { attributes: true, attributeFilter: ['hidden'] });
  }

  // Popups (the dropdowns and the text editors) hold one history entry while
  // open, like the overlays above; switching from one popup to another reuses
  // the entry. On phones the dropdowns open as popups; on a TV every dropdown
  // counts as one, so Back closes it rather than what it is in.
  let openPopup = null; // { source, close }
  const popupOpened = (source, close) => {
    if (!openPopup) history.pushState({ [STATE_KEY]: '#popup' }, '');
    openPopup = { source, close };
  };
  const popupClosed = (source) => {
    if (!openPopup || openPopup.source !== source) return;
    openPopup = null;
    if (topEntry() === '#popup') popEntries(1);
  };
  document.addEventListener('streampicker:open', e => {
    if (!e.detail?.popup && !IS_TV) return;
    const root = e.detail.source;
    popupOpened(root, () => root.querySelector('[data-pick-trigger]')?.click()); // toggles the open menu shut
  });
  document.addEventListener('streampicker:close', e => {
    if (e.detail?.popup || IS_TV) popupClosed(e.detail.source);
  });
  document.addEventListener('siiis:popup-open', e => popupOpened(e.detail.source, e.detail.close));
  document.addEventListener('siiis:popup-close', e => popupClosed(e.detail.source));

  // TV: where the selection goes decides the entries of the page and of the
  // settings' sections. Back then brings the selection from a section back
  // to the settings' menu, and from the page up to its tabs; the next Back
  // closes the settings, or leaves the app.
  if (IS_TV) {
    document.addEventListener('snav:focus', e => {
      const where = e.detail?.where;
      if (openPopup) return;
      if (where === 'pane') {
        if (!paneEntry && topEntry() === '#settingsModal' && activated()) {
          paneEntry = true;
          history.pushState({ [STATE_KEY]: '#pane' }, '');
        }
        return;
      }
      if (paneEntry && topEntry() === '#pane') {
        paneEntry = false;
        popEntries(1);
      }
      if (tracked.size || where === 'layer') return;
      if (where === 'page' && !pageEntry && activated()) {
        pageEntry = true;
        history.pushState({ [STATE_KEY]: '#page' }, '');
      } else if (where === 'topbar' && pageEntry && topEntry() === '#page') {
        pageEntry = false;
        popEntries(1);
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
    // The entry of a section of the settings: back to their menu.
    if (id === '#settingsModal' && paneEntry) {
      paneEntry = false;
      spatialNav.backInside();
      return;
    }
    if (id) {
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
