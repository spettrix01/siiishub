// Android phone bottom navigation: Film / Serie / Libreria / Impostazioni
// (Android TV keeps the top tabs, reached with the remote's D-pad).
// It drives the existing top tabs (hidden by CSS on Android) so section
// switching, genres, trending and the library keep their single code path;
// the active state and the "new downloads" badge are mirrored from them.
// Settings open as a page laid out between the topbar and this bar (the
// modal markup is restyled by CSS), so the bar stays usable while they are
// open and the Back button returns to the previous section.
import { $ } from './dom.js';
import { IS_PHONE } from './platform.js';
import { openSettings } from './settings-ui.js';
import { closeModal } from './modal.js';

const nav = $('#bottomNav');

if (nav && IS_PHONE) {
  nav.hidden = false;
  document.documentElement.classList.add('has-bottom-nav');

  const items = [...nav.querySelectorAll('.bottom-nav-item[data-bn-section]')];
  const settingsItem = nav.querySelector('.bottom-nav-item[data-bn-settings]');
  const settingsModal = $('#settingsModal');
  const topbar = $('.topbar');
  const tabFor = section => $(`.tabs .tab[data-section="${section}"]`);
  const badge = $('#bottomNavBadge');
  const badgeSource = $('#libraryBadge');
  const settingsOpen = () => !!settingsModal && !settingsModal.hidden;

  const setActive = (btn, on) => {
    btn.classList.toggle('is-active', on);
    if (on) btn.setAttribute('aria-current', 'page');
    else btn.removeAttribute('aria-current');
  };

  const sync = () => {
    const inSettings = settingsOpen();
    const justOpened = inSettings && !document.documentElement.classList.contains('settings-open');
    document.documentElement.classList.toggle('settings-open', inSettings);
    for (const btn of items) {
      setActive(btn, !inSettings && !!tabFor(btn.dataset.bnSection)?.classList.contains('is-active'));
    }
    if (settingsItem) setActive(settingsItem, inSettings);
    if (badge && badgeSource) {
      badge.hidden = badgeSource.hidden;
      badge.textContent = badgeSource.textContent;
    }
    if (justOpened) {
      // The chip row keeps its scroll position between openings: bring the
      // active section back into view.
      requestAnimationFrame(() => {
        settingsModal.querySelector('.settings-nav-item.is-active')?.scrollIntoView({ block: 'nearest', inline: 'center' });
      });
    }
  };

  nav.addEventListener('click', e => {
    const btn = e.target.closest('.bottom-nav-item');
    if (!btn) return;
    if (btn.hasAttribute('data-bn-settings')) {
      if (!settingsOpen()) openSettings();
      return;
    }
    if (settingsOpen()) closeModal('#settingsModal');
    const tab = tabFor(btn.dataset.bnSection);
    if (!tab) return;
    tab.click();
    $('#page-scroll')?.scrollTo({ top: 0 });
  });

  // The settings page starts right below the topbar: publish its height.
  if (topbar) {
    const publishTopbar = () => document.documentElement.style.setProperty('--topbar-h', `${topbar.offsetHeight}px`);
    publishTopbar();
    new ResizeObserver(publishTopbar).observe(topbar);
  }

  // The plugin shortens the page while the keyboard is up, so focused fields
  // stay above it: hide this bar meanwhile instead of letting it ride on the
  // keyboard. The full height is kept per width, so a rotation starts over.
  const fullHeight = new Map();
  const syncKeyboard = () => {
    const w = window.innerWidth;
    const h = window.innerHeight;
    const full = Math.max(fullHeight.get(w) || 0, h);
    fullHeight.set(w, full);
    document.documentElement.classList.toggle('keyboard-open', h < full * 0.75);
  };
  window.addEventListener('resize', syncKeyboard);
  syncKeyboard();

  // Keep the tapped settings section chip in view in the scrollable row.
  $('#settingsNavList')?.addEventListener('click', e => {
    e.target.closest('.settings-nav-item')?.scrollIntoView({ block: 'nearest', inline: 'center', behavior: 'smooth' });
  });

  const observer = new MutationObserver(sync);
  for (const tab of document.querySelectorAll('.tabs .tab')) {
    observer.observe(tab, { attributes: true, attributeFilter: ['class'] });
  }
  if (settingsModal) {
    observer.observe(settingsModal, { attributes: true, attributeFilter: ['hidden'] });
  }
  if (badgeSource) {
    observer.observe(badgeSource, { attributes: true, attributeFilter: ['hidden'], childList: true, characterData: true, subtree: true });
  }
  sync();
}
