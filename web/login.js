// Login page of siiishub-server (server/auth.rs): an account's username and
// password; from the home network, the server's profile without an account;
// at the first start, the administrator's account. The texts come from the
// app's translations, in the browser's language.
const LOCALES = ['cs', 'da', 'de', 'el', 'en', 'es', 'fi', 'fr', 'hi', 'hu', 'it', 'ja', 'ko',
  'nb', 'nl', 'pl', 'pt', 'ro', 'ru', 'sv', 'tr', 'uk', 'zh'];

const $ = (id) => document.getElementById(id);

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

// Back where the server sent the browser from (the phone remote's page),
// never to another site.
function enter() {
  const next = new URLSearchParams(location.search).get('next') || '';
  location.replace(/^\/(?![/\\])/.test(next) ? next : '/');
}

const MESSAGES = {
  'wrong-credentials': ['web.login.wrongCredentials', 'Wrong username or password.'],
  'wrong-password': ['web.login.wrong', 'Wrong password.'],
  'too-many-attempts': ['web.login.tooMany', 'Too many attempts: try again in a minute.'],
  'not-home': ['web.login.setupAtHome', 'Create the first account from a device on the server\'s home network.'],
  'username-invalid': ['settings.account.error.username-invalid', 'Usernames take letters, digits, dots, dashes and underscores, up to 32.'],
  'username-taken': ['settings.account.error.username-taken', 'That username is taken.'],
  'password-short': ['settings.account.error.password-short', 'The password needs at least 6 characters.'],
};

function message(code) {
  const [key, fallback] = MESSAGES[code] || ['web.login.unreachable', 'The server does not answer.'];
  return t(key, fallback);
}

// Posts `body` to `url`: signed in, the app opens; else `errorEl` says why.
async function submit(url, body, errorEl, button) {
  errorEl.hidden = true;
  if (button) button.disabled = true;
  try {
    const res = await fetch(url, {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (res.ok) {
      enter();
      return;
    }
    const reply = await res.json().catch(() => ({}));
    errorEl.textContent = message(reply.error);
  } catch {
    errorEl.textContent = message('');
  } finally {
    if (button) button.disabled = false;
  }
  errorEl.hidden = false;
}

$('loginForm').addEventListener('submit', (e) => {
  e.preventDefault();
  submit('/api/login', { username: $('loginUser').value.trim(), password: $('loginPassword').value },
    $('loginError'), $('loginSubmit'));
});

$('setupForm').addEventListener('submit', (e) => {
  e.preventDefault();
  submit('/api/account/setup', { username: $('setupUser').value.trim(), password: $('setupPassword').value },
    $('setupError'), e.submitter);
});

$('guestBtn').addEventListener('click', () => {
  submit('/api/login/guest', {}, $('loginError'), $('guestBtn'));
});

async function start() {
  await translate();
  const options = await fetch('/api/login/options', { credentials: 'same-origin' })
    .then(r => r.json())
    .catch(() => ({}));
  // No account yet: the administrator's is made here, from the home network.
  // Signing in stays for the password of the versions before accounts.
  const setup = !!options.setup;
  $('setupForm').hidden = !(setup && options.home);
  $('loginForm').hidden = setup && !options.password;
  $('setupAtHome').hidden = !(setup && !options.home && !options.password);
  $('guestBox').hidden = !options.guest;
  const first = [$('setupUser'), $('loginUser')].find(el => !el.closest('form').hidden);
  first?.focus();
}

start();
