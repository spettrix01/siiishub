import { CHECK_SVG } from './dom.js';
import { t } from './i18n.js';

export function streamPickerHtml(kind, label, extraClass = '', opts = {}) {
  const { searchable = false, searchPlaceholderKey = 'common.search' } = opts;
  const menuClass = 'streams-pick-menu' + (searchable ? ' streams-pick-menu-searchable' : '');
  const searchBlock = searchable ? `
        <div class="streams-pick-search">
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" d="m20 20-4.2-4.2M11 18a7 7 0 1 1 0-14 7 7 0 0 1 0 14Z"/></svg>
          <input type="text" placeholder="${t(searchPlaceholderKey)}" data-i18n-ph="${searchPlaceholderKey}" data-pick-search autocomplete="off" spellcheck="false" />
        </div>` : '';
  return `
    <div class="streams-pick${extraClass ? ' ' + extraClass : ''}" data-stream-pick="${kind}">
      <span class="streams-pick-caption">${label}</span>
      <button type="button" class="streams-pick-trigger" data-pick-trigger aria-haspopup="listbox" aria-expanded="false">
        <span class="streams-pick-label" data-pick-label>—</span>
        <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="m6 9 6 6 6-6"/></svg>
      </button>
      <div class="${menuClass}" role="listbox" hidden data-pick-menu>
        ${searchBlock}
        <div data-pick-options></div>
      </div>
    </div>`;
}

function defaultSearchFilter(item, query) {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return String(item.value).toLowerCase().includes(q) ||
         String(item.label).toLowerCase().includes(q);
}

function normalizeMulti(arr) {
  return [...new Set((Array.isArray(arr) ? arr : []).map(v => String(v)))];
}

