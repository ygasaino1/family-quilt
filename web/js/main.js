import { loadLocale, applyDom, t, fmtNumber } from './i18n.js';
import { api } from './api.js';
import { Quilt } from './quilt.js';
import { QuiltView } from './view.js';
import { Minimap } from './minimap.js';
import { h, toast, icon, fillIcons, confirmBox } from './dom.js';
import { pref, setPref } from './prefs.js';
import * as sidebar from './sidebar.js';
import * as review from './review.js';
import * as settings from './settings.js';
import * as gate from './gate.js';
import * as exporter from './export.js';

const $ = (id) => document.getElementById(id);
export const app = { view: null, status: null, reload, toast, openPerson: null, refreshAll };
window.__app = app; // for tests

// ---------- appearance ----------
function applyAppearance() {
  const theme = pref('theme');
  if (theme === 'auto') document.documentElement.removeAttribute('data-theme');
  else document.documentElement.dataset.theme = theme;
  document.documentElement.style.setProperty('--scale', pref('scale'));
  document.body.classList.toggle('privacy', pref('privacy') === '1');
  const pt = document.getElementById('privacy-toggle');
  if (pt) {
    pt.setAttribute('aria-pressed', pref('privacy'));
    pt.replaceChildren(icon(pref('privacy') === '1' ? 'eye-off' : 'eye'));
  }
  app.view?.readColors();
  app.view && (app.view.privacy = pref('privacy') === '1');
}
export { applyAppearance };

// ---------- loading the quilt ----------
let polling = false;
let cancelled = false;
let lastDataKey = null;
let firstLoad = true;

/** Set by "Run the Graphviz layout anyway" on a large file; lasts until the engine is changed. */
let forcePort = false;

function engineQuery() {
  const e = pref('engine');
  if (e === 'port' && forcePort) return '&engine=port&force=1';
  return e === 'dot' || e === 'port' ? `&engine=${e}` : '';
}

function modeQuery() {
  return `mode=${pref('layout')}&pending=1&live=${pref('live') === '1' ? 1 : 0}${engineQuery()}`;
}

async function reload({ glide = false, fit = false } = {}) {
  const p = await api('/quilt?' + modeQuery());
  applyPayload(p, { glide, fit: fit || firstLoad });
  firstLoad = false;
  if (p.job && p.job.state === 'running') pollJob();
  else hideStrip();
}

function applyPayload(p, opts) {
  const q = new Quilt(p);
  app.view.setQuilt(q, opts);
  $('empty').hidden = q.n > 0;
  updateLegend();
  sidebar.refresh();
  showEngine(p.engine);
}

/** Which engine made the order on screen, in the View menu. */
function showEngine(engine) {
  const note = $('engine-note');
  if (!note) return;
  const want = pref('engine');
  const big = $('engine-force');
  if (big) big.hidden = !(want === 'port' && typeof engine === 'string' && engine.startsWith('own (large file'));
  if (want !== 'dot' && want !== 'port') note.textContent = '';
  else if (big && !big.hidden) note.textContent = t('engine.large', { why: engine.slice(5, -1) });
  else if (engine === want) note.textContent = t(want === 'port' ? 'engine.used_port' : 'engine.used_dot');
  else if (typeof engine === 'string' && engine.startsWith('own (')) note.textContent = t('engine.fallback', { why: engine.slice(5, -1) });
  else note.textContent = t('engine.waiting');
}

function stripText(job) {
  const stage = job.stage ? t('strip.' + String(job.stage).toLowerCase()) : t('strip.starting');
  if (String(job.stage).toLowerCase() === 'polishing' && job.score_start != null && job.score_now != null) {
    return t('strip.polishing', { from: fmtNumber(job.score_start), to: fmtNumber(job.score_now) });
  }
  return stage;
}

function hideStrip() {
  $('strip').hidden = true;
}

