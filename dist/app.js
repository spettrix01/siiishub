import './js/platform.js';
import { state } from './js/state.js';
import { init as initUserStore, migrateFromLocalStorage } from './js/userstore.js';
import { applyTheme, currentTheme } from './js/theme.js';
import { loadSettings, loadGenres } from './js/api.js';
import { setLang, onLangChange, t } from './js/i18n.js';
import { renderGenres } from './js/topbar.js';
import { loadMore, showEmpty, clearGrid } from './js/grid.js';
import { openSettings } from './js/settings-ui.js';
import { refreshTrending } from './js/trending.js';
import './js/details.js';
import './js/player.js';
import './js/modal.js';
import './js/remote-approval.js';
import './js/android-back.js';
import './js/bottom-nav.js';
import './js/android-genres.js';
import './js/android-inputs.js';
import './js/android-player.js';
import { startDownloadBadgePolling } from './js/library.js';

function syncScrollbarVar() {
  const ps = document.getElementById('page-scroll');
  if (!ps) return;
  const sbw = Math.max(0, ps.offsetWidth - ps.clientWidth);
  document.documentElement.style.setProperty('--scrollbar-w', `${sbw}px`);
}
syncScrollbarVar();
window.addEventListener('resize', syncScrollbarVar);
new ResizeObserver(syncScrollbarVar).observe(document.getElementById('page-scroll'));

await initUserStore();
await migrateFromLocalStorage();

applyTheme(currentTheme());

await loadSettings();
setLang(state.settings.language, { rerender: false });
if (state.settings.tmdbKey) {
  await loadGenres();
}
renderGenres();
if (!state.settings.tmdbKey) {
  showEmpty(
    t('welcome.title'),
    t('welcome.body'),
    t('welcome.openSettings'),
    () => openSettings('tmdb'),
  );
} else {
  refreshTrending();
  loadMore();
}
startDownloadBadgePolling();

onLangChange(async () => {
  if (!state.settings.tmdbKey || state.section === 'library') return;
  try { await loadGenres(); } catch {}
  renderGenres();
  refreshTrending();
  clearGrid();
  loadMore();
});
