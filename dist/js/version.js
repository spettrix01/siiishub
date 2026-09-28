// The version of SIIISHUB in the bottom right corner, and whether a newer
// one is out: the latest release on GitHub, which the backend asks
// (update.rs) at start and then every six hours. On a phone, whose bottom
// bar takes the corner, a popup at start tells of a newer version instead.
import { appVersion, updateCheck } from './api.js';
import { escapeHTML, openExternal } from './dom.js';
import { t, onLangChange } from './i18n.js';
import { IS_PHONE, IS_TV } from './platform.js';
import { showConfirm } from './modal.js';

const EVERY = 6 * 3600 * 1000;
// The head of the popup: an arrow into a tray.
const DOWNLOAD_ICON = '<svg viewBox="0 0 24 24" width="22" height="22"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 4v11m-4.5-4.5L12 15l4.5-4.5M5 19h14"/></svg>';

const label = document.getElementById('appVersion');
let version = '';
let update = null;

function render() {
  if (!label || !version) return;
  let state = '';
  if (update?.newer) {
    const text = escapeHTML(t('update.available', { version: update.latest }));
    // The remote's selection does not reach the corner of a TV: the news
    // alone there.
    state = IS_TV
      ? `<span class="app-version-state is-new">${text}</span>`
      : `<a class="app-version-state is-new" href="${escapeHTML(update.url)}" target="_blank" rel="noreferrer noopener" title="${escapeHTML(t('update.open'))}">${text}</a>`;
  } else if (update) {
    state = `<span class="app-version-state is-ok">${escapeHTML(t('update.upToDate'))}</span>`;
  }
  label.innerHTML = `<span class="app-version-number">v${escapeHTML(version)}</span>${state}`;
}

async function check() {
  try {
    update = await updateCheck();
  } catch {
    // Offline, or GitHub not answering: what was known stays.
  }
  render();
}

async function offerOnPhone() {
  try {
    update = await updateCheck();
  } catch {
    return;
  }
  // Another popup open: not over it.
  if (!update?.newer || !document.getElementById('alertModal')?.hidden) return;
  const download = await showConfirm(t('update.popupBody', { latest: update.latest, current: version }), {
    title: t('update.popupTitle'),
    variant: 'accent',
    icon: DOWNLOAD_ICON,
    okLabel: t('update.download'),
    cancelLabel: t('update.later'),
  });
  if (download) openExternal(update.url);
}

export async function startVersion() {
  version = await appVersion();
  if (!version) return;
  if (IS_PHONE) {
    offerOnPhone();
    return;
  }
  render();
  check();
  setInterval(check, EVERY);
  onLangChange(render);
}
