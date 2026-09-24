// Android phone: the text fields of the settings (TMDB key, addon URL,
// debrid key, tracker list) and of the player settings (track search, IMDb
// id) are edited in a popup centred on the screen, like the dropdowns (the
// TV edits them in place, as on a PC: tv-nav.js keeps left, right and OK
// with the field). Tapping a field opens the popup instead of the keyboard;
// OK copies the text back into the field and fires its input/change
// handlers, so saving keeps its single code path. android-back.js closes
// the popup on Back through the siiis:popup-open/close events.
import { escapeHTML } from './dom.js';
import { IS_PHONE } from './platform.js';
import { t } from './i18n.js';

const FIELDS = '#settingsModal input[type="text"], #settingsModal input[type="password"], #settingsModal input[type="url"], #settingsModal textarea, #playerSettings input[type="text"], #playerSettings input[type="search"]';
const EYE_SVG = '<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.7" d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="1.7"/></svg>';

let current = null; // { popup, close, field }

// The field's caption, else the title of the settings section it sits in
// or the active player settings tab (the addon URL has no caption of its own).
function labelFor(field) {
  const caption = field.closest('label.field')?.querySelector(':scope > span');
  const paneTitle = field.closest('.pane') ? document.getElementById('settingsPaneTitle') : null;
  const playerTab = field.closest('#playerSettings')?.querySelector('[data-pst-tab].is-active');
  return (caption?.textContent || field.getAttribute('aria-label') || paneTitle?.textContent || playerTab?.textContent || '').trim();
}

function editableField(target) {
  const field = target instanceof Element ? target.closest(FIELDS) : null;
  return field && !field.disabled && !field.readOnly ? field : null;
}

function openEditor(field) {
  if (current) current.close();
  const isArea = field.tagName === 'TEXTAREA';
  const secret = field.type === 'password';
  // A field paired with an action button (the addon URL and its Add button,
  // the IMDb id and its Search button) runs that action on OK: one step.
  const submit = field.parentElement?.querySelector(':scope > button.primary, :scope > [data-os-imdb-go]') || null;

  const backdrop = document.createElement('div');
  backdrop.className = 'pick-backdrop';
  const popup = document.createElement('div');
  popup.className = 'input-popup';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-modal', 'true');
  popup.innerHTML = `
    <p class="input-popup-title"></p>
    <div class="input-popup-row">
      ${isArea
        ? '<textarea rows="5" spellcheck="false"></textarea>'
        : `<input type="${secret ? 'password' : 'text'}" autocomplete="off" autocapitalize="off" spellcheck="false">`}
      ${secret ? `<button type="button" class="ghost input-popup-reveal" aria-label="${escapeHTML(t('common.revealToggle'))}">${EYE_SVG}</button>` : ''}
    </div>
    <div class="input-popup-actions">
      <button type="button" class="primary" data-ok></button>
    </div>`;
  popup.querySelector('.input-popup-title').textContent = labelFor(field);
  popup.querySelector('[data-ok]').textContent = submit?.textContent.trim() || t('common.ok');
  const editor = popup.querySelector('input, textarea');
  editor.placeholder = field.placeholder || '';
  editor.value = field.value;

  const close = () => {
    if (current?.popup !== popup) return;
    current = null;
    popup.remove();
    backdrop.remove();
    document.dispatchEvent(new CustomEvent('siiis:popup-close', { detail: { source: popup } }));
  };
  const commit = () => {
    if (editor.value !== field.value) {
      field.value = editor.value;
      field.dispatchEvent(new Event('input', { bubbles: true }));
      field.dispatchEvent(new Event('change', { bubbles: true }));
    }
    close();
    submit?.click();
  };
  // No Cancel button: Back or a tap on the backdrop closes without saving.
  popup.querySelector('[data-ok]').addEventListener('click', commit);
  backdrop.addEventListener('click', close);
  popup.querySelector('.input-popup-reveal')?.addEventListener('click', () => {
    editor.type = editor.type === 'password' ? 'text' : 'password';
  });
  editor.addEventListener('keydown', e => {
    if (e.key === 'Enter' && !isArea) { e.preventDefault(); commit(); }
    else if (e.key === 'Escape') { e.preventDefault(); close(); }
  });

  current = { popup, close, field };
  document.body.append(backdrop, popup);
  document.dispatchEvent(new CustomEvent('siiis:popup-open', { detail: { source: popup, close } }));
  editor.focus();
  try { editor.setSelectionRange(editor.value.length, editor.value.length); } catch { /* not a text-like input */ }
}

if (IS_PHONE) {
  // A tap on a field must not focus it, or the keyboard would open for the
  // page underneath: cancel its pointerdown. The popup opens on the click
  // that ends the tap. Opening it on pointerdown let that same click land on
  // the freshly added backdrop, which closed the popup at once whenever the
  // field lay outside the popup's box: a field low on the page, or any field
  // while the page was still shortened by a keyboard that was closing.
  document.addEventListener('pointerdown', e => {
    if (editableField(e.target)) e.preventDefault();
  }, true);
  document.addEventListener('click', e => {
    const field = editableField(e.target);
    if (!field) return;
    e.preventDefault();
    if (current?.field !== field) openEditor(field);
  }, true);
  // A tap on the field's caption activates the <label>, which focuses the
  // field (and then clicks it, handled above as the same field).
  document.addEventListener('focusin', e => {
    const field = editableField(e.target);
    if (!field || current) return;
    field.blur();
    openEditor(field);
  });
}
