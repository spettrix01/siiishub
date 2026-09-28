// The version of SIIISHUB in the bottom right corner, and whether a newer
// one is out: the latest release on GitHub, which the backend asks
// (update.rs) at start and then every six hours. A green dot next to the
// version when it is the latest; a red one when it is not, with a banner
// above it that opens the new version's page. On a phone, whose bottom bar
// takes the corner, a popup at start tells of a newer version instead.
import { appVersion, updateCheck } from './api.js';
import { escapeHTML, openExternal } from './dom.js';
import { t, onLangChange } from './i18n.js';
import { IS_PHONE, IS_TV } from './platform.js';
import { showConfirm } from './modal.js';

const EVERY = 6 * 3600 * 1000;
// The head of the popup: an arrow into a tray.
const DOWNLOAD_ICON = '<svg viewBox="0 0 24 24" width="22" height="22"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 4v11m-4.5-4.5L12 15l4.5-4.5M5 19h14"/></svg>';
// The banner's link leaves the app.
const OUT_ICON = '<svg viewBox="0 0 24 24" width="11" height="11" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" d="M8 16 16 8m-7 0h7v7"/></svg>';

const label = document.getElementById('appVersion');
let version = '';
let update = null;

function render() {
  if (!label || !version) return;
  let banner = '';
  let dot = '';
  if (update) {
    const state = update.newer ? 'is-new' : 'is-ok';
    const said = update.newer ? t('update.available') : t('update.upToDate');
    dot = `<span class="app-version-dot ${state}" role="img" aria-label="${escapeHTML(said)}"></span>`;
  }
  if (update?.newer) {
    const text = escapeHTML(t('update.available'));
    // The remote's selection does not reach the corner of a TV: the news
    // alone there.
    banner = IS_TV
      ? `<span class="app-version-banner">${text}</span>`
      : `<a class="app-version-banner" href="${escapeHTML(update.url)}" target="_blank" rel="noreferrer noopener" title="${escapeHTML(`SIIISHUB ${update.latest} · ${t('update.open')}`)}">${text}${OUT_ICON}</a>`;
  }
  label.innerHTML = `${banner}<span class="app-version-line"><span class="app-version-number">v${escapeHTML(version)}</span>${dot}</span>`;
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
