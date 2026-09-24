import { $, $$, CHECK_SVG, COPY_SVG, copyToClipboard, escapeHTML, openExternal } from './dom.js';
import { state, TMDB_API } from './state.js';
import { saveSettings, loadGenres, fetchAddonMeta, openDownloadDir, remoteInfo, onRemoteClientCount, remoteRememberDevice, remoteForgetDevice } from './api.js';
import { renderGenres } from './topbar.js';
import { clearGrid, loadMore } from './grid.js';
import { refreshDlMagnetBarVisibility } from './library.js';
import { streamPickerHtml, setupStreamPicker } from './picker.js';
import { langPickerHtml, setupLangPicker, buildLangItems, localizedLangName } from './lang-picker.js';
import { t, locMsg, setLang, APP_LANGUAGES, onLangChange } from './i18n.js';
import { setTheme, currentTheme } from './theme.js';
import { IS_ANDROID, IS_PHONE, IS_TV } from './platform.js';
import { parseRemoteAddress, canScanQr, scanRemoteQr, connectRemote } from './remote-client.js';

const settingsModal = $('#settingsModal');

function settingsPanes() {
  const panes = [
    { value: 'language', label: t('settings.pane.language') },
    { value: 'appearance', label: t('settings.pane.appearance') },
    { value: 'tmdb', label: t('settings.pane.tmdb') },
    { value: 'addons', label: t('settings.pane.addons') },
    { value: 'trackers', label: t('settings.pane.trackers') },
    { value: 'download', label: t('settings.pane.download') },
    { value: 'remote', label: t('settings.pane.remote') },
  ];
  // Android keeps the torrent tracker list at its defaults. Instead of being
  // controlled, the phone is the remote of another screen (its own Remote
  // section); the TV, like a PC, can be driven by a phone.
  if (IS_TV) return panes.filter(p => p.value !== 'trackers');
  return IS_ANDROID
    ? panes.filter(p => p.value !== 'trackers').map(p => (p.value === 'remote' ? { ...p, value: 'remoteClient' } : p))
    : panes;
}

function nativeLangName(code) {
  const l = APP_LANGUAGES.find(x => x.code === code);
  return l ? l.native : code;
}

function errText(e) {
  const msg = locMsg((e && e.message) || (typeof e === 'string' ? e : String(e)));
  return `${t('common.error')}: ${msg}`;
}

let activePane = 'language';

function paneLabel(panes, value) {
  return panes.find(p => p.value === value)?.label || '';
}

function applyActivePane(pane) {
  activePane = pane;
  $$('.pane', settingsModal).forEach(p => p.classList.toggle('is-active', p.dataset.pane === pane));
  $$('.settings-nav-item', settingsModal).forEach(btn => {
    const on = btn.dataset.pane === pane;
    btn.classList.toggle('is-active', on);
    btn.setAttribute('aria-selected', on ? 'true' : 'false');
  });
  const title = $('#settingsPaneTitle');
  if (title) title.textContent = paneLabel(settingsPanes(), pane);
  if (pane === 'trackers') autoSizeTracker();
  if (pane === 'remote') refreshRemoteInfo();
}

function setupAppearanceSection() {
  const grid = $('#themeGrid');
  if (!grid) return;
  const reflect = () => {
    const cur = currentTheme();
    $$('.theme-option', grid).forEach(btn =>
      btn.setAttribute('aria-checked', btn.dataset.themeValue === cur ? 'true' : 'false'),
    );
  };
  if (!grid.dataset.ready) {
    grid.dataset.ready = '1';
    grid.addEventListener('click', e => {
      const btn = e.target.closest('.theme-option');
      if (!btn) return;
      setTheme(btn.dataset.themeValue);
      reflect();
    });
  }
  reflect();
}

// Left sidebar listing the sections; built once, labels refreshed on language change.
function ensureSettingsNav() {
  const list = $('#settingsNavList');
  if (!list || list.dataset.ready) return;
  list.dataset.ready = '1';
  for (const { value, label } of settingsPanes()) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'settings-nav-item';
    btn.dataset.pane = value;
    btn.setAttribute('role', 'tab');
    btn.setAttribute('aria-selected', 'false');
    btn.textContent = label;
    btn.addEventListener('click', () => applyActivePane(value));
    list.appendChild(btn);
  }
}

