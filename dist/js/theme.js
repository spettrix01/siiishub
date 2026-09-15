import { userStore } from './userstore.js';

const THEMES = ['dark-orange', 'light-orange', 'dark-purple', 'light-purple', 'dark-teal', 'light-teal'];
const DEFAULT_THEME = 'dark-orange';

const ACCENTS = ['orange', 'purple', 'teal'];

function parse(theme) {
  const value = THEMES.includes(theme) ? theme : DEFAULT_THEME;
  const [mode, accent] = value.split('-');
  return { value, mode, accent };
}

export function currentTheme() {
  const mode = userStore.getItem('siiishub-theme') === 'light' ? 'light' : 'dark';
  const saved = userStore.getItem('siiishub-accent');
  const accent = ACCENTS.includes(saved) ? saved : 'orange';
  return `${mode}-${accent}`;
}

export function applyTheme(theme) {
  const { mode, accent } = parse(theme);
  const root = document.documentElement;
  if (mode === 'light') root.setAttribute('data-theme', 'light');
  else root.removeAttribute('data-theme');
  if (accent !== 'orange') root.setAttribute('data-accent', accent);
  else root.removeAttribute('data-accent');
}

export function setTheme(theme) {
  const { value, mode, accent } = parse(theme);
  userStore.setItem('siiishub-theme', mode);
  userStore.setItem('siiishub-accent', accent);
  applyTheme(value);
  return value;
}
