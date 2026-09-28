// Settings → Account. In the apps: the SIIISHUB server the app syncs its
// library, favorites, addons, keys and preferences with (account.rs),
// signed in with an account made in the server version. In the browser
// version: who is signed in, their password and, for the administrator,
// the server's accounts and the devices signed in to each
// (server/accounts.rs).
import { $, escapeHTML } from './dom.js';
import { t, intlLocale } from './i18n.js';
import { IS_WEB, IS_ANDROID, IS_TV } from './platform.js';
import { showConfirm, showForm } from './modal.js';
import {
  syncStatus, syncSignIn, syncSignOut, syncNow,
  accountStatus, accountsList, accountCreate, accountDelete, accountPassword,
  accountDeviceRemove,
} from './api.js';
import { refreshAfterSync } from './sync-client.js';

// Kinds as the other sections' hints: success, error, loading.
function hint(text = '', kind = '') {
  const el = $('#accountHint');
  if (!el) return;
  el.className = kind ? `hint ${kind}` : 'hint';
  el.textContent = text;
}

function errorCode(e) {
  return typeof e === 'string' ? e : (e?.message || String(e));
}

// The backend's error codes (account.rs, accounts.rs) in words.
function errorText(e) {
  const code = errorCode(e);
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

// Asked before signing out, with the account's name.
function confirmSignOut(key) {
  return showConfirm(t(key, { user: $('#accountName').textContent }), {
    title: t('settings.account.signOut'),
    variant: 'warn',
    okLabel: t('settings.account.signOut'),
  });
}

// ---------- Apps ----------

async function renderApp({ check = false } = {}) {
  const status = await syncStatus().catch(() => null);
  const signedIn = !!status?.signed_in;
  $('#accountDesc').textContent = t('settings.account.descApp');
  show({
    '#accountSignInForm': !signedIn,
    '#accountCard': signedIn,
    '#accountGuestSignInBtn': false,
    '#accountPasswordBtn': false,
    '#accountAdminBox': false,
  });
  if (signedIn) {
    const when = status.last_sync
      ? t('settings.account.lastSync', { time: syncTime(status.last_sync) })
      : t('settings.account.never');
    card(status.username, `${status.server.replace(/^https?:\/\//, '')} · ${when}`);
    if (check) checkLink();
    else if (status.error) linkState('error', errorText(status.error));
  } else {
    $('#accountLink').hidden = true;
    if (status?.error) hint(errorText(status.error), 'error');
  }
}

// Whether the server answers, under the account's name: a dot and a word.
function linkState(state, text) {
  const el = $('#accountLink');
  el.dataset.state = state;
  el.textContent = text;
  el.hidden = false;
}

// Opened, the section syncs at once: the server answering is the check, and
// the time of the last sync comes up to date with it. A server that no
// longer knows this device signs it out, back to the sign-in form.
let checking = 0;
async function checkLink() {
  const run = ++checking;
  linkState('checking', t('settings.account.checking'));
  try {
    const applied = await syncNow();
    if (run !== checking) return;
    await refreshAfterSync(applied);
    await renderApp();
    linkState('ok', t('settings.account.connected'));
  } catch (err) {
    if (run !== checking) return;
    if (errorCode(err) === 'signed-out') {
      await renderApp();
      hint(errorText(err), 'error');
      return;
    }
    linkState('error', errorText(err));
  }
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
      linkState('ok', t('settings.account.connected'));
      hint(t('settings.account.synced'), 'success');
    } catch (err) {
      hint(errorText(err), 'error');
    } finally {
      btn.disabled = false;
    }
  });
  $('#accountSignOutBtn').addEventListener('click', async () => {
    if (!(await confirmSignOut('settings.account.signOutConfirmApp'))) return;
    await syncSignOut().catch(() => {});
    hint('');
    await renderApp();
  });
}

// ---------- Browser version ----------

let selfId = null;

async function renderWeb() {
  const { account, login } = await accountStatus().catch(() => ({ account: null }));
  selfId = account?.id || null;
  show({
    '#accountSignInForm': false,
    '#accountCard': !!account,
    '#accountGuestSignInBtn': !account && login !== false,
    '#accountPasswordBtn': !!account,
    '#accountAdminBox': !!account?.admin,
  });
  if (!account) {
    $('#accountDesc').textContent = t('settings.account.descGuest');
    return;
  }
  $('#accountDesc').textContent = t('settings.account.descWeb');
  card(account.username, account.admin ? t('settings.account.admin') : '');
  if (account.admin) await renderAccounts();
}

// The head of the popup for a new password: a key.
const PASSWORD_ICON = '<svg viewBox="0 0 24 24" width="22" height="22"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M10.7 12.3 20 3M16.5 6.5 19 9M18.5 4.5l2 2M12 15.5a4.5 4.5 0 1 1-9 0 4.5 4.5 0 0 1 9 0Z"/></svg>';