function refreshSettingsNavLabels() {
  const panes = settingsPanes();
  $$('.settings-nav-item', settingsModal).forEach(btn => {
    btn.textContent = paneLabel(panes, btn.dataset.pane) || btn.textContent;
  });
  const title = $('#settingsPaneTitle');
  if (title) title.textContent = paneLabel(panes, activePane);
}

export function openSettings(pane = 'language') {
  settingsModal.hidden = false;
  document.body.style.overflow = 'hidden';

  ensureSettingsNav();
  applyActivePane(pane);
  $('#tmdbKeyInput').value = state.settings.tmdbKey || '';

  const saved = state.settings.tracker_fallbacks || [];
  $('#trackerList').value = saved.join('\n');
  autoSizeTracker();

  const dlPath = $('#openDownloadDir .ddl-path');
  if (dlPath) dlPath.textContent = state.settings.downloadDir || '';

  setupAppLanguageSection();
  setupPlayerLangsSection();
  setupAppearanceSection();
  if (IS_PHONE) setupRemoteClientSection();
  else setupRemoteSection();
  setupDebridSection();
  resetHint('#tmdbHint');
  resetHint('#addonHint');
  resetHint('#trackerHint');
  resetHint('#debridHint');
  resetHint('#appLanguageHint');
  resetHint('#playerLangsHint');
  resetHint('#remoteHint');
  resetHint('#remoteClientHint');
  renderAddons();
}

let appLanguagePicker = null;

function appLanguageItems() {
  return APP_LANGUAGES.map(l => ({ value: l.code, label: localizedLangName(l.code, l.native) }));
}

function setupAppLanguageSection() {
  const slot = $('#appLanguageSlot');
  if (!slot) return;
  const current = state.settings.language || 'eng';
  if (!slot.dataset.ready) {
    slot.dataset.ready = '1';
    slot.innerHTML = streamPickerHtml('app-language', '', 'streams-pick-quiet', {
      searchable: true,
      searchPlaceholderKey: 'settings.language.uiSearch',
    });
    const root = slot.querySelector('[data-stream-pick="app-language"]');
    appLanguagePicker = setupStreamPicker(root, {
      items: appLanguageItems(),
      value: current,
      popup: IS_PHONE,
      onChange: async (code) => {
        setHint('#appLanguageHint', t('common.saving'));
        try {
          await saveSettings({ language: code });
          setLang(code);
          setHint('#appLanguageHint', t('settings.language.uiSaved', { lang: nativeLangName(code) }), 'success');
        } catch (e) {
          setHint('#appLanguageHint', errText(e), 'error');
        }
      },
    });
  } else if (appLanguagePicker) {
    appLanguagePicker.setValue(current, false);
  }
}

onLangChange(() => {
  refreshSettingsNavLabels();
  appLanguagePicker?.setItems(appLanguageItems());
  audioLangsPicker?.setItems(buildLangItems());
  subLangsPicker?.setItems(buildLangItems());
  debridProviderPicker?.setItems(debridProviderItems());
});

let debridProviderPicker = null;
let debridProviderValue = '';

function debridProviderItems() {
  return [
    { value: '', label: t('settings.debrid.providerNone') },
    { value: 'rd', label: 'Real-Debrid' },
    { value: 'ad', label: 'AllDebrid' },
  ];
}

const DEBRID_KEY_URLS = {
  rd: { url: 'https://real-debrid.com/apitoken', labelKey: 'settings.debrid.getKeyRd' },
  ad: { url: 'https://alldebrid.com/apikeys', labelKey: 'settings.debrid.getKeyAd' },
};

