import { $ } from './dom.js';
import { state, tmdbType, TMDB_IMG } from './state.js';
import { fetchTrending } from './api.js';
import { openDetails } from './details.js';
import { wireRailNav } from './rail-nav.js';
import { t } from './i18n.js';

const railSection = $('#trendingRail');
const railList = $('#trendingRailList');
const navPrev = $('#trendingNavPrev');
const navNext = $('#trendingNavNext');

let railGen = 0;
const trendingItems = new Map();

function shouldShow() {
  return !state.query && !!state.settings.tmdbKey;
}

function renderSkeletons(n = 10) {
  railList.innerHTML = '';
  const frag = document.createDocumentFragment();
  for (let i = 0; i < n; i++) {
    const el = document.createElement('div');
    el.className = 'trending-card is-skeleton';
    el.innerHTML = '<div class="trending-poster"></div><div class="trending-name-skel"></div>';
    frag.appendChild(el);
  }
  railList.appendChild(frag);
  updateNavVisibility();
}

function renderItems(items) {
  railList.innerHTML = '';
  trendingItems.clear();
  const frag = document.createDocumentFragment();
  let rank = 1;
  for (const it of items) {
    if (!it.poster_path) continue;
    if (rank > 20) break;
    trendingItems.set(it.id, it);
    const card = document.createElement('button');
    card.type = 'button';
    card.className = 'trending-card';
    card.dataset.id = it.id;
    card.setAttribute('role', 'listitem');
    const title = it.title || it.name || '—';
    const rating = it.vote_average && it.vote_average > 0 ? it.vote_average.toFixed(1) : '';
    const ratingHtml = rating ? `<span class="trending-rating">${rating}</span>` : '';
    card.innerHTML = `
      <div class="trending-poster">
        <span class="trending-rank">${rank}</span>
        <img loading="lazy" decoding="async" alt="" />
        ${ratingHtml}
      </div>
      <h3 class="trending-name"></h3>
    `;
    const img = card.querySelector('img');
    img.src = `${TMDB_IMG}/w342${it.poster_path}`;
    img.alt = title;
    card.querySelector('.trending-name').textContent = title;
    card.setAttribute('aria-label', t('trending.cardAria', { title, rank }));
    rank++;
    frag.appendChild(card);
  }
  railList.appendChild(frag);
  updateNavVisibility();
}

const { updateNavVisibility } = wireRailNav(railList, navPrev, navNext);

railList.addEventListener('click', (e) => {
  const card = e.target.closest('.trending-card');
  if (!card || card.classList.contains('is-skeleton')) return;
  const id = Number(card.dataset.id);
  const item = trendingItems.get(id);
  if (!item) return;
  openDetails(item);
});

export async function refreshTrending() {
  if (!shouldShow()) {
    railSection.hidden = true;
    railList.innerHTML = '';
    trendingItems.clear();
    return;
  }
  railSection.hidden = false;
  const myGen = ++railGen;
  const type = tmdbType();
  renderSkeletons(10);
  try {
    const data = await fetchTrending(type, 'day');
    if (myGen !== railGen) return;
    const list = (data.results || []).filter(it => it.poster_path);
    if (list.length === 0) {
      railSection.hidden = true;
      return;
    }
    renderItems(list);
    railList.scrollLeft = 0;
    requestAnimationFrame(updateNavVisibility);
  } catch (e) {
    if (myGen !== railGen) return;
    railSection.hidden = true;
  }
}
