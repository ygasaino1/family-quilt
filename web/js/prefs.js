// Per-viewer conveniences kept in the browser; the page works when storage is unavailable.
const defaults = { lang: 'en', theme: 'auto', scale: '1', privacy: '0', live: '0', outlines: '1', label: 'full', layout: 'tree', engine: 'own', hl: 'all', blood: '1', literal: '1', case: '0' };
const mem = {};
export function pref(key) {
  try {
    const v = localStorage.getItem('fq.' + key);
    if (v !== null) return v;
  } catch { /* storage blocked */ }
  return mem[key] ?? defaults[key];
}
export function setPref(key, value) {
  mem[key] = String(value);
  try {
    localStorage.setItem('fq.' + key, String(value));
  } catch { /* storage blocked */ }
}