function refreshDebridTokenField(provider) {
  const tokenInput = $('#debridTokenInput');
  if (!tokenInput) return;
  // The key field only shows up once a provider is chosen.
  const tokenField = $('#debridTokenField');
  if (tokenField) tokenField.hidden = provider === '';
  const savedProvider = state.settings.debridProvider || '';
  const savedToken = state.settings.debridToken || '';
  if (provider === '') {
    tokenInput.value = '';
    tokenInput.placeholder = t('settings.debrid.noProvider');
  } else if (provider === savedProvider && savedToken) {
    tokenInput.value = savedToken;
    tokenInput.placeholder = t('settings.debrid.keyPlaceholder');
  } else {
    tokenInput.value = '';
    tokenInput.placeholder = t('settings.debrid.keyPlaceholder');
  }
  tokenInput.type = 'password';
  const reveal = $('#debridReveal');
  if (reveal) reveal.classList.remove('is-on');

  const link = $('#debridGetKeyLink');
  if (link) {
    const info = DEBRID_KEY_URLS[provider];
    if (info) {
      link.href = info.url;
      link.textContent = t(info.labelKey);
      link.hidden = false;
    } else {
      link.hidden = true;
      link.removeAttribute('href');
      link.textContent = '';
    }
  }
}

function setupDebridSection() {
  const slot = $('#debridProviderSlot');
  const tokenInput = $('#debridTokenInput');
  if (!slot || !tokenInput) return;

  debridProviderValue = state.settings.debridProvider || '';

  if (!slot.dataset.ready) {
    slot.dataset.ready = '1';
    slot.innerHTML = streamPickerHtml('debrid-provider', '', 'streams-pick-quiet');
    const root = slot.querySelector('[data-stream-pick="debrid-provider"]');
    debridProviderPicker = setupStreamPicker(root, {
      items: debridProviderItems(),
      value: debridProviderValue,
      popup: IS_PHONE,
      onChange: v => {
        debridProviderValue = v || '';
        refreshDebridTokenField(debridProviderValue);
        saveDebrid();
      },
    });
  } else if (debridProviderPicker) {
    debridProviderPicker.setValue(debridProviderValue, false);
  }

  refreshDebridTokenField(debridProviderValue);
}

$('#debridGetKeyLink')?.addEventListener('click', (e) => {
  const href = e.currentTarget.getAttribute('href');
  if (href && href !== '#') {
    e.preventDefault();
    openExternal(href);
  }
});

$('#debridReveal')?.addEventListener('click', () => {
  const inp = $('#debridTokenInput');
  const btn = $('#debridReveal');
  if (!inp) return;
  if (inp.type === 'password') {
    inp.type = 'text';
    btn?.classList.add('is-on');
  } else {
    inp.type = 'password';
    btn?.classList.remove('is-on');
  }
});

async function saveDebrid() {
  const provider = debridProviderValue || '';
  const tokenRaw = $('#debridTokenInput').value.trim();
  const savedProvider = state.settings.debridProvider || '';
  const savedToken = state.settings.debridToken || '';

  if (provider !== '' && provider !== savedProvider && tokenRaw.length === 0) {
    setHint('#debridHint', t('settings.debrid.pasteNewKey'), 'error');
    return;
  }
  if (provider !== '' && tokenRaw.length === 0 && savedToken.length === 0) {
    setHint('#debridHint', t('settings.debrid.pasteKey'), 'error');
    return;
  }

  const patch = { debridProvider: provider };
  if (provider === '') {
    patch.debridToken = '';
  } else if (tokenRaw.length > 0) {
    patch.debridToken = tokenRaw;
  }

  setHint('#debridHint', t('common.saving'));
  try {
    await saveSettings(patch);
    const label = provider === 'rd' ? 'Real-Debrid' : provider === 'ad' ? 'AllDebrid' : t('settings.debrid.providerNone');
    if (provider === '') {
      setHint('#debridHint', t('settings.debrid.removed'), 'success');
    } else if (state.settings.debridTokenSet) {
      setHint('#debridHint', t('settings.debrid.configured', { provider: label }), 'success');
    } else {
      setHint('#debridHint', t('settings.debrid.missingToken', { provider: label }), 'error');
    }
    refreshDebridTokenField(debridProviderValue);
    refreshDlMagnetBarVisibility();
  } catch (e) {
    setHint('#debridHint', errText(e), 'error');
  }
}
$('#debridTokenInput')?.addEventListener('change', saveDebrid);