async function pollJob() {
  if (polling) {
    // a loop that is about to stop (after "keep current") must carry on for the job started since
    cancelled = false;
    return;
  }
  polling = true;
  cancelled = false;
  let lastSnap = null;
  try {
    for (;;) {
      await new Promise((r) => setTimeout(r, 350));
      if (cancelled) break;
      const pr = await api(`/quilt/progress?mode=${pref('layout')}&pending=1${engineQuery()}`);
      const job = pr.job;
      if (!job || job.state !== 'running') {
        if (job && job.state === 'done') await reload({ glide: true });
        break;
      }
      $('strip').hidden = false;
      $('strip-text').textContent = stripText(job);
      if (pref('live') === '1' && job.snapshot_seq != null && job.snapshot_seq !== lastSnap) {
        lastSnap = job.snapshot_seq;
        const p = await api(`/quilt?${modeQuery()}&snapshot=1`);
        if (p.layout === 'progress') applyPayload(p, { glide: true });
      }
    }
  } catch (e) {
    console.warn(e);
  } finally {
    polling = false;
    hideStrip();
  }
}

async function keepCurrent() {
  cancelled = true;
  await api('/quilt/cancel', { method: 'POST' }).catch(() => {});
  hideStrip();
}

// ---------- status polling: new proposals, changed data ----------
async function pollStatus() {
  try {
    const s = await api('/status');
    const prevPending = app.status?.pending_proposals ?? 0;
    app.status = s;
    setBadges(s.pending_proposals);
    if (s.pending_proposals > prevPending && app.status && lastDataKey !== null) toast(t('toast.new_proposal'));
    if (lastDataKey !== null && s.data_key !== lastDataKey) {
      lastDataKey = s.data_key;
      await reload({ glide: true });
      review.onDataChanged();
    }
    lastDataKey = s.data_key;
  } catch (e) {
    console.warn(e);
  }
}

function setBadges(n) {
  for (const id of ['review-badge', 'bell-badge']) {
    $(id).hidden = !n;
    $(id).textContent = n;
  }
}

async function refreshAll() {
  const s = await api('/status');
  app.status = s;
  lastDataKey = s.data_key;
  setBadges(s.pending_proposals);
  await reload({ glide: true });
  review.onDataChanged();
}

// ---------- legend & tooltip ----------
function updateLegend() {
  const svg = (shape) => {
    const ns = 'http://www.w3.org/2000/svg';
    const s = document.createElementNS(ns, 'svg');
    s.setAttribute('viewBox', '0 0 16 16');
    s.setAttribute('aria-hidden', 'true');
    const fill = shape.hollow ? 'none' : 'currentColor';
    const e = document.createElementNS(ns, shape.round ? 'circle' : 'rect');
    if (shape.round) {
      e.setAttribute('cx', 8);
      e.setAttribute('cy', 8);
      e.setAttribute('r', 6);
    } else {
      e.setAttribute('x', 2);
      e.setAttribute('y', 2);
      e.setAttribute('width', 12);
      e.setAttribute('height', 12);
    }
    e.setAttribute('fill', shape.pending ? 'var(--pending)' : fill);
    e.setAttribute('fill-opacity', shape.pending ? '.3' : '1');
    e.setAttribute('stroke', shape.pending ? 'var(--pending)' : 'currentColor');
    e.setAttribute('stroke-width', '1.6');
    if (shape.pending) e.setAttribute('stroke-dasharray', '3 2');
    s.append(e);
    return s;
  };
  const L = $('legend');
  L.replaceChildren(
    h('span', {}, svg({ round: true }), t('legend.blood')),
    h('span', {}, svg({ round: true, hollow: true }), t('legend.not_blood')),
    h('span', {}, svg({ pending: true }), t('legend.waiting')),
  );
}

