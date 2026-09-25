// What the other devices of an account change reaches the interface here,
// without reloading it: in the apps signed in to it (account.rs), and in the
// browser version signed in with it, whose server tells its pages
// (server/sync_api.rs).
import { state } from './state.js';
import { loadSettings, onSyncChanged } from './api.js';
import { reload as reloadUserStore } from './userstore.js';
import { applyTheme, currentTheme } from './theme.js';
import { setLang } from './i18n.js';

/** Shows what a sync changed (`{ userdata, settings }`, from account.rs). */
export async function refreshAfterSync(applied) {
  if (!applied || (!applied.userdata && !applied.settings)) return;
  if (applied.userdata) {
    await reloadUserStore();
    applyTheme(currentTheme());
    // Progress bars, Continue watching, Favorites.
    window.dispatchEvent(new CustomEvent('siiis:resume', { detail: null }));
    window.dispatchEvent(new CustomEvent('siiis:favorite', { detail: null }));
  }
  if (applied.settings) {
    const hadKey = !!state.settings.tmdbKey;
    const lang = state.settings.language;
    await loadSettings();
    // The device's first TMDB key: the home starts over with it.
    if (!hadKey && state.settings.tmdbKey) {
      location.reload();
      return;
    }
    if (state.settings.language !== lang) setLang(state.settings.language);
  }
}

onSyncChanged(refreshAfterSync).catch(() => {});
