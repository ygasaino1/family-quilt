// Small DOM helpers. All data goes in through textContent, never innerHTML.
import { t } from './i18n.js';

export function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v == null) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'text') el.textContent = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  return el;
}

export function toast(message, kind = 'info') {
  const box = document.getElementById('toasts');
  const el = h('div', { class: 'toast ' + kind, role: kind === 'error' ? 'alert' : 'status' }, message);
  box.append(el);
  setTimeout(() => el.remove(), kind === 'error' ? 9000 : 4500);
}

/** A modal question. Resolves true/false. */
export function confirmBox(title, body, { ok = t('common.ok'), danger = false } = {}) {
  return new Promise((resolve) => {
    const d = h('dialog', { class: 'dlg' });
    const done = (v) => {
      d.close();
      d.remove();
      resolve(v);
    };
    d.append(h('h2', {}, title), h('p', {}, body), h('div', { class: 'actions' }, h('button', { class: 'btn', onclick: () => done(false) }, t('common.cancel')), h('button', { class: 'btn ' + (danger ? 'danger' : 'primary'), onclick: () => done(true) }, ok)));
    d.addEventListener('cancel', () => done(false));
    document.body.append(d);
    d.showModal();
  });
}

export function download(name, blob) {
  const a = h('a', { href: URL.createObjectURL(blob), download: name });
  document.body.append(a);
  a.click();
  setTimeout(() => {
    URL.revokeObjectURL(a.href);
    a.remove();
  }, 1000);
}

const SEX_WORD = { M: 'male', F: 'female', X: 'other', U: 'unknown' };
/** The API gives sex as a word in people and a letter in the quilt; both end up as a translation key. */
export const sexWord = (v) => SEX_WORD[v] || String(v || 'unknown').toLowerCase();
export const sexLetter = (v) => ({ male: 'M', female: 'F', other: 'X', unknown: 'U' })[String(v).toLowerCase()] || (String(v).length === 1 ? String(v).toUpperCase() : 'U');

/** replaceChildren that tolerates null/false/nested arrays. */
export function fill(el, ...kids) {
  el.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
}

const ICONS = {
  tree: 'M12 3v4M12 7H6v4M12 7h6v4M6 11v2M18 11v2M4 17h4v4H4zM16 17h4v4h-4zM10 13h4v4h-4z',
  people: 'M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM2.5 20c0-3.6 2.9-6 6.5-6s6.5 2.4 6.5 6M16 4.3a3.5 3.5 0 0 1 0 6.4M18 14.4c2.1.7 3.5 2.5 3.5 5.6',
  review: 'M9 4h6l1 2h3v14H5V6h3zM9 13l2.2 2.2L15.5 11',
  key: 'M14.5 9.5a4 4 0 1 1-3.4 6.1L4 22v-3l-1-1 1.5-1.5 1 1 1-1-1-1 4.7-4.7A4 4 0 0 1 14.5 9.5zM16.5 11.5h.01',
  lock: 'M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 7 0v3M12 15v2',
  sliders: 'M4 7h9M17 7h3M4 17h3M11 17h9M15 4v6M9 14v6',
  search: 'M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4',
  eye: 'M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z',
  'eye-off': 'M3 3l18 18M10.6 5.1A9.6 9.6 0 0 1 12 5c6.4 0 10 7 10 7a17 17 0 0 1-3.2 4M6.5 6.6C3.8 8.4 2 12 2 12s3.6 7 10 7a9.6 9.6 0 0 0 4.2-1M9.9 9.9a3 3 0 0 0 4.2 4.2',
  bell: 'M6 16V11a6 6 0 1 1 12 0v5l2 2H4zM10 21h4',
  back: 'M15 5l-7 7 7 7',
  forward: 'M9 5l7 7-7 7',
  close: 'M6 6l12 12M18 6L6 18',
  download: 'M12 4v11M7 11l5 5 5-5M5 20h14',
  plus: 'M12 5v14M5 12h14',
  minus: 'M5 12h14',
  edit: 'M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4',
  trash: 'M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13',
  warn: 'M12 3l10 18H2zM12 10v5M12 18h.01',
  view: 'M4 6h16M7 12h10M10 18h4',
};
/** A small outline icon (24x24 grid). Decorative: the button carries the accessible name. */
export function icon(name, size = 20) {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('width', size);
  svg.setAttribute('height', size);
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.8');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  const path = document.createElementNS(ns, 'path');
  path.setAttribute('d', ICONS[name] || '');
  svg.append(path);
  return svg;
}

/** Put an icon into every <span data-icon="name"> of the page. */
export function fillIcons(root = document) {
  for (const el of root.querySelectorAll('[data-icon]')) el.replaceChildren(icon(el.dataset.icon, +el.dataset.size || 20));
}