function initTooltip() {
  const tip = $('tip');
  app.view.addEventListener('hover', (e) => {
    const d = e.detail;
    const q = app.view.q;
    if (!d || !q) return (tip.hidden = true);
    tip.replaceChildren();
    if (d.type === 'col') {
      const col = q.cols[d.col];
      tip.append(h('b', {}, col.p.map((r) => q.people[r].n).join(' + ') || t('tip.family')), t('tip.children', { n: col.c.length }), col.h !== 'none' ? ' · ' + t('state.' + col.h + '.long') : '');
    } else {
      const p = q.people[d.row];
      tip.append(h('b', {}, p.n));
      const dates = [p.b, p.d].filter(Boolean).join(' – ');
      if (dates) tip.append(dates);
      if (d.type === 'cell' && d.role === 'child') {
        tip.append(h('div', { class: 'muted' }, d.cell.o ? t('tip.non_birth') : t('tip.birth_child')));
      }
      if (p.p) tip.append(h('div', { class: 'muted' }, t('tip.pending')));
    }
    tip.hidden = false;
    tip.style.left = Math.min(d.x + 14, innerWidth - 300) + 'px';
    tip.style.top = d.y + 14 + 'px';
  });
}

// ---------- search ----------
const search = { rows: [], idx: -1 };
function runSearch() {
  const q = app.view.q;
  const text = $('search').value;
  const cs = $('search-case').getAttribute('aria-pressed') === 'true';
  search.rows = [];
  if (q && text) {
    const needle = cs ? text : text.toLowerCase();
    // "Literal" matches the text as typed; off = every word must appear somewhere in the name.
    const literal = $('search-literal').getAttribute('aria-pressed') === 'true';
    const words = needle.split(/\s+/).filter(Boolean);
    for (let i = 0; i < q.n; i++) {
      const name = cs ? q.people[i].n : q.people[i].n.toLowerCase();
      if (literal ? name.includes(needle) : words.every((w) => name.includes(w))) search.rows.push(i);
    }
  }
  search.idx = search.rows.length ? 0 : -1;
  app.view.setMatches(search.rows);
  $('search-count').textContent = text ? (search.rows.length ? t('search.count', { i: search.idx + 1, n: search.rows.length }) : t('search.none')) : '';
}
function gotoMatch(step) {
  if (!search.rows.length) return;
  search.idx = (search.idx + step + search.rows.length) % search.rows.length;
  const r = search.rows[search.idx];
  const v = app.view;
  v.centerOnRow(r, true);
  v.setSelection([{ kind: 'row', ids: [v.q.people[r].id] }]);
  $('search-count').textContent = t('search.count', { i: search.idx + 1, n: search.rows.length });
}

// ---------- toolbar ----------
const HL = ['all', 'none', 'ancestors', 'descendants'];
function syncHl() {
  $('hl-mode').textContent = t('hl.' + pref('hl'));
  const blood = pref('blood') === '1';
  $('hl-blood').textContent = blood ? t('hl.blood') : t('hl.family');
  $('hl-blood').setAttribute('aria-pressed', String(blood));
  $('hl-blood').title = t('hl.blood_title');
  app.view.setHighlight(pref('hl'), blood);
}

