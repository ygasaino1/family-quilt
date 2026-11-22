// Canvas 2D quilt renderer and its interaction (pan, zoom, selection).
// The geometry (staircase of row blocks and column blocks, like the original GeneaQuilts) is in geometry.js.
import { t } from './i18n.js';
import { CW, RH, HH, WORLD_FONT, buildGeometry, floorIndex, blank, blend, mappedFrom } from './geometry.js';

export { CW, RH, HH };

const FIT_MARGIN = 28;
const HALF = 7.6; // half the size of a mark: marks nearly fill the crossing of a row and a column
const MIN_K = 0.002;
const MAX_K = 8;

const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const ease = (x) => (x < 0.5 ? 2 * x * x : 1 - Math.pow(-2 * x + 2, 2) / 2);

export class QuiltView extends EventTarget {
  constructor(canvas) {
    super();
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d', { alpha: false });
    this.q = null;
    this.tf = { x: 0, y: 0, k: 1 };
    this.size = { w: 100, h: 100, dpr: 1 };
    this.sel = []; // [{kind:'row'|'col', ids:[personId], colKey?, colour}]
    this.hl = 'all'; // all | none | ancestors | descendants
    this.blood = true;
    this._labelMode = 'full';
    this.geo = null; // where everything sits (geometry.js)
    this.mctx = document.createElement('canvas').getContext('2d');
    this.outlines = true;
    this.privacy = false;
    this.matches = new Set(); // rows matching the search
    this.dirty = true;
    this.anim = null; // glide between two layouts
    this.tween = null; // view tween (Fit)
    this.hover = null;
    this.colors = {};
    this.selRows = new Map();
    this.relRows = new Map();
    this.selCols = new Map();
    this.pointers = new Map();
    this.readColors();
    this.bind();
    new ResizeObserver(() => this.resize()).observe(canvas.parentElement);
    this.resize();
    const loop = () => {
      this.step();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }

  // ---------- setup ----------
  readColors() {
    const cs = getComputedStyle(document.documentElement);
    const g = (n) => cs.getPropertyValue(n).trim();
    this.colors = {
      bg: g('--quilt-bg'), cell: g('--quilt-cell'), fg: g('--quilt-fg'), line: g('--quilt-line'), muted: g('--muted'), dead: g('--quilt-dead'),
      pending: g('--pending'), head: g('--quilt-head'), match: g('--match'), focus: g('--focus'),
      sel: [1, 2, 3, 4, 5, 6, 7].map((i) => g('--sel' + i)),
    };
    this.dirty = true;
  }

  resize() {
    const p = this.canvas.parentElement;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(50, p.clientWidth);
    const h = Math.max(50, p.clientHeight);
    this.canvas.width = Math.round(w * dpr);
    this.canvas.height = Math.round(h * dpr);
    this.canvas.style.width = w + 'px';
    this.canvas.style.height = h + 'px';
    this.size = { w, h, dpr };
    this.dirty = true;
    this.emitView();
  }

  get labelMode() {
    return this._labelMode;
  }

  /** A different label changes the width of the name slots, so the geometry is rebuilt. */
  set labelMode(v) {
    if (v === this._labelMode) return;
    this._labelMode = v;
    if (this.q && this.q.n) {
      this.geo = this.makeGeo(this.q);
      this.anim = null;
      this.dirty = true;
      this.emitView();
      this.dispatchEvent(new CustomEvent('quilt'));
    }
  }

  /** The geometry being drawn: the real one, or the blend while the layout glides. */
  get g() {
    return this.anim ? this.anim.cur : this.geo;
  }

  makeGeo(q) {
    this.mctx.font = `${WORLD_FONT}px ${this.font()}`;
    return buildGeometry(q, (s) => this.mctx.measureText(s).width, this._labelMode);
  }

  setQuilt(q, { glide = false, fit = false } = {}) {
    const old = this.q;
    const oldGeo = this.geo;
    this.q = q;
    this.geo = q && q.n ? this.makeGeo(q) : null;
    if (glide && old && oldGeo && this.geo && !reduced() && old.n && q.n) {
      const from = mappedFrom(old, oldGeo, q, this.geo);
      const cur = blank(this.geo);
      blend(cur, from, this.geo, 0);
      this.anim = { from, to: this.geo, cur, t0: performance.now(), dur: 500, p: 0 };
    } else {
      this.anim = null;
    }
    this.recomputeSelection();
    if (fit) this.fit(false);
    this.dirty = true;
    this.emitView();
    this.dispatchEvent(new CustomEvent('quilt'));
  }

  // ---------- view transform ----------
  /** Show the whole quilt with a margin of FIT_MARGIN screen pixels on every side (the zoom bar sits in the bottom corner). */
  fit(animate = true) {
    const g = this.geo;
    const { w, h } = this.size;
    const bw = g ? g.width : 100;
    const bh = g ? g.height : 100;
    const m = FIT_MARGIN;
    const k = clamp(Math.min((w - 2 * m) / bw, (h - 2 * m) / bh), MIN_K, 1.5);
    const target = { k, x: (w - bw * k) / 2, y: (h - bh * k) / 2 };
    this.moveTo(target, animate);
  }

  moveTo(target, animate) {
    if (!animate || reduced()) {
      this.tf = target;
      this.tween = null;
      this.dirty = true;
      this.emitView();
      return;
    }
    this.tween = { from: { ...this.tf }, to: target, t0: performance.now(), dur: 220 };
  }

  zoomAt(factor, sx = this.size.w / 2, sy = this.size.h / 2) {
    const k = clamp(this.tf.k * factor, MIN_K, MAX_K);
    const f = k / this.tf.k;
    this.tf = { k, x: sx - (sx - this.tf.x) * f, y: sy - (sy - this.tf.y) * f };
    this.tween = null;
    this.dirty = true;
    this.emitView();
  }

  panBy(dx, dy) {
    this.tf = { ...this.tf, x: this.tf.x + dx, y: this.tf.y + dy };
    this.tween = null;
    this.dirty = true;
    this.emitView();
  }

  /** Put world point (wx, wy) at the middle of the screen. */
  centerOn(wx, wy, animate = true) {
    const { w, h } = this.size;
    this.moveTo({ k: this.tf.k, x: w / 2 - wx * this.tf.k, y: h / 2 - wy * this.tf.k }, animate);
  }

  /** Centre on a person's name. */
  centerOnRow(r, animate = true) {
    const g = this.geo;
    if (!g) return;
    this.centerOn(g.rnx[r] + Math.min(120, g.lo[r] + g.tw[r] / 2), g.ry[r] + RH / 2, animate);
  }

  /** World rectangle currently visible. */
  visible() {
    const { x, y, k } = this.tf;
    const { w, h } = this.size;
    return { x0: -x / k, y0: -y / k, x1: (w - x) / k, y1: (h - y) / k };
  }

  emitView() {
    this.dispatchEvent(new CustomEvent('view'));
  }

  // ---------- selection ----------
  /** sel: [{kind:'row', ids:[...]} | {kind:'col', colKey, ids}] */
  setSelection(list) {
    this.sel = list.map((s, i) => ({ ...s, colour: s.colour ?? i % 7 }));
    this.recomputeSelection();
    this.dirty = true;
    this.dispatchEvent(new CustomEvent('select', { detail: this.sel }));
  }

  clearSelection() {
    if (this.sel.length) this.setSelection([]);
  }

  setHighlight(mode, blood) {
    this.hl = mode;
    this.blood = blood;
    this.recomputeSelection();
    this.dirty = true;
  }

  recomputeSelection() {
    this.selRows = new Map();
    this.relRows = new Map();
    this.selCols = new Map();
    const q = this.q;
    if (!q) return;
    for (const s of this.sel) {
      const rows = new Set();
      for (const id of s.ids) {
        const r = q.idRow.get(id);
        if (r !== undefined) {
          rows.add(r);
          this.selRows.set(r, s.colour);
        }
      }
      if (s.kind === 'col') {
        const j = q.colIndexByKey.get(s.colKey);
        if (j !== undefined) this.selCols.set(j, s.colour);
      }
      if (s.kind === 'row' || s.kind === 'cell') {
        const up = this.hl === 'all' || this.hl === 'ancestors' ? q.reach(rows, -1, this.blood) : new Set();
        const down = this.hl === 'all' || this.hl === 'descendants' ? q.reach(rows, 1, this.blood) : new Set();
        for (const r of [...up, ...down]) if (!this.selRows.has(r)) this.relRows.set(r, s.colour);
      }
    }
  }

  setMatches(rows) {
    this.matches = new Set(rows);
    this.dirty = true;
  }

  // ---------- input ----------
  bind() {
    const c = this.canvas;
    c.tabIndex = 0;
    c.addEventListener('wheel', (e) => {
      e.preventDefault();
      const r = c.getBoundingClientRect();
      this.zoomAt(Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015)), e.clientX - r.left, e.clientY - r.top);
    }, { passive: false });
    c.addEventListener('pointerdown', (e) => {
      c.setPointerCapture(e.pointerId);
      c.focus({ preventScroll: true });
      this.pointers.set(e.pointerId, { x: e.clientX, y: e.clientY, sx: e.clientX, sy: e.clientY, moved: false, shift: e.shiftKey });
      if (this.pointers.size === 2) this.pinch = this.pinchState();
    });
    c.addEventListener('pointermove', (e) => {
      const p = this.pointers.get(e.pointerId);
      if (!p) return this.hoverAt(e);
      const dx = e.clientX - p.x;
      const dy = e.clientY - p.y;
      p.x = e.clientX;
      p.y = e.clientY;
      if (Math.hypot(e.clientX - p.sx, e.clientY - p.sy) > 4) p.moved = true;
      if (this.pointers.size === 2 && this.pinch) {
        const s = this.pinchState();
        const r = c.getBoundingClientRect();
        this.zoomAt(s.d / this.pinch.d, s.cx - r.left, s.cy - r.top);
        this.panBy(s.cx - this.pinch.cx, s.cy - this.pinch.cy);
        this.pinch = s;
      } else if (p.moved) {
        c.style.cursor = 'grabbing';
        this.panBy(dx, dy);
      }
    });
    const up = (e) => {
      const p = this.pointers.get(e.pointerId);
      this.pointers.delete(e.pointerId);
      this.pinch = null;
      c.style.cursor = '';
      if (p && !p.moved && e.type === 'pointerup' && this.pointers.size === 0) {
        const r = c.getBoundingClientRect();
        this.click(e.clientX - r.left, e.clientY - r.top, p.shift || e.shiftKey);
      }
    };
    c.addEventListener('pointerup', up);
    c.addEventListener('pointercancel', up);
    c.addEventListener('pointerleave', () => {
      if (this.hover) {
        this.hover = null;
        this.dispatchEvent(new CustomEvent('hover', { detail: null }));
      }
    });
    c.addEventListener('keydown', (e) => this.key(e));
  }

  pinchState() {
    const [a, b] = [...this.pointers.values()];
    return { d: Math.hypot(a.x - b.x, a.y - b.y) || 1, cx: (a.x + b.x) / 2, cy: (a.y + b.y) / 2 };
  }

  key(e) {
    const step = 60;
    const keys = {
      ArrowLeft: () => this.panBy(step, 0), ArrowRight: () => this.panBy(-step, 0),
      ArrowUp: () => this.panBy(0, step), ArrowDown: () => this.panBy(0, -step),
      '+': () => this.zoomAt(1.25), '=': () => this.zoomAt(1.25), '-': () => this.zoomAt(0.8),
      '0': () => this.fit(true), f: () => this.fit(true), Escape: () => this.clearSelection(),
    };
    if (e.ctrlKey || e.metaKey || e.altKey || !keys[e.key]) return;
    e.preventDefault();
    keys[e.key]();
  }

  /** What is under screen point (sx, sy)? During the glide, pick what is drawn, not where it is going. */
  hitTest(sx, sy) {
    const q = this.q;
    const g = this.g;
    if (!q || !g) return null;
    const { x, y, k } = this.tf;
    const wx = (sx - x) / k;
    const wy = (sy - y) / k;
    let r = -1;
    let j = -1;
    if (this.anim) {
      for (let i = 0; i < q.n; i++) if (wy >= g.ry[i] && wy < g.ry[i] + RH) { r = i; break; }
      for (let c = 0; c < q.m; c++) if (wx >= g.cx[c] && wx < g.cx[c] + CW) { j = c; break; }
    } else {
      r = floorIndex(g.ry, wy);
      if (r >= 0 && wy >= g.ry[r] + RH) r = -1;
      j = floorIndex(g.cx, wx);
      if (j >= 0 && wx >= g.cx[j] + CW) j = -1;
      if (j >= 0 && wy >= g.chy[j] && wy < g.chy[j] + HH) return { type: 'col', col: j };
    }
    if (r >= 0 && j >= 0) {
      const cell = q.cellAt(j, r);
      if (cell) return { type: 'cell', row: r, col: j, ...cell };
    }
    if (this.anim || r < 0 || this.fontSize() < 6) return null;
    if (wx >= g.rnx[r] && wx <= g.rnx[r] + g.lo[r] + g.tw[r]) return { type: 'label', row: r };
    return null;
  }

  font() {
    return 'system-ui, "Segoe UI", sans-serif';
  }

  /** Names are drawn at a fixed size in world units, so on screen they scale with the zoom. */
  fontSize() {
    return WORLD_FONT * this.tf.k;
  }

  hoverAt(e) {
    const r = this.canvas.getBoundingClientRect();
    const h = this.hitTest(e.clientX - r.left, e.clientY - r.top);
    const same = (this.hover?.row ?? this.hover?.col) === (h?.row ?? h?.col) && this.hover?.type === h?.type;
    this.canvas.style.cursor = h ? 'pointer' : '';
    if (!same) {
      this.hover = h;
      this.dispatchEvent(new CustomEvent('hover', { detail: h ? { ...h, x: e.clientX, y: e.clientY } : null }));
    }
  }

  click(sx, sy, shift) {
    const q = this.q;
    const h = this.hitTest(sx, sy);
    if (!h) {
      if (!shift) this.clearSelection();
      return;
    }
    let item;
    if (h.type === 'col') {
      item = { kind: 'col', colKey: q.cols[h.col].k, ids: q.members(h.col).map((r) => q.people[r].id) };
    } else {
      item = { kind: 'row', ids: [q.people[h.row].id] };
    }
    if (shift) {
      const used = new Set(this.sel.map((s) => s.colour));
      let colour = 0;
      while (used.has(colour) && colour < 6) colour++;
      this.setSelection([...this.sel, { ...item, colour }]);
    } else {
      this.setSelection([{ ...item, colour: 0 }]);
    }
  }

  // ---------- drawing ----------
  step() {
    const now = performance.now();
    if (this.tween) {
      const p = clamp((now - this.tween.t0) / this.tween.dur, 0, 1);
      const e = ease(p);
      const { from, to } = this.tween;
      this.tf = { k: from.k + (to.k - from.k) * e, x: from.x + (to.x - from.x) * e, y: from.y + (to.y - from.y) * e };
      if (p >= 1) this.tween = null;
      this.dirty = true;
      this.emitView();
    }
    if (this.anim) {
      const a = this.anim;
      a.p = clamp((now - a.t0) / a.dur, 0, 1);
      blend(a.cur, a.from, a.to, ease(a.p));
      if (a.p >= 1) this.anim = null;
      this.dirty = true;
    }
    if (this.dirty) {
      this.dirty = false;
      const t0 = performance.now();
      this.draw();
      this.lastDrawMs = performance.now() - t0;
    }
  }

  draw() {
    const { ctx, colors: C } = this;
    const { w, h, dpr } = this.size;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = C.bg;
    ctx.fillRect(0, 0, w, h);
    const q = this.q;
    const g = this.g;
    if (!q || !q.n || !g) return;
    const { x, y, k } = this.tf;
    const vis = this.visible();
    let r0 = 0;
    let r1 = q.n - 1;
    let c0 = 0;
    let c1 = q.m - 1;
    if (!this.anim) {
      // rows and columns are sorted by position, so the visible ones are found by bisection
      r0 = Math.max(0, floorIndex(g.ry, vis.y0 - RH));
      r1 = floorIndex(g.ry, vis.y1);
      c0 = Math.max(0, floorIndex(g.cx, vis.x0 - CW));
      c1 = floorIndex(g.cx, vis.x1);
    }
    const px = CW * k; // on-screen width of one column
    const lw = (n) => n / k; // n screen pixels, in world units
    const inX = (a, b) => b >= vis.x0 && a <= vis.x1;
    const inY = (a, b) => b >= vis.y0 && a <= vis.y1;

    ctx.setTransform(dpr * k, 0, 0, dpr * k, dpr * x, dpr * y);
    // 1. the strips: a row strip spans a person's name and marks, a column strip its header and marks
    ctx.fillStyle = C.cell;
    for (let i = r0; i <= r1; i++) if (inX(g.rx0[i], g.rx1[i])) ctx.fillRect(g.rx0[i], g.ry[i], g.rx1[i] - g.rx0[i], RH);
    for (let j = c0; j <= c1; j++) if (inY(g.cy0[j], g.cy1[j])) ctx.fillRect(g.cx[j], g.cy0[j], CW, g.cy1[j] - g.cy0[j]);

    // 2. selection bands over the strips
    const band = (colour, a, x0, y0, wd, ht) => {
      ctx.globalAlpha = a;
      ctx.fillStyle = C.sel[colour];
      ctx.fillRect(x0, y0, wd, ht);
    };
    for (let i = r0; i <= r1; i++) {
      const s = this.selRows.get(i);
      const rel = s === undefined ? this.relRows.get(i) : undefined;
      const wd = g.rx1[i] - g.rx0[i];
      if (s !== undefined) band(s, 0.34, g.rx0[i], g.ry[i], wd, RH);
      else if (rel !== undefined) band(rel, 0.16, g.rx0[i], g.ry[i], wd, RH);
      if (this.matches.has(i)) {
        ctx.globalAlpha = 1;
        ctx.strokeStyle = C.match;
        ctx.lineWidth = lw(2);
        ctx.strokeRect(g.rx0[i], g.ry[i] + 1, wd, RH - 2);
      }
    }
    for (const [j, colour] of this.selCols) if (j >= c0 && j <= c1) band(colour, 0.34, g.cx[j], g.cy0[j], CW, g.cy1[j] - g.cy0[j]);
    ctx.globalAlpha = 1;

    // 3. the grid: one line between consecutive rows and one between consecutive columns, each as long as
    //    the union of its two neighbours (this is all the original draws; the "cells" are line crossings)
    ctx.strokeStyle = C.line;
    ctx.lineWidth = Math.max(lw(1), 0.3);
    ctx.globalAlpha = px < 3 ? 0.55 : 1;
    ctx.beginPath();
    for (let i = r0; i <= r1; i++) {
      let xa = g.rx0[i];
      let xb = g.rx1[i];
      if (i > g.rrunA[i]) {
        xa = Math.min(xa, g.rx0[i - 1]);
        xb = Math.max(xb, g.rx1[i - 1]);
      }
      if (inX(xa, xb)) {
        ctx.moveTo(xa, g.ry[i]);
        ctx.lineTo(xb, g.ry[i]);
      }
      if (i === g.rrunB[i] && inX(g.rx0[i], g.rx1[i])) {
        ctx.moveTo(g.rx0[i], g.ry[i] + RH);
        ctx.lineTo(g.rx1[i], g.ry[i] + RH);
      }
    }
    for (let j = c0; j <= c1; j++) {
      let ya = g.cy0[j];
      let yb = g.cy1[j];
      if (j > g.crunA[j]) {
        ya = Math.min(ya, g.cy0[j - 1]);
        yb = Math.max(yb, g.cy1[j - 1]);
      }
      if (inY(ya, yb)) {
        ctx.moveTo(g.cx[j], ya);
        ctx.lineTo(g.cx[j], yb);
      }
      if (j === g.crunB[j] && inY(g.cy0[j], g.cy1[j])) {
        ctx.moveTo(g.cx[j] + CW, g.cy0[j]);
        ctx.lineTo(g.cx[j] + CW, g.cy1[j]);
      }
    }
    ctx.stroke();
    ctx.globalAlpha = 1;

    // 4. header cells (between the parent rows above and the child rows below), then the marks
    if (px >= 1.2) this.drawHeaderBoxes(q, g, c0, c1, vis, lw);
    if (px >= 1.2) this.drawCells(q, g, c0, c1, r0, r1, px);

    // 5. text, in screen space
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.drawHeaders(q, g, c0, c1, px);
    this.drawLabels(q, g, r0, r1);
  }

  drawHeaderBoxes(q, g, c0, c1, vis, lw) {
    const { ctx, colors: C } = this;
    ctx.fillStyle = C.cell;
    ctx.strokeStyle = C.line;
    ctx.lineWidth = Math.max(lw(1), 0.3);
    for (let j = c0; j <= c1; j++) {
      if (g.chy[j] > vis.y1 || g.chy[j] + HH < vis.y0) continue;
      ctx.fillRect(g.cx[j], g.chy[j], CW, HH);
      ctx.strokeRect(g.cx[j], g.chy[j], CW, HH);
    }
  }

  drawCells(q, g, c0, c1, r0, r1, px) {
    const { ctx, colors: C } = this;
    const tiny = px < 4;
    const male = new Path2D();
    const female = new Path2D();
    const other = new Path2D();
    const hollowM = new Path2D();
    const hollowF = new Path2D();
    const hollowO = new Path2D();
    const links = []; // marks of a link that is waiting for review
    const shape = (path, sex, mx, my, hs) => {
      if (sex === 'M') path.rect(mx - hs, my - hs, hs * 2, hs * 2);
      else if (sex === 'F') {
        path.moveTo(mx + hs, my);
        path.arc(mx, my, hs, 0, Math.PI * 2);
      } else {
        path.moveTo(mx, my - hs - 1);
        path.lineTo(mx + hs + 1, my);
        path.lineTo(mx, my + hs + 1);
        path.lineTo(mx - hs - 1, my);
        path.closePath();
      }
    };
    const pick = (sex, hollow) => (sex === 'M' ? (hollow ? hollowM : male) : sex === 'F' ? (hollow ? hollowF : female) : hollow ? hollowO : other);
    for (let j = c0; j <= c1; j++) {
      const col = q.cols[j];
      const mx = g.cx[j] + CW / 2;
      const drawOne = (r, hollow, linkPending) => {
        if (r < r0 || r > r1) return;
        const sex = q.people[r].s;
        const my = g.ry[r] + RH / 2;
        if (tiny) {
          ctx.fillStyle = linkPending ? C.pending : C.fg;
          ctx.fillRect(mx - CW * 0.35, my - RH * 0.35, CW * 0.7, RH * 0.7);
        } else if (linkPending) links.push([mx, my]);
        else shape(pick(sex, hollow), sex, mx, my, HALF);
      };
      for (const r of col.p) drawOne(r, false, false);
      for (const c of col.c) drawOne(c.r, !!c.o, !!c.pd);
    }
    if (tiny) return;
    const lw = (n) => n / this.tf.k;
    ctx.fillStyle = C.fg;
    ctx.fill(male);
    ctx.fill(female);
    ctx.fill(other);
    ctx.strokeStyle = C.fg;
    ctx.lineWidth = lw(1.8);
    ctx.stroke(hollowM);
    ctx.stroke(hollowF);
    ctx.stroke(hollowO);
    // A link waiting for review: an amber circle with a slash (where the mark will appear once approved).
    for (const [mx, my] of links) {
      const r = HALF * 0.82;
      ctx.beginPath();
      ctx.arc(mx, my, r, 0, Math.PI * 2);
      ctx.fillStyle = C.cell;
      ctx.fill();
      ctx.lineWidth = lw(2);
      ctx.strokeStyle = C.pending;
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(mx - r * 0.7, my + r * 0.7);
      ctx.lineTo(mx + r * 0.7, my - r * 0.7);
      ctx.stroke();
    }
  }

  /** The state letter of each column, in its header cell. */
  drawHeaders(q, g, c0, c1, px) {
    if (px < 9) return;
    const { ctx, colors: C } = this;
    const { x, y, k } = this.tf;
    ctx.font = `700 ${Math.min(HH * k * 0.62, 16)}px ${this.font()}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = C.fg;
    for (let j = c0; j <= c1; j++) {
      const sxm = x + (g.cx[j] + CW / 2) * k;
      const sym = y + (g.chy[j] + HH / 2) * k;
      if (sxm < -20 || sxm > this.size.w + 20 || sym < -20 || sym > this.size.h + 20) continue;
      ctx.fillText(t('state.' + q.cols[j].h), sxm, sym + 0.5);
    }
  }

  /** Names sit left-aligned in their slot, between the columns where the person is a child (left) and a parent (right). */
  drawLabels(q, g, r0, r1) {
    const fs = this.fontSize();
    if (fs < 4) return;
    const alpha = this.anim ? Math.max(0, (this.anim.p - 0.55) / 0.45) : 1; // names return once the glide settles
    if (alpha <= 0) return;
    const { ctx, colors: C } = this;
    const { x, y, k } = this.tf;
    ctx.globalAlpha = alpha;
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'left';
    ctx.font = `${fs}px ${this.font()}`;
    const greek = fs < 8.5 || this.privacy;
    ctx.lineJoin = 'round';
    const W = this.size.w;
    for (let i = r0; i <= r1; i++) {
      const p = q.people[i];
      const sx = x + (g.rnx[i] + g.lo[i]) * k;
      const sy = y + (g.ry[i] + RH / 2) * k;
      const tw = g.tw[i] * k;
      if (sy < -fs || sy > this.size.h + fs || sx > W || sx + tw < 0) continue;
      const text = q.labelText(i, this._labelMode);
      if (greek) {
        ctx.fillStyle = C.muted;
        ctx.globalAlpha = alpha * 0.35;
        ctx.fillRect(sx, sy - fs * 0.3, tw, fs * 0.6);
        ctx.globalAlpha = alpha;
        if (p.p && tw > 2) {
          ctx.strokeStyle = C.pending;
          ctx.lineWidth = 1;
          ctx.strokeRect(sx - 2, sy - fs * 0.45, tw + 4, fs * 0.9);
        }
        continue;
      }
      if (p.d) {
        // a recorded death (unknown is not "deceased")
        ctx.fillStyle = C.dead;
        ctx.fillRect(sx - 3, sy - fs * 0.62, tw + 6, fs * 1.24);
      }
      if (p.p) {
        // waiting for review: an amber frame around the name and a hatched marker before it
        const bh = Math.min(RH * k * 0.92, fs * 1.5);
        ctx.fillStyle = C.pending;
        ctx.globalAlpha = alpha * 0.14;
        ctx.beginPath();
        ctx.roundRect(sx - 5, sy - bh / 2, tw + 10, bh, 4);
        ctx.fill();
        ctx.globalAlpha = alpha;
        ctx.strokeStyle = C.pending;
        ctx.lineWidth = 1.6;
        ctx.stroke();
        const m = Math.min(RH * k * 0.7, fs * 1.1);
        this.pendingMarker(x + (g.rnx[i] + 1) * k, sy - m / 2, m);
      }
      if (this.outlines) {
        ctx.lineWidth = 3;
        ctx.strokeStyle = C.cell;
        ctx.strokeText(text, sx, sy);
      }
      ctx.fillStyle = C.fg;
      ctx.fillText(text, sx, sy);
    }
    ctx.globalAlpha = 1;
  }

  /** The small hatched amber square that marks something waiting for review. */
  pendingMarker(x, y, s) {
    const { ctx, colors: C } = this;
    ctx.save();
    ctx.beginPath();
    ctx.roundRect(x, y, s, s, 2);
    ctx.fillStyle = C.cell;
    ctx.fill();
    ctx.clip();
    ctx.strokeStyle = C.pending;
    ctx.lineWidth = 1.6;
    ctx.beginPath();
    for (let d = -s; d < s * 2; d += 4) {
      ctx.moveTo(x + d, y + s);
      ctx.lineTo(x + d + s, y);
    }
    ctx.stroke();
    ctx.restore();
    ctx.beginPath();
    ctx.roundRect(x, y, s, s, 2);
    ctx.strokeStyle = C.pending;
    ctx.lineWidth = 1.4;
    ctx.stroke();
  }
}