// A popup asks the current password and the new one; it stays open on an
// error.
async function changePassword() {
  const changed = await showForm({
    title: t('settings.account.changePassword'),
    icon: PASSWORD_ICON,
    fields: [
      { name: 'current', label: t('settings.account.currentPassword'), type: 'password', autocomplete: 'current-password' },
      { name: 'next', label: t('settings.account.newPassword'), type: 'password', autocomplete: 'new-password' },
    ],
    okLabel: t('settings.account.save'),
    submit: async ({ current, next }) => {
      try {
        await accountPassword(current || '', next || '');
      } catch (err) {
        return errorText(err);
      }
      return null;
    },
  });
  if (changed) hint(t('settings.account.passwordChanged'), 'success');
}

// The head of the popup for a new account: a person with a +.
const NEW_ACCOUNT_ICON = '<svg viewBox="0 0 24 24" width="22" height="22"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M15 19a6 6 0 0 0-12 0M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM19 8v6M16 11h6"/></svg>';

// A popup asks the username and the password; it stays open on an error.
async function createAccount() {
  let user = '';
  const created = await showForm({
    title: t('settings.account.newAccount'),
    icon: NEW_ACCOUNT_ICON,
    fields: [
      { name: 'username', label: t('settings.account.username') },
      { name: 'password', label: t('settings.account.password'), type: 'password', autocomplete: 'new-password' },
    ],
    okLabel: t('settings.account.create'),
    // The server says what is wrong with an empty or short field.
    submit: async ({ username, password }) => {
      user = String(username || '').trim();
      try {
        await accountCreate(user, password || '');
      } catch (err) {
        return errorText(err);
      }
      return null;
    },
  });
  if (!created) return;
  hint(t('settings.account.created', { user }), 'success');
  await renderAccounts();
}

// The accounts whose devices are open, kept across a new render of the list.
const openAccounts = new Set();

const CHEVRON = '<svg class="account-chevron" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="m9 6 6 6-6 6"/></svg>';
const KICK_ICON = '<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" d="M6 6l12 12M18 6 6 18"/></svg>';
// A device as its kind: a TV, a phone, a PC's screen.
const DEVICE_ICONS = {
  tv: '<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" d="M3.5 7h17a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1ZM8 3l4 4 4-4"/></svg>',
  phone: '<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" d="M8 3h8a1.5 1.5 0 0 1 1.5 1.5v15A1.5 1.5 0 0 1 16 21H8a1.5 1.5 0 0 1-1.5-1.5v-15A1.5 1.5 0 0 1 8 3Zm3 15h2"/></svg>',
  screen: '<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" d="M4 4.5h16a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1Zm4 15h8m-4-4v4"/></svg>',
};

function deviceIcon(kind = '') {
  if (/\btv\b/i.test(kind)) return DEVICE_ICONS.tv;
  if (/android/i.test(kind)) return DEVICE_ICONS.phone;
  return DEVICE_ICONS.screen;
}

// "Android TV · Amazon AFTKA": what the app said it is when it signed in,
// and the device's own name, which apps tell from 1.3.8 on.
function deviceLabel(d) {
  const kind = d.kind || 'App';
  return d.name ? `${kind} · ${d.name}` : kind;
}

function signedInDate(secs) {
  return new Date(secs * 1000).toLocaleDateString(intlLocale(), { dateStyle: 'medium' });
}

function deviceRow(account, d) {
  const label = deviceLabel(d);
  let state = '';
  if (d.online) state = `<span class="account-device-state is-online">${escapeHTML(t('settings.account.deviceOnline'))}</span>`;
  else if (d.seen) state = `<span class="account-device-state">${escapeHTML(t('settings.account.deviceSeen', { time: syncTime(d.seen) }))}</span>`;
  const facts = [
    d.version ? `v${d.version}` : '',
    d.address || '',
    d.created ? t('settings.account.deviceSince', { date: signedInDate(d.created) }) : '',
  ].filter(Boolean).map(fact => `<span>${escapeHTML(fact)}</span>`);
  const meta = [state, ...facts].filter(Boolean).join('');
  const kick = t('settings.account.deviceKick');
  return `
        <li class="account-device${d.online ? ' is-online' : ''}">
          <span class="account-device-icon">${deviceIcon(d.kind)}</span>
          <span class="account-device-text">
            <span class="account-device-name">${escapeHTML(label)}</span>
            <span class="account-device-meta">${meta}</span>
          </span>
          <button type="button" class="account-device-kick" data-device-kick="${escapeHTML(d.id)}" data-account="${escapeHTML(account.id)}" data-name="${escapeHTML(label)}" data-user="${escapeHTML(account.username)}" title="${escapeHTML(kick)}" aria-label="${escapeHTML(`${kick}: ${label}`)}">${KICK_ICON}</button>
        </li>`;
}

