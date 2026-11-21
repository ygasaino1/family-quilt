// Export and import: JSON, GEDCOM, PNG and SVG of the current view, and opening a GEDCOM file.
import { t } from './i18n.js';
import { h, toast, download, confirmBox } from './dom.js';
import { CW, RH, HH } from './view.js';

let app;
export function init(a) {
  app = a;
}

async function fetchDownload(path, fallbackName, type) {
  const res = await fetch(path);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const cd = res.headers.get('content-disposition') || '';
  const name = /filename="([^"]+)"/.exec(cd)?.[1] || fallbackName;
  download(name, new Blob([await res.arrayBuffer()], { type }));
}

export function open() {
  const dlg = document.getElementById('dlg-export');
  const run = (fn) => async () => {
    try {
      await fn();
      dlg.close();
    } catch (e) {
      toast(e.message, 'error');
    }
  };
  dlg.replaceChildren(
    h('h2', {}, t('export.title')),
    h('div', { class: 'list' },
      h('button', { class: 'item', id: 'exp-json', onclick: run(() => fetchDownload('/api/export/json', 'family-tree.json', 'application/json')) }, h('b', {}, t('export.json')), h('span', { class: 'muted' }, t('export.json_hint'))),
      h('button', { class: 'item', id: 'exp-ged', onclick: run(() => fetchDownload('/api/export/gedcom', 'family-tree.ged', 'text/plain')) }, h('b', {}, t('export.gedcom')), h('span', { class: 'muted' }, t('export.gedcom_hint'))),
      h('button', { class: 'item', id: 'exp-png', onclick: run(png) }, h('b', {}, t('export.png')), h('span', { class: 'muted' }, t('export.view_hint'))),
      h('button', { class: 'item', id: 'exp-svg', onclick: run(svg) }, h('b', {}, t('export.svg')), h('span', { class: 'muted' }, t('export.view_hint'))),
      h('button', { class: 'item', id: 'exp-open', onclick: () => { dlg.close(); pickGedcom(); } }, h('b', {}, t('export.open')), h('span', { class: 'muted' }, t('export.open_hint')))),
    h('div', { class: 'actions' }, h('button', { class: 'btn', onclick: () => dlg.close() }, t('common.close'))),
  );
  dlg.showModal();
}

function png() {
  return new Promise((resolve, reject) => {
    app.view.draw();
    app.view.canvas.toBlob((b) => {
      if (!b) return reject(new Error('PNG failed'));
      download('family-tree.png', b);
      resolve();
    }, 'image/png');
  });
}

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

/** SVG of the part of the quilt on screen, in world units (so it scales without loss). */
export function buildSvg(view) {
  const q = view.q;
  const C = view.colors;
  const v = view.visible();
  const x0 = Math.max(v.x0, -190);
  const y0 = Math.max(v.y0, -HH);
  const x1 = v.x1;
  const y1 = v.y1;
  const r0 = Math.max(0, Math.floor(y0 / RH));
  const r1 = Math.min(q.n - 1, Math.ceil(y1 / RH));
  const c0 = Math.max(0, Math.floor(x0 / CW));
  const c1 = Math.min(q.m - 1, Math.ceil(x1 / CW));
  const out = [`<svg xmlns="http://www.w3.org/2000/svg" viewBox="${x0} ${y0} ${x1 - x0} ${y1 - y0}" width="${Math.round(x1 - x0)}" height="${Math.round(y1 - y0)}" font-family="sans-serif" font-size="14">`, `<rect x="${x0}" y="${y0}" width="${x1 - x0}" height="${y1 - y0}" fill="${C.bg}"/>`];
  const line = (a, b, c, d) => out.push(`<line x1="${a}" y1="${b}" x2="${c}" y2="${d}" stroke="${C.grid}" stroke-width="1"/>`);
  for (let i = r0; i <= r1; i++) if (q.first[i] >= 0 && q.first[i] !== q.last[i]) line(q.first[i] * CW + CW / 2, i * RH + RH / 2, q.last[i] * CW + CW / 2, i * RH + RH / 2);
  for (let j = c0; j <= c1; j++) line(j * CW + CW / 2, q.colMin[j] * RH + RH / 2, j * CW + CW / 2, q.colMax[j] * RH + RH / 2);
  const shape = (sex, cx, cy, hollow, pending) => {
    const st = pending ? `fill="${C.pending}" fill-opacity=".3" stroke="${C.pending}" stroke-dasharray="3 2" stroke-width="1.8"` : hollow ? `fill="none" stroke="${C.fg}" stroke-width="1.6"` : `fill="${C.fg}"`;
    if (sex === 'M') return `<rect x="${cx - 9}" y="${cy - 9}" width="18" height="18" ${st}/>`;
    if (sex === 'F') return `<circle cx="${cx}" cy="${cy}" r="9" ${st}/>`;
    return `<polygon points="${cx},${cy - 10} ${cx + 10},${cy} ${cx},${cy + 10} ${cx - 10},${cy}" ${st}/>`;
  };
  for (let j = c0; j <= c1; j++) {
    const col = q.cols[j];
    const cx = j * CW + CW / 2;
    if (col.h !== 'none') out.push(`<text x="${cx}" y="-8" text-anchor="middle" font-weight="600" fill="${C.fg}">${esc(t('state.' + col.h))}</text>`);
    for (const r of col.p) if (r >= r0 && r <= r1) out.push(shape(q.people[r].s, cx, r * RH + RH / 2, false, !!q.people[r].p));
    for (const c of col.c) if (c.r >= r0 && c.r <= r1) out.push(shape(q.people[c.r].s, cx, c.r * RH + RH / 2, !!c.o, !!c.pd || !!q.people[c.r].p));
  }
  for (let i = r0; i <= r1; i++) {
    const first = q.first[i] < 0 ? 0 : q.first[i];
    out.push(`<text x="${first * CW - 5}" y="${i * RH + RH / 2 + 5}" text-anchor="end" fill="${C.fg}"${view.outlines ? ` stroke="${C.bg}" stroke-width="3" paint-order="stroke"` : ''}>${esc(q.label(i, view.labelMode))}</text>`);
  }
  out.push('</svg>');
  return out.join('\n');
}

function svg() {
  download('family-tree.svg', new Blob([buildSvg(app.view)], { type: 'image/svg+xml' }));
}

export function pickGedcom() {
  const input = h('input', { type: 'file', accept: '.ged,.gedcom,text/plain', id: 'ged-file', style: 'position:fixed;inset-inline-start:-999px' });
  input.addEventListener('change', async () => {
    const f = input.files[0];
    input.remove();
    if (!f) return;
    if (app.view.q?.n && !(await confirmBox(t('import.title'), t('import.add_to_existing'), { ok: t('import.go') }))) return;
    try {
      const res = await fetch('/api/import/gedcom', { method: 'POST', body: await f.arrayBuffer() });
      const body = await res.json();
      if (!res.ok) throw new Error(body.message || `HTTP ${res.status}`);
      toast(t('import.done', { people: body.people, links: body.links, warnings: body.warning_count }));
      await app.refreshAll();
      app.view.fit(false);
    } catch (e) {
      toast(e.message, 'error');
    }
  });
  document.body.append(input);
  input.click();
}