export function setupStreamPicker(rootEl, opts = {}) {
  const {
    items = [],
    value,
    values = [],
    multi = false,
    onChange,
    onClose,
    emptyLabel = '—',
    renderOption,
    searchFilter = defaultSearchFilter,
    menuMaxHeight = 320,
    menuMinHeight = 80,
    // popup: options centred on the screen over a backdrop (Android) instead
    // of a dropdown anchored to the trigger.
    popup = false,
  } = opts;

  const trigger = rootEl.querySelector('[data-pick-trigger]');
  const menu = rootEl.querySelector('[data-pick-menu]');
  const labelEl = rootEl.querySelector('[data-pick-label]');
  const searchEl = rootEl.querySelector('[data-pick-search]') || null;
  const optsEl = rootEl.querySelector('[data-pick-options]') || menu;

  let currentItems = items.slice();
  let currentValue = value;
  let order = multi ? normalizeMulti(values) : null;
  let query = '';
  let backdrop = null;

  const findItem = v => currentItems.find(i => String(i.value) === String(v));

  function paintLabel() {
    if (multi) {
      const labels = order.map(v => findItem(v)?.label || v);
      labelEl.textContent = labels.length ? labels.join(', ') : emptyLabel;
      rootEl.classList.toggle('has-value', labels.length > 0);
    } else {
      const sel = findItem(currentValue);
      labelEl.textContent = sel ? sel.label : emptyLabel;
      rootEl.classList.toggle('has-value', !!sel);
    }
  }

  function paintOptions() {
    optsEl.innerHTML = '';
    const filtered = (searchEl && query)
      ? currentItems.filter(it => searchFilter(it, query))
      : currentItems;

    if (!filtered.length && searchEl && query) {
      const empty = document.createElement('div');
      empty.className = 'streams-pick-empty';
      empty.textContent = t('common.noResults');
      optsEl.appendChild(empty);
      return;
    }

    const selectedSet = multi ? new Set(order) : null;
    for (const it of filtered) {
      const btn = document.createElement('button');
      btn.type = 'button';
      const isActive = multi
        ? selectedSet.has(String(it.value))
        : String(it.value) === String(currentValue);
      btn.className = 'streams-pick-option' + (isActive ? ' is-active' : '');
      btn.dataset.value = it.value;
      btn.setAttribute('role', 'option');
      if (renderOption) {
        renderOption(btn, it);
      } else {
        btn.innerHTML = `<span></span>${CHECK_SVG}`;
        btn.querySelector('span').textContent = it.label;
      }
      btn.addEventListener('click', e => {
        e.stopPropagation();
        if (multi) {
          const v = String(it.value);
          const idx = order.indexOf(v);
          if (idx >= 0) order.splice(idx, 1);
          else order.push(v);
          paint();
          onChange?.(order.slice());
        } else {
          if (String(it.value) !== String(currentValue)) {
            currentValue = it.value;
            paint();
            onChange?.(currentValue);
          }
          close();
        }
      });
      optsEl.appendChild(btn);
    }
  }

  function paint() {
    paintLabel();
    paintOptions();
  }

  function positionMenu() {
    if (detachIfOrphaned()) return;
    if (popup) {
      menu.classList.add('is-popup');
      menu.style.position = 'fixed';
      menu.style.left = '50%';
      menu.style.top = '50%';
      menu.style.right = 'auto';
      menu.style.bottom = '';
      menu.style.transform = 'translate(-50%, -50%)';
      menu.style.width = `${Math.min(320, window.innerWidth - 40)}px`;
      menu.style.maxHeight = `${Math.min(Math.round(window.innerHeight * 0.7), 480)}px`;
      menu.style.zIndex = '9999';
      return;
    }
    const r = trigger.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const compact = rootEl.classList.contains('streams-pick-compact');
    const minW = compact ? 250 : r.width;
    const width = Math.min(Math.max(minW, r.width), vw - 16);
    let left = compact ? (r.right - width) : r.left;
    left = Math.max(8, Math.min(left, vw - width - 8));

    const gap = 6;
    const spaceBelow = vh - r.bottom - 8;
    const spaceAbove = r.top - 8;
    let openUp = false;
    let maxH = Math.min(menuMaxHeight, spaceBelow - gap);
    if (maxH < menuMinHeight && spaceAbove > spaceBelow) {
      openUp = true;
      maxH = Math.min(menuMaxHeight, spaceAbove - gap);
    }
    maxH = Math.max(menuMinHeight, maxH);

    menu.style.position = 'fixed';
    menu.style.left = `${left}px`;
    menu.style.width = `${width}px`;
    menu.style.maxHeight = `${maxH}px`;
    menu.style.top = openUp ? '' : `${r.bottom + gap}px`;
    menu.style.bottom = openUp ? `${vh - r.top + gap}px` : '';
    menu.style.right = 'auto';
    menu.style.zIndex = '9999';
  }

  function open() {
    document.dispatchEvent(new CustomEvent('streampicker:open', { detail: { source: rootEl, popup } }));
    if (menu.parentNode !== document.body) document.body.appendChild(menu);
    if (popup && !backdrop) {
      backdrop = document.createElement('div');
      backdrop.className = 'pick-backdrop';
      document.body.insertBefore(backdrop, menu);
    }
    menu.hidden = false;
    positionMenu();
    rootEl.classList.add('is-open');
    trigger.setAttribute('aria-expanded', 'true');
    window.addEventListener('resize', positionMenu);
    window.addEventListener('scroll', positionMenu, true);
    // Not in popup mode: on a phone the keyboard would cover the list.
    if (searchEl && !popup) queueMicrotask(() => searchEl.focus());
  }

  function close() {
    const wasOpen = !menu.hidden;
    menu.hidden = true;
    backdrop?.remove();
    backdrop = null;
    if (menu.parentNode === document.body) rootEl.appendChild(menu);
    rootEl.classList.remove('is-open');
    trigger.setAttribute('aria-expanded', 'false');
    window.removeEventListener('resize', positionMenu);
    window.removeEventListener('scroll', positionMenu, true);
    if (searchEl) {
      searchEl.value = '';
      query = '';
      paintOptions();
    }
    if (wasOpen) {
      onClose?.();
      document.dispatchEvent(new CustomEvent('streampicker:close', { detail: { source: rootEl, popup } }));
    }
  }

  function detachIfOrphaned() {
    if (document.body.contains(rootEl)) return false;
    if (menu.parentNode === document.body) document.body.removeChild(menu);
    backdrop?.remove();
    backdrop = null;
    document.removeEventListener('click', onDocClick);
    document.removeEventListener('streampicker:open', onOtherOpen);
    window.removeEventListener('resize', positionMenu);
    window.removeEventListener('scroll', positionMenu, true);
    return true;
  }

  trigger.addEventListener('click', e => {
    e.stopPropagation();
    if (menu.hidden) open(); else close();
  });

  if (searchEl) {
    searchEl.addEventListener('input', () => {
      query = searchEl.value;
      paintOptions();
    });
    searchEl.addEventListener('click', e => e.stopPropagation());
    searchEl.addEventListener('keydown', e => {
      if (e.key === 'Escape') { close(); trigger.focus(); }
    });
  }

  function onDocClick(e) {
    if (detachIfOrphaned()) return;
    if (!menu.hidden && !rootEl.contains(e.target) && !menu.contains(e.target)) close();
  }
  document.addEventListener('click', onDocClick);

  function onOtherOpen(e) {
    if (detachIfOrphaned()) return;
    if (e.detail?.source !== rootEl && !menu.hidden) close();
  }
  document.addEventListener('streampicker:open', onOtherOpen);

  paint();

  return {
    getValue: () => multi ? null : currentValue,
    setValue: (v, fire = false) => {
      currentValue = v;
      paint();
      if (fire) onChange?.(v);
    },
    getValues: () => multi ? order.slice() : (currentValue == null ? [] : [currentValue]),
    setValues: (arr) => {
      if (multi) order = normalizeMulti(arr);
      else currentValue = Array.isArray(arr) ? arr[0] : arr;
      paint();
    },
    setItems: (newItems) => {
      currentItems = newItems.slice();
      paint();
    },
  };
}
