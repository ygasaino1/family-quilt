// Export and import: JSON, GEDCOM, PNG and SVG of the current view, and opening a GEDCOM file.
import { t } from './i18n.js';
import { h, toast, download, confirmBox } from './dom.js';
import { CW, RH, HH, WORLD_FONT } from './geometry.js';

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

/** SVG of the part of the quilt on screen, in world units (so it scales without loss). Same drawing rules as the canvas. */
export function buildSvg(view) {
  const q = view.q;
  const g = view.geo;
  const C = view.colors;
  const v = view.visible();
  const x0 = Math.max(v.x0, -20);
  const y0 = Math.max(v.y0, -20);
  const x1 = Math.min(v.x1, g.width + 20);
  const y1 = Math.min(v.y1, g.height + 20);
  const inX = (a, b) => b >= x0 && a <= x1;
  const inY = (a, b) => b >= y0 && a <= y1;
  const hs = 7.6;
  const out = [`<svg xmlns="http://www.w3.org/2000/svg" viewBox="${x0} ${y0} ${x1 - x0} ${y1 - y0}" width="${Math.round(x1 - x0)}" height="${Math.round(y1 - y0)}" font-family="sans-serif" font-size="${WORLD_FONT}">`, `<rect x="${x0}" y="${y0}" width="${x1 - x0}" height="${y1 - y0}" fill="${C.bg}"/>`];
  const rows = [];
  for (let i = 0; i < q.n; i++) if (inY(g.ry[i], g.ry[i] + RH) && inX(g.rx0[i], g.rx1[i])) rows.push(i);
  const cols = [];
  for (let j = 0; j < q.m; j++) if (inX(g.cx[j], g.cx[j] + CW) && inY(g.cy0[j], g.cy1[j])) cols.push(j);
  for (const i of rows) out.push(`<rect x="${g.rx0[i]}" y="${g.ry[i]}" width="${g.rx1[i] - g.rx0[i]}" height="${RH}" fill="${C.cell}"/>`);
  for (const j of cols) out.push(`<rect x="${g.cx[j]}" y="${g.cy0[j]}" width="${CW}" height="${g.cy1[j] - g.cy0[j]}" fill="${C.cell}"/>`);
  // the grid: a line between consecutive rows and between consecutive columns
  const d = [];
  for (const i of rows) {
    let xa = g.rx0[i];
    let xb = g.rx1[i];
    if (i > g.rrunA[i]) {
      xa = Math.min(xa, g.rx0[i - 1]);
      xb = Math.max(xb, g.rx1[i - 1]);
    }
    d.push(`M${xa} ${g.ry[i]}H${xb}`);
    if (i === g.rrunB[i]) d.push(`M${g.rx0[i]} ${g.ry[i] + RH}H${g.rx1[i]}`);
  }
  for (const j of cols) {
    let ya = g.cy0[j];
    let yb = g.cy1[j];
    if (j > g.crunA[j]) {
      ya = Math.min(ya, g.cy0[j - 1]);
      yb = Math.max(yb, g.cy1[j - 1]);
    }
    d.push(`M${g.cx[j]} ${ya}V${yb}`);
    if (j === g.crunB[j]) d.push(`M${g.cx[j] + CW} ${g.cy0[j]}V${g.cy1[j]}`);
  }
  out.push(`<path d="${d.join('')}" fill="none" stroke="${C.line}" stroke-width="1"/>`);
  const shape = (sex, cx, cy, hollow, pending) => {
    const st = pending ? `fill="${C.cell}" stroke="${C.pending}" stroke-width="1.8"` : hollow ? `fill="none" stroke="${C.fg}" stroke-width="1.6"` : `fill="${C.fg}"`;
    if (sex === 'M') return `<rect x="${cx - hs}" y="${cy - hs}" width="${hs * 2}" height="${hs * 2}" ${st}/>`;
    if (sex === 'F') return `<circle cx="${cx}" cy="${cy}" r="${hs}" ${st}/>`;
    return `<polygon points="${cx},${cy - hs - 1} ${cx + hs + 1},${cy} ${cx},${cy + hs + 1} ${cx - hs - 1},${cy}" ${st}/>`;
  };
  const seen = new Set(rows);
  for (const j of cols) {
    const col = q.cols[j];
    const mx = g.cx[j] + CW / 2;
    out.push(`<rect x="${g.cx[j]}" y="${g.chy[j]}" width="${CW}" height="${HH}" fill="${C.cell}" stroke="${C.line}"/>`);
    out.push(`<text x="${mx}" y="${g.chy[j] + HH * 0.72}" text-anchor="middle" font-weight="700" fill="${C.fg}">${esc(t('state.' + col.h))}</text>`);
    for (const r of col.p) if (seen.has(r)) out.push(shape(q.people[r].s, mx, g.ry[r] + RH / 2, false, false));
    for (const c of col.c) if (seen.has(c.r)) out.push(shape(q.people[c.r].s, mx, g.ry[c.r] + RH / 2, !!c.o, !!c.pd));
  }
  for (const i of rows) {
    out.push(`<text x="${g.rnx[i] + g.lo[i]}" y="${g.ry[i] + RH * 0.72}" fill="${C.fg}"${view.outlines ? ` stroke="${C.cell}" stroke-width="3" paint-order="stroke"` : ''}>${esc(q.labelText(i, view.labelMode))}</text>`);
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
