// Login page of siiishub-server: sends the password and opens the app. The
// texts come from the app's translations, in the browser's language.
const LOCALES = ['cs', 'da', 'de', 'el', 'en', 'es', 'fi', 'fr', 'hi', 'hu', 'it', 'ja', 'ko',
  'nb', 'nl', 'pl', 'pt', 'ro', 'ru', 'sv', 'tr', 'uk', 'zh'];

const form = document.getElementById('loginForm');
const input = document.getElementById('loginPassword');
const errorEl = document.getElementById('loginError');
const submit = document.getElementById('loginSubmit');

let strings = {};
const t = (key, fallback) => strings[key] || fallback;

function browserLocale() {
  for (const tag of navigator.languages || [navigator.language || 'en']) {
    let code = String(tag).toLowerCase().split('-')[0];
    if (code === 'no' || code === 'nn') code = 'nb';
    if (LOCALES.includes(code)) return code;
  }
  return 'en';
}

async function loadLocale(code) {
  try {
    return Object.values(await import(`/js/locales/${code}.js`))[0] || {};
  } catch {
    return {};
  }
}

async function translate() {
  const code = browserLocale();
  const [en, local] = await Promise.all([loadLocale('en'), code === 'en' ? {} : loadLocale(code)]);
  strings = { ...en, ...local };
  document.documentElement.lang = code;
  for (const el of document.querySelectorAll('[data-i18n]')) {
    const text = strings[el.dataset.i18n];
    if (text) el.textContent = text;
  }
}

function showError(message) {
  errorEl.textContent = message;
  errorEl.hidden = false;
  input.select();
  input.focus();
}

form.addEventListener('submit', async (e) => {
  e.preventDefault();
  errorEl.hidden = true;
  submit.disabled = true;
  try {
    const res = await fetch('/api/login', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ password: input.value }),
    });
    if (res.ok) {
      // Back where the server sent the browser from (the phone remote's
      // page), never to another site.
      const next = new URLSearchParams(location.search).get('next') || '';
      location.replace(/^\/(?![/\\])/.test(next) ? next : '/');
      return;
    }
    const body = await res.json().catch(() => ({}));
    showError(body.error === 'too-many-attempts'
      ? t('web.login.tooMany', 'Too many attempts: try again in a minute.')
      : t('web.login.wrong', 'Wrong password.'));
  } catch {
    showError(t('web.login.unreachable', 'The server does not answer.'));
  } finally {
    submit.disabled = false;
  }
});

translate();