function bindToolbar() {
  const toggle = (id, key, after) =>
    $(id).addEventListener('click', () => {
      const on = $(id).getAttribute('aria-pressed') !== 'true';
      $(id).setAttribute('aria-pressed', String(on));
      setPref(key, on ? '1' : '0');
      after?.(on);
    });
  $('search').addEventListener('input', runSearch);
  $('search').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') gotoMatch(e.shiftKey ? -1 : 1);
    if (e.key === 'Escape') {
      $('search').value = '';
      runSearch();
      app.view.canvas.focus();
    }
  });
  $('search-case').setAttribute('aria-pressed', String(pref('case') === '1'));
  $('search-literal').setAttribute('aria-pressed', String(pref('literal') === '1'));
  toggle('search-case', 'case', runSearch);
  toggle('search-literal', 'literal', runSearch);
  $('search-next').addEventListener('click', () => gotoMatch(1));
  $('search-prev').addEventListener('click', () => gotoMatch(-1));
  $('label-mode').value = pref('label');
  app.view.labelMode = pref('label');
  $('label-mode').addEventListener('change', () => {
    setPref('label', $('label-mode').value);
    app.view.labelMode = $('label-mode').value;
    app.view.dirty = true;
  });
  $('engine-mode').value = pref('engine');
  $('engine-mode').addEventListener('change', async () => {
    setPref('engine', $('engine-mode').value);
    forcePort = false;
    await reload({ glide: true });
  });
  $('engine-force').addEventListener('click', async () => {
    forcePort = true;
    await reload({ glide: true });
  });
  $('layout-mode').value = pref('layout');
  $('layout-mode').addEventListener('change', async () => {
    setPref('layout', $('layout-mode').value);
    await reload({ glide: true });
  });
  $('hl-mode').addEventListener('click', () => {
    setPref('hl', HL[(HL.indexOf(pref('hl')) + 1) % HL.length]);
    syncHl();
  });
  $('hl-blood').addEventListener('click', () => {
    setPref('blood', pref('blood') === '1' ? '0' : '1');
    syncHl();
  });
  $('outlines').setAttribute('aria-pressed', String(pref('outlines') === '1'));
  app.view.outlines = pref('outlines') === '1';
  $('outlines').addEventListener('click', () => {
    const on = $('outlines').getAttribute('aria-pressed') !== 'true';
    $('outlines').setAttribute('aria-pressed', String(on));
    setPref('outlines', on ? '1' : '0');
    app.view.outlines = on;
    app.view.dirty = true;
  });
  $('zoom-in').addEventListener('click', () => app.view.zoomAt(1.25));
  $('zoom-out').addEventListener('click', () => app.view.zoomAt(0.8));
  $('fit').addEventListener('click', () => app.view.fit(true));
  $('export').addEventListener('click', () => exporter.open());
  $('settings').addEventListener('click', () => settings.open());
  $('nav-agent').addEventListener('click', () => settings.open({ agent: true }));
  $('bell').addEventListener('click', () => showScreen('review'));
  const pop = $('view-pop');
  const setPop = (open) => {
    pop.hidden = !open;
    $('view-btn').setAttribute('aria-expanded', String(open));
  };
  $('view-btn').addEventListener('click', () => setPop(pop.hidden));
  document.addEventListener('pointerdown', (e) => !pop.hidden && !e.target.closest('.zoomctl') && setPop(false));
  pop.addEventListener('keydown', (e) => e.key === 'Escape' && (setPop(false), $('view-btn').focus()));
  $('privacy-toggle').addEventListener('click', () => {
    setPref('privacy', pref('privacy') === '1' ? '0' : '1');
    applyAppearance();
  });
  $('hist-back').addEventListener('click', () => stepHistory(-1));
  $('hist-fwd').addEventListener('click', () => stepHistory(1));
  $('strip-keep').addEventListener('click', keepCurrent);
  $('empty-open').addEventListener('click', () => exporter.pickGedcom());
  $('empty-add').addEventListener('click', () => sidebar.addFirstPerson());
  document.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === 'f') {
      e.preventDefault();
      showScreen('quilt');
      $('search').focus();
      $('search').select();
    }
  });
}

// ---------- back / forward through the people you looked at ----------
const hist = { ids: [], at: -1, moving: false };
function trackSelection(e) {
  if (hist.moving) return;
  const sel = e.detail;
  if (sel.length !== 1 || sel[0].kind !== 'row' || sel[0].ids.length !== 1) return;
  const id = sel[0].ids[0];
  if (hist.ids[hist.at] === id) return;
  hist.ids = hist.ids.slice(0, hist.at + 1).concat(id).slice(-50);
  hist.at = hist.ids.length - 1;
  syncHistory();
}
function syncHistory() {
  $('hist-back').disabled = hist.at <= 0;
  $('hist-fwd').disabled = hist.at >= hist.ids.length - 1;
}
function stepHistory(d) {
  const at = hist.at + d;
  if (at < 0 || at >= hist.ids.length) return;
  hist.at = at;
  hist.moving = true;
  app.openPerson(hist.ids[at]);
  hist.moving = false;
  syncHistory();
}

