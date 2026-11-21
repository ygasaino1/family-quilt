// Translation tables: every visible string goes through t(key). English is the source table.
// Languages other than English are added by dropping locales/<code>.json next to en.json.
// "qps" is a pseudo-language that makes untranslated or truncated text easy to spot.

let table = {};
let lang = 'en';
export const missing = new Set();

const ACCENT = { a: 'à', e: 'é', i: 'ï', o: 'ô', u: 'ü', A: 'Å', E: 'É', I: 'Ï', O: 'Ö', U: 'Û' };
function pseudo(s) {
  const body = s.replace(/\{[a-z_]+\}|[aeiouAEIOU]/g, (m) => (m.length > 1 ? m : ACCENT[m]));
  return `⟦${body}${'~'.repeat(Math.ceil(s.length * 0.3))}⟧`;
}

export async function loadLocale(code = 'en') {
  const src = code === 'qps' ? 'en' : code;
  let res = await fetch(`/locales/${src}.json`);
  if (!res.ok) res = await fetch('/locales/en.json');
  const raw = await res.json();
  table = code === 'qps' ? Object.fromEntries(Object.entries(raw).map(([k, v]) => [k, pseudo(v)])) : raw;
  lang = code;
  document.documentElement.lang = code === 'qps' ? 'en' : code;
  document.documentElement.dir = 'ltr'; // right-to-left languages will set this from their table
}

export function t(key, params) {
  let s = table[key];
  if (s === undefined) {
    missing.add(key);
    console.warn('missing translation key:', key);
    return `⟦${key}⟧`;
  }
  if (params) s = s.replace(/\{([a-z_]+)\}/g, (m, k) => (k in params ? params[k] : m));
  return s;
}

export function applyDom(root = document) {
  for (const el of root.querySelectorAll('[data-i18n]')) el.textContent = t(el.dataset.i18n);
  for (const el of root.querySelectorAll('[data-i18n-title]')) el.title = t(el.dataset.i18nTitle);
  for (const el of root.querySelectorAll('[data-i18n-aria]')) el.setAttribute('aria-label', t(el.dataset.i18nAria));
  for (const el of root.querySelectorAll('[data-i18n-placeholder]')) el.placeholder = t(el.dataset.i18nPlaceholder);
}

// ---- The one place that formats dates and numbers ----
export function fmtNumber(n) {
  return new Intl.NumberFormat(lang === 'qps' ? 'en' : lang).format(n);
}

/** "1950-03-02" | "1950-03" | "1950" (+ qualifier) -> text. */
export function fmtDate(s, qualifier) {
  if (!s) return '';
  const m = /^(\d{4})(?:-(\d\d))?(?:-(\d\d))?$/.exec(s);
  let text = s;
  if (m) {
    const l = lang === 'qps' ? 'en' : lang;
    if (m[3]) text = new Intl.DateTimeFormat(l, { dateStyle: 'medium', timeZone: 'UTC' }).format(new Date(Date.UTC(+m[1], +m[2] - 1, +m[3])));
    else if (m[2]) text = new Intl.DateTimeFormat(l, { year: 'numeric', month: 'long', timeZone: 'UTC' }).format(new Date(Date.UTC(+m[1], +m[2] - 1, 1)));
    else text = m[1];
  }
  if (qualifier && qualifier !== 'exact') return t('date.' + qualifier, { date: text });
  return text;
}

/** "3 min ago" from an ISO time, in the page language. */
export function fmtAgo(iso) {
  const then = Date.parse(iso);
  if (!then) return '';
  const secs = Math.round((then - Date.now()) / 1000);
  if (Math.abs(secs) < 45) return t('ago.now');
  const rtf = new Intl.RelativeTimeFormat(lang === 'qps' ? 'en' : lang, { numeric: 'auto' });
  const units = [['day', 86400], ['hour', 3600], ['minute', 60]];
  for (const [u, s] of units) if (Math.abs(secs) >= s) return rtf.format(Math.round(secs / s), u);
  return rtf.format(secs, 'second');
}
