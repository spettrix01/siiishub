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

// Posts `body` to `url`: signed in, the app opens; else `errorEl` says why
// and the error's code comes back.
async function submit(url, body, errorEl, button) {
  errorEl.hidden = true;
  if (button) button.disabled = true;
  let code = '';
  try {
    const res = await fetch(url, {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (res.ok) {
      enter();
      return null;
    }
    code = (await res.json().catch(() => ({}))).error || '';
  } catch {
    // Unreachable: the message says so.
  } finally {
    if (button) button.disabled = false;
  }
  errorEl.textContent = message(code);
  errorEl.hidden = false;
  return code;
}

$('loginForm').addEventListener('submit', (e) => {
  e.preventDefault();
  submit('/api/login', { username: $('loginUser').value.trim(), password: $('loginPassword').value },
    $('loginError'), $('loginSubmit'));
});

$('setupForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const code = await submit('/api/account/setup',
    { username: $('setupUser').value.trim(), password: $('setupPassword').value },
    $('setupError'), e.submitter);
  // Made meanwhile from another browser: this one signs in now.
  if (code === 'setup-done') {
    $('setupError').hidden = true;
    show();
  }
});

$('guestBtn').addEventListener('click', () => {
  submit('/api/login/guest', {}, $('guestError'), $('guestBtn'));
});

// One form at a time. No account yet: from the home network the
// administrator's is made, and that is all; from outside only the password
// of the versions before accounts signs in, when the server has one.
async function show() {
  const options = await fetch('/api/login/options', { credentials: 'same-origin' })
    .then(r => r.json())
    .catch(() => ({}));
  const setup = !!options.setup;
  const setupHere = setup && !!options.home;
  const passwordOnly = setup && !options.home && !!options.password;
  $('setupForm').hidden = !setupHere;
  $('loginForm').hidden = setupHere || (setup && !options.password);
  $('loginUserLabel').hidden = $('loginUser').hidden = passwordOnly;
  $('setupAtHome').hidden = !(setup && !options.home && !options.password);
  $('guestBox').hidden = !options.guest;
  const first = [$('setupUser'), $('loginUser'), $('loginPassword')]
    .find(el => !el.hidden && !el.closest('form').hidden);
  first?.focus();
}

async function start() {
  await translate();
  await show();
}

start();