const HINT_ICONS = {
  success: `<svg class="hint-icon" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><circle cx="12" cy="12" r="10" fill="currentColor" opacity="0.18"/><path d="m7 12.5 3.2 3.2L17 9" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  error: `<svg class="hint-icon" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M12 3.5 22 20H2L12 3.5Z" fill="currentColor" opacity="0.18"/><path d="M12 3.5 22 20H2L12 3.5Z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/><path d="M12 10v5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><circle cx="12" cy="17.5" r="1.1" fill="currentColor"/></svg>`,
  loading: `<svg class="hint-icon" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-dasharray="40 60" opacity="0.95"/></svg>`,
  info: `<svg class="hint-icon" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><circle cx="12" cy="12" r="10" fill="currentColor" opacity="0.15"/><circle cx="12" cy="12" r="9.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M12 11v6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><circle cx="12" cy="8" r="1.15" fill="currentColor"/></svg>`,
};

function resolveHintKind(msg, kind) {
  if (kind === 'success' || kind === 'error' || kind === 'loading') return kind;
  if (msg && /[…\.]{1}\s*$/.test(msg) && /…$/.test(msg)) return 'loading';
  return 'info';
}

function renderHint(el, msg, kind) {
  el.className = 'hint ' + kind;
  el.innerHTML = `${HINT_ICONS[kind] || HINT_ICONS.info}<span class="hint-text"></span>`;
  el.querySelector('.hint-text').textContent = msg;
}

function clearHint(el) {
  el.innerHTML = '';
  el.className = 'hint';
}

function resetHint(sel) {
  const el = $(sel);
  if (!el) return;
  clearHint(el);
}

const hintTimers = new Map();

function setHint(sel, msg, kind) {
  const el = $(sel);
  if (!el) return;
  const prev = hintTimers.get(sel);
  if (prev) { clearTimeout(prev); hintTimers.delete(sel); }
  if (!msg) { clearHint(el); return; }
  const resolved = resolveHintKind(msg, kind);
  renderHint(el, msg, resolved);
  const delay = resolved === 'success' ? 3500 : resolved === 'error' ? 6000 : 0;
  if (delay > 0) {
    hintTimers.set(sel, setTimeout(() => {
      hintTimers.delete(sel);
      clearHint(el);
    }, delay));
  }
}

$('#openSettings').addEventListener('click', () => openSettings());

$('#tmdbReveal').addEventListener('click', () => {
  const inp = $('#tmdbKeyInput');
  inp.type = inp.type === 'password' ? 'text' : 'password';
});

async function saveTmdbKey() {
  const key = $('#tmdbKeyInput').value.trim();
  setHint('#tmdbHint', t('settings.tmdb.verifying'));
  try {
    const url = new URL(TMDB_API + '/configuration');
    url.searchParams.set('api_key', key);
    const r = await fetch(url);
    if (!r.ok) throw new Error(t('settings.tmdb.responseStatus', { status: r.status }));
    await saveSettings({ tmdbKey: key });
    setHint('#tmdbHint', t('settings.tmdb.saved'), 'success');
    await loadGenres();
    renderGenres();
    clearGrid();
    loadMore();
  } catch (e) {
    setHint('#tmdbHint', t('settings.tmdb.invalid', { error: e.message }), 'error');
  }
}
$('#tmdbKeyInput').addEventListener('change', saveTmdbKey);

function addonHasResource(a, name) {
  const res = a.resources || [];
  if (res.length === 0) return name === 'stream';
  return res.includes(name);
}

function brandOf(a) {
  const name = (a.name || '').trim();
  if (!name) return t('settings.addons.brandOther');
  const m = name.match(/^[^\s|]+/);
  return m ? m[0] : name;
}

function groupByBrand(items) {
  const brands = new Map();
  for (const a of items) {
    const b = brandOf(a);
    if (!brands.has(b)) brands.set(b, []);
    brands.get(b).push(a);
  }
  const sorted = [...brands.entries()].sort((x, y) =>
    x[0].localeCompare(y[0], undefined, { sensitivity: 'base' })
  );
  for (const [, list] of sorted) {
    list.sort((x, y) => (x.name || '').localeCompare(y.name || '', undefined, { sensitivity: 'base' }));
  }
  return sorted;
}

