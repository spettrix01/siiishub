import { $, $$, escapeHTML } from './dom.js';
import { state } from './state.js';
import { closePlayer } from './player.js';
import { t } from './i18n.js';

const detailsModal = $('#detailsModal');
const settingsModal = $('#settingsModal');
const alertModal = $('#alertModal');
const alertCard = alertModal.querySelector('.alert-card');
const alertTitle = $('#alertTitle');
const alertBody = $('#alertBody');
const alertOk = alertModal.querySelector('[data-alert-ok]');
const alertIcon = alertModal.querySelector('[data-alert-icon]');
const defaultIcon = alertIcon.innerHTML;

function setVariant(variant) {
  alertCard.classList.remove('is-error', 'is-warn', 'is-info', 'is-accent');
  alertCard.classList.add(`is-${variant}`);
}

export function closeModal(sel) {
  $(sel).hidden = true;
  if (sel === '#detailsModal') {
    state.detailGen++;
    $('#detailsBody').innerHTML = '';
  }
  if (detailsModal.hidden && settingsModal.hidden && alertModal.hidden) {
    document.body.style.overflow = '';
  }
}

function attachAlertHandlers({ onOk, onCancel, onKey, onOverlay }) {
  const cancelBtn = alertModal.querySelector('[data-alert-cancel]');
  alertOk.addEventListener('click', onOk);
  cancelBtn?.addEventListener('click', onCancel);
  document.addEventListener('keydown', onKey, true);
  alertModal.addEventListener('click', onOverlay);
  setTimeout(() => alertOk.focus(), 0);
  return () => {
    alertOk.removeEventListener('click', onOk);
    cancelBtn?.removeEventListener('click', onCancel);
    document.removeEventListener('keydown', onKey, true);
    alertModal.removeEventListener('click', onOverlay);
  };
}

export function showConfirm(message, {
  title = t('modal.confirmTitle'),
  variant = 'warn',
  okLabel = t('modal.confirm'),
  cancelLabel = t('common.cancel'),
} = {}) {
  setVariant(variant);
  alertTitle.textContent = title;
  alertBody.textContent = message;
  setAlertButtons(okLabel, cancelLabel);
  alertModal.hidden = false;
  document.body.style.overflow = 'hidden';

  return new Promise(resolve => {
    let settled = false;
    let cleanup;
    const settle = (ok) => {
      if (settled) return;
      settled = true;
      cleanup();
      closeModal('#alertModal');
      setAlertButtons(t('common.ok'), null);
      resolve(ok);
    };
    cleanup = attachAlertHandlers({
      onOk: () => settle(true),
      onCancel: () => settle(false),
      onKey: (e) => {
        if (alertModal.hidden) { cleanup(); return; }
        if (e.key === 'Enter') { e.preventDefault(); settle(true); }
        else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); settle(false); }
      },
      onOverlay: (e) => {
        if (e.target.matches('[data-close]')) settle(false);
      },
    });
  });
}

export function showMultiSelectConfirm(items, {
  title = t('modal.selection'),
  description = '',
  variant = 'warn',
  okLabel = t('modal.delete'),
  cancelLabel = t('common.cancel'),
} = {}) {
  setVariant(variant);
  alertTitle.textContent = title;

  const descHtml = description
    ? `<div class="alert-select-desc">${escapeHTML(description)}</div>`
    : '';
  const itemsHtml = items.map((it, i) =>
    `<label class="alert-select-item">`
    + `<input type="checkbox" data-idx="${i}"${it.defaultChecked === false ? '' : ' checked'} />`
    + `<span class="alert-select-label">`
    + `<span class="alert-select-primary">${escapeHTML(it.label || '—')}</span>`
    + (it.sub ? `<span class="alert-select-sub">${escapeHTML(it.sub)}</span>` : '')
    + `</span>`
    + `</label>`
  ).join('');
  alertBody.innerHTML =
    descHtml
    + `<div class="alert-select-list" role="group" data-alert-select>${itemsHtml}</div>`;
  setAlertButtons(okLabel, cancelLabel);
  alertModal.hidden = false;
  document.body.style.overflow = 'hidden';

  const updateOk = () => {
    const checked = alertBody.querySelectorAll('input[type="checkbox"]:checked').length;
    alertOk.textContent = checked > 0 ? `${okLabel} (${checked})` : okLabel;
    alertOk.disabled = checked === 0;
  };
  alertBody.querySelectorAll('input[type="checkbox"]').forEach(cb => {
    cb.addEventListener('change', updateOk);
  });
  updateOk();

  return new Promise(resolve => {
    let settled = false;
    let cleanup;
    const settle = (sel) => {
      if (settled) return;
      settled = true;
      cleanup();
      closeModal('#alertModal');
      setAlertButtons(t('common.ok'), null);
      alertOk.disabled = false;
      alertBody.innerHTML = '';
      resolve(sel);
    };
    const onOk = () => {
      const checks = alertBody.querySelectorAll('input[type="checkbox"]:checked');
      const out = [...checks]
        .map(cb => items[Number(cb.dataset.idx)]?.value)
        .filter(v => v != null);
      settle(out.length ? out : null);
    };
    const onCancel = () => settle(null);
    cleanup = attachAlertHandlers({
      onOk,
      onCancel,
      onKey: (e) => {
        if (alertModal.hidden) { cleanup(); return; }
        if (e.key === 'Enter' && !alertOk.disabled) { e.preventDefault(); onOk(); }
        else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onCancel(); }
      },
      onOverlay: (e) => {
        if (e.target.matches('[data-close]')) onCancel();
      },
    });
  });
}

