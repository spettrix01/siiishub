export const TMDB_API = 'https://api.themoviedb.org/3';
export const TMDB_IMG = 'https://image.tmdb.org/t/p';

export const state = {
  section: 'movie',
  libraryTab: 'continua',
  genres: { movie: [], tv: [] },
  selectedGenre: null,
  page: 1,
  loading: false,
  done: false,
  query: '',
  items: new Map(),
  gen: 0,
  detailGen: 0,
  settings: { tmdbKey: '', addons: [], language: 'eng' },
  detail: null,
};

export function tmdbType() {
  return state.section === 'series' ? 'tv' : 'movie';
}
