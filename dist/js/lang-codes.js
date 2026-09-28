// Language codes as files, addons and the settings write them: ISO 639-1
// (`it`), ISO 639-2 in its B and T forms (`fre`, `fra`), or with a region
// after them (`pt-BR`). langKey() gives each language one key, and none
// (empty) to an undetermined one. The players use it to pick tracks: the
// app's (js/player.js) and the browser's (web/player.js).
const LANGS = {
  en: ['eng'], it: ['ita'], es: ['spa'], fr: ['fre', 'fra'], de: ['ger', 'deu'], pt: ['por'],
  nl: ['dut', 'nld'], pl: ['pol'], ru: ['rus'], uk: ['ukr'], cs: ['cze', 'ces'], sk: ['slo', 'slk'],
  hu: ['hun'], ro: ['rum', 'ron'], el: ['gre', 'ell'], bg: ['bul'], hr: ['hrv'], sr: ['srp'],
  sl: ['slv'], bs: ['bos'], mk: ['mac', 'mkd'], sq: ['alb', 'sqi'], tr: ['tur'], sv: ['swe'],
  da: ['dan'], fi: ['fin'], nb: ['nob', 'nor'], is: ['ice', 'isl'], et: ['est'], lv: ['lav'],
  lt: ['lit'], be: ['bel'], ja: ['jpn'], ko: ['kor'], zh: ['chi', 'zho'], ar: ['ara'],
  he: ['heb'], hi: ['hin'], fa: ['per', 'fas'], th: ['tha'], vi: ['vie'], id: ['ind'],
  ms: ['may', 'msa'], ca: ['cat'], eu: ['baq', 'eus'], gl: ['glg'], ga: ['gle'], cy: ['wel', 'cym'],
};
// Two-letter codes of the same language, and codes for none.
const ALIASES = { no: 'nb', iw: 'he', in: 'id' };
const UNDETERMINED = ['und', 'unk', 'zxx'];

export function langKey(code) {
  const c = String(code || '').trim().toLowerCase().split(/[-_]/)[0];
  if (!c || UNDETERMINED.includes(c)) return '';
  if (c.length === 2) return ALIASES[c] || c;
  for (const [two, three] of Object.entries(LANGS)) if (three.includes(c)) return two;
  return c;
}

/** Two codes of one language, neither of them empty. */
export function sameLang(a, b) {
  return !!a && !!b && langKey(a) === langKey(b);
}