/**
 * A popup with a form, like the confirmations. `fields` are
 * { name, label, type, autocomplete }; `submit(values)` runs on OK and
 * returns an error, shown under the fields with the popup still open, or
 * nothing to close it. Resolves true once submitted, false if cancelled.
 */
export function showForm({
  title,
  fields,
  icon = '',
  variant = 'accent',
  okLabel = t('common.ok'),
  cancelLabel = t('common.cancel'),
  submit,
}) {
  setVariant(variant);
  if (icon) alertIcon.innerHTML = icon;
  alertTitle.textContent = title;
  alertBody.innerHTML =
    `<form class="alert-form" novalidate>`
    + fields.map(f =>
      `<label class="field">`
      + `<span>${escapeHTML(f.label)}</span>`
      + `<input name="${escapeHTML(f.name)}" type="${escapeHTML(f.type || 'text')}"`
      + ` autocomplete="${escapeHTML(f.autocomplete || 'off')}" autocapitalize="none" spellcheck="false" />`
      + `</label>`
    ).join('')
    + `<p class="hint error" role="alert" data-alert-error></p>`
    + `</form>`;
  const form = alertBody.querySelector('form');
  const errorEl = alertBody.querySelector('[data-alert-error]');
  setAlertButtons(okLabel, cancelLabel);
  alertModal.hidden = false;
  document.body.style.overflow = 'hidden';

  return new Promise(resolve => {
    let settled = false;
    let busy = false;
    let cleanup;
    const settle = (ok) => {
      if (settled) return;
      settled = true;
      cleanup();
      closeModal('#alertModal');
      setAlertButtons(t('common.ok'), null);
      alertOk.disabled = false;
      alertIcon.innerHTML = defaultIcon;
      alertBody.innerHTML = '';
      resolve(ok);
    };
    const onOk = async () => {
      if (busy || settled) return;
      busy = true;
      alertOk.disabled = true;
      const values = Object.fromEntries(new FormData(form));
      const error = await Promise.resolve()
        .then(() => submit(values))
        .catch(e => e?.message || String(e));
      busy = false;
      alertOk.disabled = false;
      if (!error) {
        settle(true);
        return;
      }
      errorEl.textContent = error;
      form.querySelector('input')?.focus();
    };
    const onCancel = () => settle(false);
    // Never a real submit, which would put the fields in the address.
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      onOk();
    });
    cleanup = attachAlertHandlers({
      onOk,
      onCancel,
      onKey: (e) => {
        if (alertModal.hidden) { cleanup(); return; }
        // Enter on Cancel cancels.
        if (e.key === 'Enter' && !e.target.closest?.('[data-alert-cancel]')) { e.preventDefault(); onOk(); }
        else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onCancel(); }
      },
      onOverlay: (e) => {
        if (e.target.matches('[data-close]')) onCancel();
      },
    });
    // After attachAlertHandlers has focused OK: the first field instead.
    setTimeout(() => form.querySelector('input')?.focus(), 0);
  });
}

function setAlertButtons(okLabel, cancelLabel) {
  alertOk.textContent = okLabel;
  let cancel = alertModal.querySelector('[data-alert-cancel]');
  if (cancelLabel) {
    if (!cancel) {
      cancel = document.createElement('button');
      cancel.type = 'button';
      cancel.className = 'ghost ghost--text';
      cancel.dataset.alertCancel = '';
      alertOk.parentElement.insertBefore(cancel, alertOk);
    }
    cancel.textContent = cancelLabel;
  } else if (cancel) {
    cancel.remove();
  }
}

// OK closes the plain alerts; the others close themselves, and a form
// stays open on an error.
alertOk.addEventListener('click', () => {
  if (alertModal.hidden || alertModal.querySelector('[data-alert-cancel]')) return;
  closeModal('#alertModal');
});
document.addEventListener('keydown', e => {
  if (alertModal.hidden) return;
  if (alertModal.querySelector('[data-alert-cancel]')) return;
  if (e.key === 'Escape' || e.key === 'Enter') {
    e.preventDefault();
    closeModal('#alertModal');
  }
});

$$('[data-close]').forEach(el => el.addEventListener('click', e => {
  const modal = e.target.closest('.modal, .dossier, .player');
  if (!modal) return;
  if (modal.classList.contains('player')) closePlayer();
  else closeModal('#' + modal.id);
}));

function forwardScroll(scrimSelector, modalSelector) {
  const scrim = $(scrimSelector);
  const modal = $(modalSelector);
  if (!scrim || !modal) return;
  scrim.addEventListener('wheel', e => {
    const lines = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? modal.clientHeight : 1;
    modal.scrollTop += e.deltaY * lines;
  }, { passive: true });
}
forwardScroll('#detailsModal .dossier-scrim', '#detailsModal');
forwardScroll('#settingsModal .modal-overlay', '#settingsModal');