function appendBrandedGroup(list, label, items) {
  if (!items.length) return;
  const heading = document.createElement('li');
  heading.className = 'addon-group-head';
  heading.textContent = label;
  list.appendChild(heading);
  for (const [brand, brandList] of groupByBrand(items)) {
    const sub = document.createElement('li');
    sub.className = 'addon-brand-head';
    sub.textContent = brand;
    list.appendChild(sub);
    for (const a of brandList) {
      list.appendChild(addonRow(a));
    }
  }
}

function renderAddons() {
  const list = $('#addonList');
  list.innerHTML = '';
  const addons = state.settings.addons || [];
  if (!addons.length) {
    list.innerHTML = `<li class="streams-empty" style="list-style:none">${escapeHTML(t('settings.addons.empty'))}</li>`;
    return;
  }
  const streamAddons = addons.filter(a => addonHasResource(a, 'stream'));
  const subsAddons = addons.filter(a => addonHasResource(a, 'subtitles') && !addonHasResource(a, 'stream'));
  const otherAddons = addons.filter(a =>
    !addonHasResource(a, 'stream') && !addonHasResource(a, 'subtitles')
  );
  appendBrandedGroup(list, t('settings.addons.groupStream'), streamAddons);
  appendBrandedGroup(list, t('settings.addons.groupSubtitles'), subsAddons);
  appendBrandedGroup(list, t('settings.addons.groupOther'), otherAddons);
}

function addonRow(a) {
  const li = document.createElement('li');
  li.className = 'addon-item';

  const info = document.createElement('div');
  info.className = 'addon-meta';
  info.innerHTML = `<p class="name"></p><p class="url"></p>`;
  info.querySelector('.name').textContent = a.name || t('settings.addons.unnamed');
  info.querySelector('.url').textContent = a.url;

  const tog = document.createElement('button');
  tog.className = 'toggle' + (a.enabled !== false ? ' on' : '');
  tog.title = t('settings.addons.toggle');
  tog.onclick = async () => {
    const next = (state.settings.addons || []).map(x =>
      x.url === a.url ? { ...x, enabled: x.enabled === false } : x
    );
    await saveSettings({ addons: next });
    renderAddons();
  };

  const cpy = document.createElement('button');
  cpy.className = 'copybtn';
  cpy.title = t('settings.addons.copyUrl');
  cpy.innerHTML = COPY_SVG;
  cpy.onclick = async () => {
    if (await copyToClipboard(a.url)) {
      cpy.classList.add('is-ok');
      cpy.innerHTML = CHECK_SVG;
      setTimeout(() => {
        cpy.classList.remove('is-ok');
        cpy.innerHTML = COPY_SVG;
      }, 1100);
    }
  };

  const del = document.createElement('button');
  del.className = 'delbtn';
  del.title = t('common.remove');
  del.innerHTML = `<svg viewBox="0 0 24 24" width="16" height="16"><path fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" d="M5 7h14M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2"/></svg>`;
  del.onclick = async () => {
    const next = (state.settings.addons || []).filter(x => x.url !== a.url);
    await saveSettings({ addons: next });
    renderAddons();
  };

  li.appendChild(info);
  li.appendChild(tog);
  li.appendChild(cpy);
  li.appendChild(del);
  return li;
}

let audioLangsPicker = null;
let subLangsPicker = null;
let playerLangsDirty = false;

function setupPlayerLangsSection() {
  const audioSlot = $('#playerAudioLangsSlot');
  const subSlot = $('#playerSubLangsSlot');
  if (!audioSlot || !subSlot) return;

  const savedAudio = state.settings.playerAudioLangs || [];
  const savedSubs = state.settings.playerSubLangs || [];

  if (!audioSlot.dataset.ready) {
    audioSlot.dataset.ready = '1';
    audioSlot.innerHTML = langPickerHtml('player-audio');
    audioLangsPicker = setupLangPicker(audioSlot.querySelector('[data-stream-pick="player-audio"]'), {
      popup: IS_PHONE,
      values: savedAudio,
      onChange: () => { playerLangsDirty = true; },
      onClose: savePlayerLangs,
    });
  } else if (audioLangsPicker) {
    audioLangsPicker.setValues(savedAudio);
  }

  if (!subSlot.dataset.ready) {
    subSlot.dataset.ready = '1';
    subSlot.innerHTML = langPickerHtml('player-sub');
    subLangsPicker = setupLangPicker(subSlot.querySelector('[data-stream-pick="player-sub"]'), {
      popup: IS_PHONE,
      values: savedSubs,
      onChange: () => { playerLangsDirty = true; },
      onClose: savePlayerLangs,
    });
  } else if (subLangsPicker) {
    subLangsPicker.setValues(savedSubs);
  }
}