// A row per account; with devices, the row opens their list.
function accountRow(a) {
  const devices = Array.isArray(a.devices) ? a.devices : [];
  const open = devices.length > 0 && openAccounts.has(a.id);
  const listId = `accountDevices-${a.id}`;
  const head = `
        <span class="remote-device-name">${escapeHTML(a.username)}</span>
        ${a.admin ? `<span class="remote-iface-badge">${escapeHTML(t('settings.account.admin'))}</span>` : ''}
        <span class="account-row-meta">${escapeHTML(t('settings.account.devices', { n: devices.length }))}</span>`;
  const toggle = devices.length
    ? `<button type="button" class="account-toggle" aria-expanded="${open}" aria-controls="${escapeHTML(listId)}">${head}${CHEVRON}</button>`
    : `<span class="account-toggle">${head}</span>`;
  const remove = a.id === selfId
    ? ''
    : `<button type="button" class="remote-device-btn is-forget" data-account-delete="${escapeHTML(a.id)}" data-name="${escapeHTML(a.username)}">${escapeHTML(t('settings.account.delete'))}</button>`;
  const list = devices.length
    ? `<ul class="account-devices" id="${escapeHTML(listId)}"${open ? '' : ' hidden'}>${devices.map(d => deviceRow(a, d)).join('')}
      </ul>`
    : '';
  return `
    <li class="account-entry${open ? ' is-open' : ''}" data-account="${escapeHTML(a.id)}">
      <div class="remote-device account-row${devices.length ? ' is-expandable' : ''}">${toggle}${remove}</div>${list}
    </li>`;
}

async function renderAccounts() {
  const list = await accountsList().catch(() => []);
  $('#accountList').innerHTML = list.map(accountRow).join('');
}

function toggleAccount(entry) {
  const toggle = entry.querySelector('button.account-toggle');
  const devices = entry.querySelector('.account-devices');
  if (!toggle || !devices) return;
  const open = devices.hidden;
  devices.hidden = !open;
  entry.classList.toggle('is-open', open);
  toggle.setAttribute('aria-expanded', String(open));
  if (open) openAccounts.add(entry.dataset.account);
  else openAccounts.delete(entry.dataset.account);
}

// Signs one of an account's devices out, once confirmed: its app has to
// sign in again with the password.
async function kickDevice(btn) {
  const { name, user, account } = btn.dataset;
  const ok = await showConfirm(t('settings.account.deviceKickConfirm', { device: name, user }), {
    title: t('settings.account.deviceKick'),
    variant: 'warn',
    okLabel: t('settings.account.deviceKick'),
  });
  if (!ok) return;
  try {
    await accountDeviceRemove(account, btn.dataset.deviceKick);
    hint(t('settings.account.deviceKicked', { device: name }), 'success');
  } catch (err) {
    // Signed out meanwhile, from the app itself: gone all the same.
    if (errorCode(err) !== 'not-found') hint(errorText(err), 'error');
  }
  await renderAccounts();
  $(`#accountList .account-entry[data-account="${CSS.escape(account)}"] .account-toggle`)?.focus();
}

// Closes this browser's session, for the login page: an account's
// sign-out, and signing in from the server's profile.
async function toLogin() {
  await fetch('/api/logout', { method: 'POST', credentials: 'same-origin' }).catch(() => {});
  location.replace('/login');
}

function wireWeb() {
  $('#accountSignOutBtn').addEventListener('click', async () => {
    if (await confirmSignOut('settings.account.signOutConfirmWeb')) await toLogin();
  });
  $('#accountGuestSignInBtn').addEventListener('click', toLogin);
  $('#accountPasswordBtn').addEventListener('click', changePassword);
  $('#accountCreateBtn').addEventListener('click', createAccount);
  $('#accountList').addEventListener('click', async (e) => {
    const kick = e.target.closest('[data-device-kick]');
    if (kick) {
      await kickDevice(kick);
      return;
    }
    const btn = e.target.closest('[data-account-delete]');
    if (!btn) {
      // Anywhere else on an account's row: its devices open or close.
      const row = e.target.closest('.account-row.is-expandable');
      if (row) toggleAccount(row.closest('.account-entry'));
      return;
    }
    const ok = await showConfirm(t('settings.account.deleteConfirm', { user: btn.dataset.name }), {
      title: t('settings.account.delete'),
      variant: 'warn',
      okLabel: t('settings.account.delete'),
    });
    if (!ok) return;
    try {
      await accountDelete(btn.dataset.accountDelete);
      openAccounts.delete(btn.dataset.accountDelete);
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
  else await renderApp({ check: true });
}
