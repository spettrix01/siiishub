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
  alertCard.classList.remove('is-error', 'is-warn', 'is-info');
  alertCard.classList.add(`is-${variant}`);
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
        else if (e.key === 'Escape') { e.preventDefault(); settle(false); }
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
  alertCard.classList.remove('is-error', 'is-warn', 'is-info');
  alertCard.classList.add(`is-${variant}`);
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
        else if (e.key === 'Escape') { e.preventDefault(); onCancel(); }
      },
      onOverlay: (e) => {
        if (e.target.matches('[data-close]')) onCancel();
      },
    });
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

alertOk.addEventListener('click', () => {
  if (!alertModal.hidden) closeModal('#alertModal');
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
