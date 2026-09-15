import { CHECK_SVG } from './dom.js';
import { streamPickerHtml, setupStreamPicker } from './picker.js';
import { t, intlLocale } from './i18n.js';

const ISO_LANGUAGES = [
  { code: 'ita', name: 'Italiano' },
  { code: 'eng', name: 'Inglese' },
  { code: 'spa', name: 'Spagnolo' },
  { code: 'fra', name: 'Francese' },
  { code: 'deu', name: 'Tedesco' },
  { code: 'por', name: 'Portoghese' },
  { code: 'jpn', name: 'Giapponese' },
  { code: 'kor', name: 'Coreano' },
  { code: 'zho', name: 'Cinese' },
  { code: 'rus', name: 'Russo' },
  { code: 'ara', name: 'Arabo' },
  { code: 'hin', name: 'Hindi' },
  { code: 'tur', name: 'Turco' },
  { code: 'pol', name: 'Polacco' },
  { code: 'nld', name: 'Olandese' },
  { code: 'swe', name: 'Svedese' },
  { code: 'dan', name: 'Danese' },
  { code: 'nor', name: 'Norvegese' },
  { code: 'fin', name: 'Finlandese' },
  { code: 'ces', name: 'Ceco' },
  { code: 'slk', name: 'Slovacco' },
  { code: 'hun', name: 'Ungherese' },
  { code: 'ron', name: 'Rumeno' },
  { code: 'ell', name: 'Greco' },
  { code: 'bul', name: 'Bulgaro' },
  { code: 'hrv', name: 'Croato' },
  { code: 'srp', name: 'Serbo' },
  { code: 'slv', name: 'Sloveno' },
  { code: 'ukr', name: 'Ucraino' },
  { code: 'bel', name: 'Bielorusso' },
  { code: 'lav', name: 'Lettone' },
  { code: 'lit', name: 'Lituano' },
  { code: 'est', name: 'Estone' },
  { code: 'heb', name: 'Ebraico' },
  { code: 'tha', name: 'Thai' },
  { code: 'vie', name: 'Vietnamita' },
  { code: 'ind', name: 'Indonesiano' },
  { code: 'msa', name: 'Malese' },
  { code: 'fil', name: 'Filippino' },
  { code: 'ben', name: 'Bengalese' },
  { code: 'tam', name: 'Tamil' },
  { code: 'tel', name: 'Telugu' },
  { code: 'mar', name: 'Marathi' },
  { code: 'urd', name: 'Urdu' },
  { code: 'fas', name: 'Persiano' },
  { code: 'cat', name: 'Catalano' },
  { code: 'eus', name: 'Basco' },
  { code: 'glg', name: 'Galiziano' },
  { code: 'isl', name: 'Islandese' },
  { code: 'gle', name: 'Irlandese' },
  { code: 'cym', name: 'Gallese' },
  { code: 'sqi', name: 'Albanese' },
  { code: 'mkd', name: 'Macedone' },
  { code: 'bos', name: 'Bosniaco' },
  { code: 'mlt', name: 'Maltese' },
  { code: 'lat', name: 'Latino' },
  { code: 'swa', name: 'Swahili' },
  { code: 'afr', name: 'Afrikaans' },
  { code: 'amh', name: 'Amarico' },
  { code: 'yor', name: 'Yoruba' },
  { code: 'hau', name: 'Hausa' },
];

const ISO3_TO_1 = {
  ita: 'it', eng: 'en', spa: 'es', fra: 'fr', deu: 'de', por: 'pt', jpn: 'ja',
  kor: 'ko', zho: 'zh', rus: 'ru', ara: 'ar', hin: 'hi', tur: 'tr', pol: 'pl',
  nld: 'nl', swe: 'sv', dan: 'da', nor: 'nb', fin: 'fi', ces: 'cs', slk: 'sk',
  hun: 'hu', ron: 'ro', ell: 'el', bul: 'bg', hrv: 'hr', srp: 'sr', slv: 'sl',
  ukr: 'uk', bel: 'be', lav: 'lv', lit: 'lt', est: 'et', heb: 'he', tha: 'th',
  vie: 'vi', ind: 'id', msa: 'ms', fil: 'fil', ben: 'bn', tam: 'ta', tel: 'te',
  mar: 'mr', urd: 'ur', fas: 'fa', cat: 'ca', eus: 'eu', glg: 'gl', isl: 'is',
  gle: 'ga', cym: 'cy', sqi: 'sq', mkd: 'mk', bos: 'bs', mlt: 'mt', lat: 'la',
  swa: 'sw', afr: 'af', amh: 'am', yor: 'yo', hau: 'ha',
};

export function localizedLangName(code, fallback) {
  try {
    const loc = intlLocale();
    const dn = new Intl.DisplayNames([loc], { type: 'language' });
    const name = dn.of(ISO3_TO_1[code] || code) || fallback;
    return name ? name.charAt(0).toLocaleUpperCase(loc) + name.slice(1) : name;
  } catch {
    return fallback;
  }
}

export function buildLangItems() {
  return ISO_LANGUAGES.map(l => {
    const name = localizedLangName(l.code, l.name);
    return { value: l.code, label: name, code: l.code, name };
  });
}

export function langPickerHtml(kind) {
  return streamPickerHtml(kind, '', 'lang-pick streams-pick-quiet', {
    searchable: true,
    searchPlaceholderKey: 'settings.language.uiSearch',
  });
}

export function setupLangPicker(rootEl, { values = [], onChange, onClose, popup = false } = {}) {
  return setupStreamPicker(rootEl, {
    popup,
    items: buildLangItems(),
    values,
    multi: true,
    onChange,
    onClose,
    emptyLabel: t('common.none'),
    searchFilter: (item, q) => {
      const ql = q.trim().toLowerCase();
      return item.code.toLowerCase().includes(ql) || item.name.toLowerCase().includes(ql);
    },
    renderOption: (btn, it) => {
      btn.innerHTML = `<span class="lpo-name"></span><span class="lpo-code"></span>${CHECK_SVG}`;
      btn.querySelector('.lpo-name').textContent = it.name;
      btn.querySelector('.lpo-code').textContent = it.code;
    },
    menuMaxHeight: 360,
    menuMinHeight: 180,
  });
}
