import { $, $$, CHECK_SVG, FORM_TAGS } from './dom.js';
import { state } from './state.js';
import { t, onLangChange } from './i18n.js';
import { clearGrid, loadMore } from './grid.js';
import { closePlayer, isPlayerOpen } from './player.js';
import { closeModal } from './modal.js';
import { refreshTrending } from './trending.js';
import { loadLibrary, refreshDlMagnetBarVisibility } from './library.js';

const detailsModal = $('#detailsModal');
const settingsModal = $('#settingsModal');
const trendingRail = $('#trendingRail');

const filterBar = $('#genreDropdown').closest('.filter-bar');
const libraryBar = $('#libraryBar');

function setLibraryMode(active) {
  if (libraryBar) libraryBar.hidden = !active;
  if (filterBar) filterBar.hidden = active;
  if (active) {
    if (trendingRail) trendingRail.hidden = true;
    clearGrid();
    loadLibrary();
    refreshDlMagnetBarVisibility();
  } else {
    const magnetBar = document.getElementById('dlMagnetBar');
    if (magnetBar) magnetBar.hidden = true;
  }
}

$$('.tab').forEach(btn => {
  btn.addEventListener('click', () => {
    if (btn.classList.contains('is-active')) return;
    const target = btn.dataset.section;
    if (state.section === 'library' && target !== 'library') setLibraryMode(false);
    $$('.tab').forEach(b => {
      b.classList.remove('is-active');
      b.setAttribute('aria-selected', 'false');
    });
    btn.classList.add('is-active');
    btn.setAttribute('aria-selected', 'true');
    state.section = target;
    state.selectedGenre = null;
    if (target === 'library') {
      setLibraryMode(true);
      return;
    }
    renderGenres();
    refreshTrending();
    clearGrid();
    loadMore();
  });
});

$('[data-go-movies]')?.addEventListener('click', () => {
  const searchEl = $('#search');
  if (searchEl.value !== '') {
    searchEl.value = '';
    state.query = '';
    clearGrid();
    loadMore();
  }
  $('.tab[data-section="movie"]')?.click();
  (document.getElementById('page-scroll') || window).scrollTo({ top: 0, behavior: 'smooth' });
});

const genreTrigger = $('#genreTrigger');
const genreMenu = $('#genreMenu');
const genreLabel = $('.genre-label', genreTrigger);

function genreList() {
  return state.section === 'series' ? state.genres.tv : state.genres.movie;
}

export function renderGenres() {
  const allItem = { id: null, name: t('genre.all') };
  const koreanItem = { id: 'lang:ko', name: t('genre.korean') };
  const items = [allItem, koreanItem, ...genreList()];
  const active = state.selectedGenre;
  const selected = active != null ? items.find(g => g.id === active) : null;
  genreLabel.textContent = selected ? selected.name : allItem.name;
  genreTrigger.classList.toggle('has-value', !!selected);

  genreMenu.innerHTML = '';
  for (const g of items) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'genre-option' + (active === g.id ? ' is-active' : '');
    btn.setAttribute('role', 'option');
    btn.innerHTML = `<span></span>${CHECK_SVG}`;
    btn.querySelector('span').textContent = g.name;
    btn.onclick = () => {
      state.selectedGenre = g.id;
      closeGenreMenu();
      renderGenres();
      clearGrid();
      loadMore();
    };
    genreMenu.appendChild(btn);
  }
}

function openGenreMenu() {
  genreMenu.hidden = false;
  genreTrigger.classList.add('is-open');
  genreTrigger.setAttribute('aria-expanded', 'true');
}
function closeGenreMenu() {
  genreMenu.hidden = true;
  genreTrigger.classList.remove('is-open');
  genreTrigger.setAttribute('aria-expanded', 'false');
}
genreTrigger.addEventListener('click', e => {
  e.stopPropagation();
  if (genreMenu.hidden) openGenreMenu(); else closeGenreMenu();
});

onLangChange(() => renderGenres());
document.addEventListener('click', e => {
  if (!genreMenu.hidden && !$('#genreDropdown').contains(e.target)) closeGenreMenu();
});

const winMinBtn = $('#winMin');
const winMaxBtn = $('#winMax');
const winCloseBtn = $('#winClose');
const tauriWin = window.__TAURI__?.window?.getCurrentWindow?.();
if (tauriWin) {
  winMinBtn?.addEventListener('click', () => tauriWin.minimize().catch(() => {}));
  winMaxBtn?.addEventListener('click', () => tauriWin.toggleMaximize().catch(() => {}));
  winCloseBtn?.addEventListener('click', () => tauriWin.close().catch(() => {}));
} else {
  winMinBtn?.parentElement?.setAttribute('hidden', '');
}

const searchInput = $('#search');
let searchTimer;
searchInput.addEventListener('input', () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    const q = searchInput.value.trim();
    if (q === state.query) return;
    state.query = q;
    if (state.section === 'library') {
      setLibraryMode(false);
      $('.tab[data-section="movie"]')?.classList.add('is-active');
      $('.tab[data-section="movie"]')?.setAttribute('aria-selected', 'true');
      state.section = 'movie';
    }
    if (q && state.selectedGenre != null) {
      state.selectedGenre = null;
      renderGenres();
    }
    refreshTrending();
    clearGrid();
    loadMore();
  }, 220);
});

document.addEventListener('keydown', e => {
  if (e.key === '/' &&
      document.activeElement !== searchInput &&
      detailsModal.hidden && settingsModal.hidden &&
      !FORM_TAGS.includes(document.activeElement?.tagName)) {
    e.preventDefault();
    searchInput.focus();
    return;
  }
  if (e.key === 'Escape') {
    if (isPlayerOpen()) closePlayer();
    else if (!detailsModal.hidden) closeModal('#detailsModal');
    else if (!settingsModal.hidden) closeModal('#settingsModal');
  }
});