let remoteClientsListener = null;
let lastDevices = [];

function setupRemoteSection() {
  const portInput = $('#remotePortInput');
  if (portInput) {
    portInput.value = state.settings.remotePort ? String(state.settings.remotePort) : '9871';
  }
  refreshRemoteInfo();
  if (!remoteClientsListener) {
    onRemoteClientCount(devices => renderDevices(devices))
      .then(unlisten => { remoteClientsListener = unlisten; })
      .catch(() => {});
  }
}

const QR_ICON_SVG = `<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.7" d="M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4z"/><path fill="currentColor" d="M14 14h2.6v2.6H14zM17.4 17.4H20V20h-2.6zM14 17.4h2.6V20H14zM17.4 14H20v2.6h-2.6z"/></svg>`;

let lastIfaces = [];

function openQrOverlay(iface) {
  const ov = $('#qrOverlay');
  const svg = $('#qrOverlaySvg');
  const cap = $('#qrOverlayCaption');
  if (!ov || !svg || !cap || !iface) return;
  svg.innerHTML = iface.qr_svg || '';
  cap.textContent = iface.url || '';
  ov.hidden = false;
}

export function closeQrOverlay() {
  const ov = $('#qrOverlay');
  if (ov) ov.hidden = true;
}

$('#qrOverlay')?.addEventListener('click', closeQrOverlay);
document.addEventListener('keydown', (e) => {
  const ov = $('#qrOverlay');
  if (!ov || ov.hidden) return;
  if (e.key === 'Escape') {
    e.preventDefault();
    e.stopPropagation();
    closeQrOverlay();
  }
}, true);

$('#remoteIfaceList')?.addEventListener('click', (e) => {
  const btn = e.target.closest('[data-qr-idx]');
  if (!btn) return;
  openQrOverlay(lastIfaces[Number(btn.dataset.qrIdx)]);
});

async function refreshRemoteInfo() {
  const info = await remoteInfo();
  const box = $('#remoteInfoBox');
  const devicesBox = $('#remoteDevicesBox');
  const list = $('#remoteIfaceList');
  if (!box || !list) return;
  if (!info.running) {
    box.hidden = true;
    if (devicesBox) devicesBox.hidden = true;
    return;
  }
  box.hidden = false;
  lastIfaces = Array.isArray(info.interfaces) ? info.interfaces : [];
  if (!lastIfaces.length) {
    list.innerHTML = `<li class="remote-ip-empty">${escapeHTML(t('settings.remote.noIp'))}</li>`;
  } else {
    list.innerHTML = lastIfaces.map((f, i) => {
      const badge = f.isVirtual
        ? `<span class="remote-iface-badge">${escapeHTML(t('settings.remote.virtualBadge'))}</span>`
        : '';
      return `
      <li class="remote-iface">
        <div class="remote-iface-meta">
          <span class="remote-iface-name">${escapeHTML(f.name || '')}${badge}</span>
          <code class="remote-iface-url">${escapeHTML(f.url || '')}</code>
        </div>
        <button type="button" class="remote-iface-qr" data-qr-idx="${i}" title="${escapeHTML(t('settings.remote.showQr'))}" aria-label="${escapeHTML(t('settings.remote.showQr'))}">${QR_ICON_SVG}</button>
      </li>`;
    }).join('');
  }
  renderDevices(info.devices || []);
}

