const TAURI = window.__TAURI__;
const invoke = TAURI?.core?.invoke ?? null;

const cache = new Map();
let initialized = false;

export async function init() {
  if (initialized) return;
  if (!invoke) { initialized = true; return; }
  try {
    const data = await invoke('userdata_load');
    cache.clear();
    if (data && typeof data === 'object') {
      for (const [k, v] of Object.entries(data)) cache.set(k, String(v));
    }
  } catch (e) {
    console.warn('userdata_load failed', e);
  }
  initialized = true;
}

export const userStore = {
  getItem(key) {
    return cache.has(key) ? cache.get(key) : null;
  },
  setItem(key, value) {
    const v = String(value);
    cache.set(key, v);
    if (invoke) invoke('userdata_set', { key, value: v }).catch(e => console.warn('userdata_set', e));
  },
  removeItem(key) {
    cache.delete(key);
    if (invoke) invoke('userdata_remove', { key }).catch(e => console.warn('userdata_remove', e));
  },
  get length() { return cache.size; },
  key(i) {
    if (!Number.isInteger(i) || i < 0) return null;
    let n = 0;
    for (const k of cache.keys()) {
      if (n === i) return k;
      n++;
    }
    return null;
  },
};

export async function migrateFromLocalStorage() {
  if (!invoke) return false;
  if (userStore.getItem('_migrated_v1') === 'true') return false;
  let migrated = 0;
  try {
    const keys = [];
    for (let i = 0; i < window.localStorage.length; i++) {
      const k = window.localStorage.key(i);
      if (!k) continue;
      if (k === 'siiishub-theme'
        || k === 'siiis:dl:seen'
        || k.startsWith('siiis:favorite:')
        || k.startsWith('siiis:resume:')) {
        keys.push(k);
      }
    }
    for (const k of keys) {
      const v = window.localStorage.getItem(k);
      if (v != null && userStore.getItem(k) == null) {
        userStore.setItem(k, v);
        migrated++;
      }
    }
  } catch (e) {
    console.warn('localStorage migration scan failed', e);
  }
  userStore.setItem('_migrated_v1', 'true');
  return migrated > 0;
}
