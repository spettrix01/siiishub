import { t, intlLocale } from './i18n.js';

let _dateFmt = null;
let _dateFmtLocale = null;
function dateFmt() {
  const loc = intlLocale();
  if (!_dateFmt || _dateFmtLocale !== loc) {
    _dateFmt = new Intl.DateTimeFormat(loc, { day: '2-digit', month: 'short', year: 'numeric' });
    _dateFmtLocale = loc;
  }
  return _dateFmt;
}

export function fmtFullDate(s, fallback = '—') {
  if (!s) return fallback;
  const d = new Date(s);
  return isNaN(d) ? s : dateFmt().format(d);
}

export function getFullDate(item) {
  return fmtFullDate(item.release_date || item.first_air_date || '', '');
}

export function fmtRelative(epochMs) {
  if (!Number.isFinite(epochMs) || epochMs <= 0) return '';
  const diffMs = Date.now() - epochMs;
  if (diffMs < 0) return '';
  const min = 60_000;
  const hour = 60 * min;
  const day = 24 * hour;
  const week = 7 * day;
  if (diffMs < 5 * min) return t('time.justNow');
  if (diffMs < hour) return t('time.minAgo', { n: Math.floor(diffMs / min) });
  if (diffMs < day) {
    const h = Math.floor(diffMs / hour);
    return h === 1 ? t('time.hourAgo') : t('time.hoursAgo', { n: h });
  }
  if (diffMs < 2 * day) return t('time.yesterday');
  if (diffMs < week) return t('time.daysAgo', { n: Math.floor(diffMs / day) });
  if (diffMs < 30 * day) {
    const w = Math.floor(diffMs / week);
    return w === 1 ? t('time.weekAgo') : t('time.weeksAgo', { n: w });
  }
  return dateFmt().format(new Date(epochMs));
}

export function fmtMoney(n) {
  return `$${n.toLocaleString(intlLocale())}`;
}

export function fmtRuntime(m) {
  if (!m) return '—';
  const h = Math.floor(m / 60);
  const mm = m % 60;
  return h ? `${h}h ${String(mm).padStart(2, '0')}` : `${mm} min`;
}

export function fmtVote(v) {
  return v ? v.toFixed(1) : '—';
}

export function fmtTime(sec) {
  if (!Number.isFinite(sec) || sec < 0) return '0:00';
  const s = Math.floor(sec);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  if (h) return `${h}:${String(m).padStart(2, '0')}:${ss}`;
  return `${m}:${ss}`;
}

export function fmtBytes(b) {
  if (!b || b < 1024) return `${b | 0} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB`;
  if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
  return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function fmtSpeed(b) {
  if (!b || b < 1024) return `${b | 0} B/s`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB/s`;
  return `${(b / 1024 / 1024).toFixed(1)} MB/s`;
}

export function fmtDownloadDate(epochSec) {
  if (!Number.isFinite(epochSec) || epochSec <= 0) return '';
  const d = new Date(epochSec * 1000);
  return isNaN(d) ? '' : dateFmt().format(d);
}