function renderDevices(devices) {
  lastDevices = Array.isArray(devices) ? devices : [];
  const listEl = $('#remoteDeviceList');
  if (!listEl) return;
  // The group only exists while something is connected.
  const devicesBox = $('#remoteDevicesBox');
  if (devicesBox) devicesBox.hidden = !lastDevices.length;
  // Connected devices first; remembered ones that are offline stay listed.
  const sorted = [...lastDevices].sort((a, b) => Number(b.online !== false) - Number(a.online !== false));
  listEl.innerHTML = sorted.map(d => {
    const online = d.online !== false;
    const stateKey = !online ? 'offline' : d.approved ? 'online' : 'pending';
    const stateLabel = !online
      ? t('settings.remote.offline')
      : d.approved ? t('settings.remote.approved') : t('settings.remote.pendingShort');
    const dot = `<span class="remote-device-dot is-${stateKey}" role="img" title="${escapeHTML(stateLabel)}" aria-label="${escapeHTML(stateLabel)}"></span>`;
    const pendingText = stateKey === 'pending'
      ? `<span class="remote-device-state">${escapeHTML(stateLabel)}</span>`
      : '';
    const remembered = d.remembered
      ? `<span class="remote-iface-badge">${escapeHTML(t('settings.remote.remembered'))}</span>`
      : '';
    // Pairing is the user's choice: approved devices offer "remember",
    // remembered ones (online or not) offer "forget".
    let action = '';
    if (d.remembered) {
      const ref = d.id != null
        ? `data-forget-id="${Number(d.id)}"`
        : `data-forget-key="${escapeHTML(d.key || '')}"`;
      action = `<button type="button" class="remote-device-btn is-forget" ${ref}>${DEVICE_ICONS.forget}<span>${escapeHTML(t('settings.remote.forget'))}</span></button>`;
    } else if (online && d.approved && d.id != null) {
      action = `<button type="button" class="remote-device-btn is-remember" data-remember-id="${Number(d.id)}">${DEVICE_ICONS.remember}<span>${escapeHTML(t('settings.remote.remember'))}</span></button>`;
    }
    return `
    <li class="remote-device${online ? '' : ' is-offline'}">
      ${dot}
      <span class="remote-device-name">${escapeHTML(d.device || t('settings.remote.device'))}</span>
      <code class="remote-device-ip">${escapeHTML(d.ip || '')}</code>
      ${remembered}
      ${pendingText}
      ${action}
    </li>`;
  }).join('');
}

const DEVICE_ICONS = {
  remember: `<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" d="M6 4h12v17l-6-4-6 4z"/></svg>`,
  forget: `<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" d="M6 6l12 12M18 6 6 18"/></svg>`,
};

$('#remoteDeviceList')?.addEventListener('click', (e) => {
  const btn = e.target.closest('[data-remember-id], [data-forget-id], [data-forget-key]');
  if (!btn) return;
  btn.disabled = true;
  const done = () => { btn.disabled = false; };
  if (btn.dataset.rememberId != null) {
    remoteRememberDevice(Number(btn.dataset.rememberId)).finally(done);
  } else {
    const id = btn.dataset.forgetId != null ? Number(btn.dataset.forgetId) : null;
    remoteForgetDevice(id, btn.dataset.forgetKey || null).finally(done);
  }
});

async function saveRemote() {
  const portRaw = Number($('#remotePortInput')?.value || 9871);
  const port = Number.isFinite(portRaw) && portRaw >= 1024 && portRaw <= 65535 ? portRaw : 9871;
  setHint('#remoteHint', t('common.saving'));
  try {
    await saveSettings({ remotePort: port });
    await new Promise(r => setTimeout(r, 200));
    await refreshRemoteInfo();
    setHint('#remoteHint', t('settings.remote.active', { port }), 'success');
  } catch (e) {
    setHint('#remoteHint', errText(e), 'error');
  }
}
$('#remotePortInput')?.addEventListener('change', saveRemote);

// Android: the phone as the remote of a PC (remote-client.js). The PC is
// reached by framing the QR code of its Remote section, or by its address.
async function setupRemoteClientSection() {
  const scanBtn = $('#remoteScanBtn');
  if (!scanBtn) return;
  const canScan = await canScanQr();
  scanBtn.hidden = !canScan;
  if (!canScan) setHint('#remoteClientHint', t('settings.remoteClient.scanUnavailable'), 'info');
}

// A newer action (another address, a scan) supersedes a pending connection.
let remoteClientRequest = 0;

