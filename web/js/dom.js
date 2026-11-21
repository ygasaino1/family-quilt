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
