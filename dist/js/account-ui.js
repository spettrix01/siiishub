// Settings → Account. In the apps: the SIIISHUB server the app syncs its
// library, favorites, addons, keys and preferences with (account.rs),
// signed in with an account made in the server version. In the browser
// version: who is signed in, their password and, for the administrator,
// the server's accounts (server/accounts.rs).
import { $, escapeHTML } from './dom.js';
import { t, intlLocale } from './i18n.js';
import { IS_WEB, IS_ANDROID, IS_TV } from './platform.js';
import { showConfirm } from './modal.js';
import {
  syncStatus, syncSignIn, syncSignOut,
  accountStatus, accountsList, accountCreate, accountDelete, accountPassword,
} from './api.js';
import { refreshAfterSync } from './sync-client.js';

// Kinds as the other sections' hints: success, error, loading.
function hint(text = '', kind = '') {
  const el = $('#accountHint');
  if (!el) return;
  el.className = kind ? `hint ${kind}` : 'hint';
  el.textContent = text;
}

// The backend's error codes (account.rs, accounts.rs) in words.
function errorText(e) {
  const code = typeof e === 'string' ? e : (e?.message || String(e));
  const key = `settings.account.error.${code}`;
  const text = t(key);
  return text === key ? `${t('common.error')}: ${code}` : text;
}

function syncTime(secs) {
  return new Date(secs * 1000).toLocaleString(intlLocale(), { dateStyle: 'short', timeStyle: 'short' });
}

// How the server lists this device.
function deviceName() {
  if (IS_TV) return 'Android TV';
  if (IS_ANDROID) return 'Android';
  return /Windows/i.test(navigator.userAgent) ? 'Windows' : 'Linux';
}

function show(parts) {
  for (const [id, visible] of Object.entries(parts)) {
    const el = $(id);
    if (el) el.hidden = !visible;
  }
}

// The card on top: an initial, a name, a line under it.
function card(name, sub = '') {
  $('#accountAvatar').textContent = (name || '?').trim().charAt(0).toUpperCase();
  $('#accountName').textContent = name;
  $('#accountSub').textContent = sub;
}

// ---------- Apps ----------

async function renderApp() {
  const status = await syncStatus().catch(() => null);
  const signedIn = !!status?.signed_in;
  $('#accountDesc').textContent = t('settings.account.descApp');
  show({
    '#accountSignInForm': !signedIn,
    '#accountCard': signedIn,
    '#accountPasswordBox': false,
    '#accountAdminBox': false,
  });
  if (signedIn) {
    const when = status.last_sync
      ? t('settings.account.lastSync', { time: syncTime(status.last_sync) })
      : t('settings.account.never');
    card(status.username, `${status.server.replace(/^https?:\/\//, '')} · ${when}`);
  }
  if (status?.error) hint(errorText(status.error), 'error');
}

function wireApp() {
  $('#accountSignInForm').addEventListener('submit', async (e) => {
    e.preventDefault();
    const server = $('#accountServerInput').value.trim();
    const user = $('#accountUserInput').value.trim();
    const password = $('#accountPasswordInput').value;
    if (!server || !user || !password) {
      hint(t('settings.account.fillIn'), 'error');
      return;
    }
    const btn = $('#accountSignInBtn');
    btn.disabled = true;
    hint(t('settings.account.syncing'), 'loading');
    try {
      const applied = await syncSignIn(server, user, password, deviceName());
      $('#accountPasswordInput').value = '';
      await refreshAfterSync(applied);
      await renderApp();
      hint(t('settings.account.synced'), 'success');
    } catch (err) {
      hint(errorText(err), 'error');
    } finally {
      btn.disabled = false;
    }
  });
  $('#accountSignOutBtn').addEventListener('click', async () => {
    await syncSignOut().catch(() => {});
    hint('');
    await renderApp();
  });
}

// ---------- Browser version ----------

let selfId = null;

async function renderWeb() {
  const { account } = await accountStatus().catch(() => ({ account: null }));
  selfId = account?.id || null;
  show({
    '#accountSignInForm': false,
    '#accountCard': true,
    '#accountPasswordBox': !!account,
    '#accountAdminBox': !!account?.admin,
  });
  if (!account) {
    $('#accountDesc').textContent = t('settings.account.descGuest');
    card(t('settings.account.guest'));
    return;
  }
  $('#accountDesc').textContent = t('settings.account.descWeb');
  card(account.username, account.admin ? t('settings.account.admin') : '');
  if (account.admin) await renderAccounts();
}

async function renderAccounts() {
  const list = await accountsList().catch(() => []);
  $('#accountList').innerHTML = list.map(a => `
    <li class="remote-device">
      <span class="remote-device-name">${escapeHTML(a.username)}</span>
      ${a.admin ? `<span class="remote-iface-badge">${escapeHTML(t('settings.account.admin'))}</span>` : ''}
      <span class="account-row-meta">${escapeHTML(t('settings.account.devices', { n: a.devices }))}</span>
      ${a.id === selfId ? '' : `<button type="button" class="remote-device-btn is-forget" data-account-delete="${escapeHTML(a.id)}" data-name="${escapeHTML(a.username)}">${escapeHTML(t('settings.account.delete'))}</button>`}
    </li>`).join('');
}

function wireWeb() {
  $('#accountSignOutBtn').addEventListener('click', async () => {
    await fetch('/api/logout', { method: 'POST', credentials: 'same-origin' }).catch(() => {});
    location.replace('/login');
  });
  $('#accountPasswordForm').addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      await accountPassword($('#accountCurrentPassword').value, $('#accountNewPassword').value);
      $('#accountCurrentPassword').value = '';
      $('#accountNewPassword').value = '';
      $('#accountPasswordBox').open = false;
      hint(t('settings.account.passwordChanged'), 'success');
    } catch (err) {
      hint(errorText(err), 'error');
    }
  });
  $('#accountCreateForm').addEventListener('submit', async (e) => {
    e.preventDefault();
    const user = $('#accountNewUser').value.trim();
    try {
      await accountCreate(user, $('#accountNewUserPassword').value);
      $('#accountNewUser').value = '';
      $('#accountNewUserPassword').value = '';
      $('#accountCreateBox').open = false;
      hint(t('settings.account.created', { user }), 'success');
      await renderAccounts();
    } catch (err) {
      hint(errorText(err), 'error');
    }
  });
  $('#accountList').addEventListener('click', async (e) => {
    const btn = e.target.closest('[data-account-delete]');
    if (!btn) return;
    const ok = await showConfirm(t('settings.account.deleteConfirm', { user: btn.dataset.name }), {
      title: t('settings.account.delete'),
      variant: 'warn',
      okLabel: t('settings.account.delete'),
    });
    if (!ok) return;
    try {
      await accountDelete(btn.dataset.accountDelete);
      await renderAccounts();
      hint('');
    } catch (err) {
      hint(errorText(err), 'error');
    }
  });
}

/** Fills the Account section each time it is shown. */
export async function setupAccountSection() {
  const pane = $('.pane[data-pane="account"]');
  if (!pane) return;
  if (!pane.dataset.ready) {
    pane.dataset.ready = '1';
    if (IS_WEB) wireWeb();
    else wireApp();
  }
  hint('');
  if (IS_WEB) await renderWeb();
  else await renderApp();
}