async function openRemoteClient(entry) {
  const request = ++remoteClientRequest;
  setHint('#remoteClientHint', t('settings.remoteClient.connecting', { host: entry.host }));
  const result = await connectRemote(entry);
  if (request !== remoteClientRequest) return;
  if (result === 'ok') {
    // The address stays in the field: connecting again is one tap.
    const input = $('#remoteAddressInput');
    if (input) input.value = entry.host;
    setHint('#remoteClientHint', '');
  } else if (result === 'unreachable') {
    setHint('#remoteClientHint', t('settings.remoteClient.unreachable', { host: entry.host }), 'error');
  } else {
    setHint('#remoteClientHint', '');
  }
}

$('#remoteScanBtn')?.addEventListener('click', async () => {
  remoteClientRequest++;
  setHint('#remoteClientHint', '');
  let entry;
  try {
    entry = await scanRemoteQr();
  } catch (e) {
    const key = e?.name === 'NotAllowedError' ? 'settings.remoteClient.cameraDenied' : 'settings.remoteClient.cameraError';
    setHint('#remoteClientHint', t(key), 'error');
    return;
  }
  if (entry) openRemoteClient(entry);
});

$('#remoteConnectBtn')?.addEventListener('click', () => {
  const entry = parseRemoteAddress($('#remoteAddressInput')?.value);
  if (!entry) {
    remoteClientRequest++;
    setHint('#remoteClientHint', t('settings.remoteClient.invalid'), 'error');
    return;
  }
  openRemoteClient(entry);
});

async function savePlayerLangs() {
  if (!playerLangsDirty) return;
  playerLangsDirty = false;
  const audio = audioLangsPicker ? audioLangsPicker.getValues() : [];
  const subs = subLangsPicker ? subLangsPicker.getValues() : [];
  setHint('#playerLangsHint', t('common.saving'));
  try {
    await saveSettings({ playerAudioLangs: audio, playerSubLangs: subs });
    setHint('#playerLangsHint', t('settings.language.tracksSaved'), 'success');
  } catch (e) {
    setHint('#playerLangsHint', errText(e), 'error');
  }
}

async function saveTrackers() {
  const lines = $('#trackerList').value
    .split('\n')
    .map(s => s.trim())
    .filter(Boolean);
  setHint('#trackerHint', t('common.saving'));
  try {
    await saveSettings({ trackerFallbacks: lines });
    setHint(
      '#trackerHint',
      lines.length
        ? t('settings.trackers.saved', { count: lines.length })
        : t('settings.trackers.emptyDefault'),
      'success'
    );
  } catch (e) {
    setHint('#trackerHint', errText(e), 'error');
  }
}
$('#trackerList').addEventListener('change', saveTrackers);

function autoSizeTracker() {
  const el = $('#trackerList');
  if (!el) return;
  el.style.height = 'auto';
  el.style.height = `${el.scrollHeight}px`;
}
$('#trackerList').addEventListener('input', autoSizeTracker);

$('#openDownloadDir').addEventListener('click', () => {
  openDownloadDir().catch(() => {});
});

$('#addAddon').addEventListener('click', async () => {
  const url = $('#addonUrl').value.trim();
  if (!url) { setHint('#addonHint', t('settings.addons.urlRequired'), 'error'); return; }
  setHint('#addonHint', t('settings.addons.verifyingManifest'));
  try {
    const meta = await fetchAddonMeta(url);
    const resourceNames = (meta.resources || [])
      .map(res => (typeof res === 'string' ? res : res?.name || ''))
      .filter(Boolean);
    const supportsStream = resourceNames.includes('stream');
    const supportsSubs = resourceNames.includes('subtitles');
    if (!supportsStream && !supportsSubs) {
      setHint('#addonHint', t('settings.addons.noResource'), 'error');
      return;
    }
    const finalName = meta.name || 'Addon';
    const next = [
      ...(state.settings.addons || []).filter(a => a.url !== url),
      { name: finalName, url, enabled: true, resources: resourceNames },
    ];
    await saveSettings({ addons: next });
    $('#addonUrl').value = '';
    const kindLabel = supportsStream ? 'stream' : 'subtitles';
    setHint('#addonHint', t('settings.addons.added', { name: finalName, kind: kindLabel }), 'success');
    renderAddons();
  } catch (e) {
    setHint('#addonHint', errText(e), 'error');
  }
});