// ---------- screens ----------
export function showScreen(name) {
  for (const n of ['quilt', 'people', 'review']) $('nav-' + n).setAttribute('aria-pressed', String(n === name));
  $('quilt-wrap').hidden = name === 'people';
  $('screen-people').hidden = name !== 'people';
  if (name === 'review') review.show();
  else review.hide();
  if (name === 'people') loadPeople();
  if (name !== 'people') requestAnimationFrame(() => app.view.resize());
}
app.showScreen = showScreen;

async function loadPeople() {
  const q = $('people-filter').value.trim();
  const list = $('people-list');
  let people;
  if (q) people = (await api('/people?limit=200&q=' + encodeURIComponent(q))).people;
  else people = app.view.q ? app.view.q.people.slice(0, 500).map((p) => ({ id: p.id, name: p.n, birth: p.b })) : [];
  list.replaceChildren(
    ...people.map((p) =>
      h('button', { class: 'item', onclick: () => { showScreen('quilt'); app.openPerson(p.id); } }, h('b', {}, p.name), h('span', { class: 'muted' }, typeof p.birth === 'string' ? p.birth : '')),
    ),
  );
  if (!people.length) list.replaceChildren(h('p', { class: 'muted' }, t('people.none')));
}

async function main() {
  await loadLocale(new URLSearchParams(location.search).get('lang') || pref('lang'));
  document.title = t('app.name');
  applyDom();
  fillIcons();
  app.view = new QuiltView($('quilt'));
  new Minimap($('minimap'), app.view);
  applyAppearance();
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', applyAppearance);
  initTooltip();
  bindToolbar();
  app.view.addEventListener('select', trackSelection);
  syncHl();
  sidebar.init(app);
  review.init(app);
  settings.init(app);
  exporter.init(app);
  $('nav-quilt').addEventListener('click', () => showScreen('quilt'));
  $('nav-people').addEventListener('click', () => showScreen('people'));
  $('nav-review').addEventListener('click', () => showScreen('review'));
  $('people-filter').addEventListener('input', loadPeople);
  $('people-add').addEventListener('click', () => sidebar.addFirstPerson());
  window.addEventListener('tree-locked', () => gate.relock());
  const showLock = (on) => ($('nav-lock').hidden = !on);
  window.addEventListener('tree-protection', (e) => showLock(e.detail));
  $('nav-lock').addEventListener('click', async () => {
    await gate.treeAction('lock').catch(() => {});
    window.dispatchEvent(new Event('tree-locked'));
  });
  const opened = await gate.ensureOpen();
  showLock(!!(opened && opened.protected));
  // The native window asks before closing when a dialog is open (edits are saved at once; this guards a half-filled form).
  window.fqCloseRequested = async () => {
    const quit = () => window.ipc && window.ipc.postMessage('quit');
    if (document.querySelector('dialog[open]') && !(await confirmBox(t('quit.title'), t('quit.body'), { ok: t('quit.ok') }))) return;
    quit();
  };
  const [s, st] = await Promise.all([api('/status'), api('/settings').catch(() => ({}))]);
  app.status = s;
  lastDataKey = s.data_key;
  $('title').textContent = st.tree_title || t('app.name');
  app.setTitle = (x) => ($('title').textContent = x || t('app.name'));
  setBadges(s.pending_proposals);
  await reload({ fit: true });
  setInterval(pollStatus, 5000);
  document.documentElement.dataset.ready = '1';
}
main().catch((e) => {
  console.error(e);
  document.body.prepend(h('pre', { class: 'toast error' }, String(e && e.message || e)));
});
