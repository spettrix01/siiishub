import { state } from './state.js';
import { EN } from './locales/en.js';
import { IT } from './locales/it.js';
import { DE } from './locales/de.js';
import { ES } from './locales/es.js';
import { FR } from './locales/fr.js';
import { RU } from './locales/ru.js';
import { PT } from './locales/pt.js';
import { NL } from './locales/nl.js';
import { PL } from './locales/pl.js';
import { CS } from './locales/cs.js';
import { HU } from './locales/hu.js';
import { RO } from './locales/ro.js';
import { TR } from './locales/tr.js';
import { SV } from './locales/sv.js';
import { DA } from './locales/da.js';
import { NB } from './locales/nb.js';
import { FI } from './locales/fi.js';
import { EL } from './locales/el.js';
import { UK } from './locales/uk.js';
import { ZH } from './locales/zh.js';
import { JA } from './locales/ja.js';
import { KO } from './locales/ko.js';
import { HI } from './locales/hi.js';

export const APP_LANGUAGES = [
  { code: 'eng', native: 'English',    tmdb: 'en-US', intl: 'en-US', ui: 'en' },
  { code: 'ita', native: 'Italiano',   tmdb: 'it-IT', intl: 'it-IT', ui: 'it' },
  { code: 'spa', native: 'Español',    tmdb: 'es-ES', intl: 'es-ES', ui: 'es' },
  { code: 'fra', native: 'Français',   tmdb: 'fr-FR', intl: 'fr-FR', ui: 'fr' },
  { code: 'deu', native: 'Deutsch',    tmdb: 'de-DE', intl: 'de-DE', ui: 'de' },
  { code: 'por', native: 'Português',  tmdb: 'pt-PT', intl: 'pt-PT', ui: 'pt' },
  { code: 'nld', native: 'Nederlands', tmdb: 'nl-NL', intl: 'nl-NL', ui: 'nl' },
  { code: 'pol', native: 'Polski',     tmdb: 'pl-PL', intl: 'pl-PL', ui: 'pl' },
  { code: 'ces', native: 'Čeština',    tmdb: 'cs-CZ', intl: 'cs-CZ', ui: 'cs' },
  { code: 'hun', native: 'Magyar',     tmdb: 'hu-HU', intl: 'hu-HU', ui: 'hu' },
  { code: 'ron', native: 'Română',     tmdb: 'ro-RO', intl: 'ro-RO', ui: 'ro' },
  { code: 'tur', native: 'Türkçe',     tmdb: 'tr-TR', intl: 'tr-TR', ui: 'tr' },
  { code: 'swe', native: 'Svenska',    tmdb: 'sv-SE', intl: 'sv-SE', ui: 'sv' },
  { code: 'dan', native: 'Dansk',      tmdb: 'da-DK', intl: 'da-DK', ui: 'da' },
  { code: 'nor', native: 'Norsk',      tmdb: 'no',    intl: 'nb-NO', ui: 'nb' },
  { code: 'fin', native: 'Suomi',      tmdb: 'fi-FI', intl: 'fi-FI', ui: 'fi' },
  { code: 'ell', native: 'Ελληνικά',   tmdb: 'el-GR', intl: 'el-GR', ui: 'el' },
  { code: 'rus', native: 'Русский',    tmdb: 'ru-RU', intl: 'ru-RU', ui: 'ru' },
  { code: 'ukr', native: 'Українська', tmdb: 'uk-UA', intl: 'uk-UA', ui: 'uk' },
  { code: 'zho', native: '中文',        tmdb: 'zh-CN', intl: 'zh-CN', ui: 'zh' },
  { code: 'jpn', native: '日本語',      tmdb: 'ja-JP', intl: 'ja-JP', ui: 'ja' },
  { code: 'kor', native: '한국어',      tmdb: 'ko-KR', intl: 'ko-KR', ui: 'ko' },
  { code: 'hin', native: 'हिन्दी',      tmdb: 'hi-IN', intl: 'hi-IN', ui: 'hi' },
];

const DEFAULT_LANG = 'eng';

const BY_CODE = new Map(APP_LANGUAGES.map(l => [l.code, l]));
const DICTS = {
  en: EN, it: IT, es: ES, fr: FR, de: DE, pt: PT, nl: NL, pl: PL, cs: CS, hu: HU,
  ro: RO, tr: TR, sv: SV, da: DA, nb: NB, fi: FI, el: EL, ru: RU, uk: UK,
  zh: ZH, ja: JA, ko: KO, hi: HI,
};

function getLang() {
  const c = state.settings && state.settings.language;
  return BY_CODE.has(c) ? c : DEFAULT_LANG;
}

function langMeta(code = getLang()) {
  return BY_CODE.get(code) || BY_CODE.get(DEFAULT_LANG);
}

export function tmdbLang() {
  return langMeta().tmdb;
}

export function intlLocale() {
  return langMeta().intl;
}

export function t(key, params) {
  const ui = langMeta().ui;
  let s = DICTS[ui] ? DICTS[ui][key] : undefined;
  if (s == null) s = EN[key];
  if (s == null) return key;
  if (params) {
    s = s.replace(/\{(\w+)\}/g, (m, k) => (params[k] != null ? String(params[k]) : m));
  }
  return s;
}

export function locMsg(raw) {
  if (raw == null) return '';
  const s = String(raw);
  const trimmed = s.trim();
  if (trimmed.startsWith('{') && trimmed.endsWith('}')) {
    try {
      const o = JSON.parse(trimmed);
      if (o && typeof o.code === 'string') {
        return t(o.code, o.params && typeof o.params === 'object' ? o.params : undefined);
      }
    } catch {}
  }
  if (/^(progress|error)\.[\w.]+$/.test(trimmed)) return t(trimmed);
  return s;
}

const listeners = new Set();
export function onLangChange(cb) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

function applyTranslations(root = document) {
  root.querySelectorAll('[data-i18n]').forEach(el => { el.textContent = t(el.dataset.i18n); });
  root.querySelectorAll('[data-i18n-html]').forEach(el => { el.innerHTML = t(el.dataset.i18nHtml); });
  root.querySelectorAll('[data-i18n-ph]').forEach(el => el.setAttribute('placeholder', t(el.dataset.i18nPh)));
  root.querySelectorAll('[data-i18n-title]').forEach(el => el.setAttribute('title', t(el.dataset.i18nTitle)));
  root.querySelectorAll('[data-i18n-aria]').forEach(el => el.setAttribute('aria-label', t(el.dataset.i18nAria)));
}

export function setLang(code, { rerender = true } = {}) {
  const next = BY_CODE.has(code) ? code : DEFAULT_LANG;
  if (!state.settings) state.settings = {};
  state.settings.language = next;
  const meta = langMeta(next);
  document.documentElement.setAttribute('lang', meta.intl.split('-')[0]);
  applyTranslations(document);
  if (rerender) {
    for (const cb of listeners) {
      try { cb(next); } catch (e) { console.warn('langchange listener failed', e); }
    }
  }
  return next;
}
