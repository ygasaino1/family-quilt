// Canvas 2D quilt renderer and its interaction (pan, zoom, selection).
// World units: one column is CW wide, one row is RH tall; the header band sits above row 0.
import { t } from './i18n.js';

export const CW = 26;
export const RH = 24;
export const HH = 24;
export const LABEL_W = 190;
const HALF = 9; // half the size of a cell shape
const MIN_K = 0.01;
const MAX_K = 8;
const STATE_LETTER = { married: 'state.married', unmarried: 'state.unmarried', divorced: 'state.divorced', ended: 'state.divorced', unknown: 'state.unknown' };

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
    this.labelMode = 'full';
    this.outlines = true;
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
      bg: g('--quilt-bg'), fg: g('--quilt-fg'), grid: g('--quilt-line'), muted: g('--muted'), dead: g('--quilt-dead'),
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

  setQuilt(q, { glide = false, fit = false } = {}) {
    const old = this.q;
    this.q = q;
    if (glide && old && !reduced() && old.n && q.n) {
      const rowFrom = new Float64Array(q.n);
      const colFrom = new Float64Array(q.m);
      for (let i = 0; i < q.n; i++) rowFrom[i] = (old.idRow.get(q.people[i].id) ?? i) * RH;
      for (let j = 0; j < q.m; j++) colFrom[j] = (old.colIndexByKey.get(q.cols[j].k) ?? j) * CW;
      this.anim = { rowFrom, colFrom, rowY: new Float64Array(q.n), colX: new Float64Array(q.m), t0: performance.now(), dur: 500 };
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
  bounds() {
    const q = this.q;
    return { x0: -LABEL_W, y0: -HH, x1: Math.max(1, q ? q.m : 1) * CW + 20, y1: Math.max(1, q ? q.n : 1) * RH + 20 };
  }

  fit(animate = true) {
    const b = this.bounds();
    const { w, h } = this.size;
    const k = clamp(Math.min(w / (b.x1 - b.x0), h / (b.y1 - b.y0)), MIN_K, 1.5);
    const target = { k, x: (w - (b.x1 - b.x0) * k) / 2 - b.x0 * k, y: (h - (b.y1 - b.y0) * k) / 2 - b.y0 * k };
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

  rowCenterY(i) {
    return this.anim ? this.anim.rowY[i] + RH / 2 : i * RH + RH / 2;
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

  /** What is under screen point (sx, sy)? */
  hitTest(sx, sy) {
    const q = this.q;
    if (!q) return null;
    const { x, y, k } = this.tf;
    const wx = (sx - x) / k;
    const wy = (sy - y) / k;
    if (this.anim) return null; // no picking during the glide
    if (wy < 0) {
      if (wy < -HH) return null;
      const j = Math.floor(wx / CW);
      return j >= 0 && j < q.m ? { type: 'col', col: j } : null;
    }
    const r = Math.floor(wy / RH);
    if (r < 0 || r >= q.n) return null;
    const j = Math.floor(wx / CW);
    if (j >= 0 && j < q.m) {
      const cell = q.cellAt(j, r);
      if (cell) return { type: 'cell', row: r, col: j, ...cell };
    }
    const first = q.first[r] < 0 ? 0 : q.first[r];
    const right = first * CW;
    const fs = this.fontSize();
    if (fs >= 6 && wx <= right && wx >= right - this.labelWidth(r, fs) / k - 6) return { type: 'label', row: r };
    return null;
  }

  labelWidth(r, fs) {
    this.ctx.font = `${fs}px ${this.font()}`;
    return this.ctx.measureText(this.q.label(r, this.labelMode)).width + 8;
  }

  font() {
    return 'system-ui, "Segoe UI", sans-serif';
  }

  fontSize() {
    return Math.min(RH * this.tf.k * 0.6, 20);
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
      // Interpolate zoom geometrically so the motion feels even.
      const k = from.k * Math.pow(to.k / from.k, e);
      this.tf = { k, x: from.x + (to.x - from.x) * e, y: from.y + (to.y - from.y) * e };
      if (p >= 1) this.tween = null;
      this.dirty = true;
      this.emitView();
    }
    if (this.anim) {
      const p = clamp((now - this.anim.t0) / this.anim.dur, 0, 1);
      const e = ease(p);
      const q = this.q;
      for (let i = 0; i < q.n; i++) this.anim.rowY[i] = this.anim.rowFrom[i] + (i * RH - this.anim.rowFrom[i]) * e;
      for (let j = 0; j < q.m; j++) this.anim.colX[j] = this.anim.colFrom[j] + (j * CW - this.anim.colFrom[j]) * e;
      if (p >= 1) this.anim = null;
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
    if (!q || !q.n) return;
    const { x, y, k } = this.tf;
    const vis = this.visible();
    const anim = this.anim;
    const ry = anim ? (i) => anim.rowY[i] : (i) => i * RH;
    const cx = anim ? (j) => anim.colX[j] : (j) => j * CW;
    const r0 = anim ? 0 : clamp(Math.floor(vis.y0 / RH) - 1, 0, q.n - 1);
    const r1 = anim ? q.n - 1 : clamp(Math.ceil(vis.y1 / RH) + 1, 0, q.n - 1);
    const c0 = anim || !q.m ? 0 : clamp(Math.floor(vis.x0 / CW) - 1, 0, q.m - 1);
    const c1 = anim ? q.m - 1 : q.m ? clamp(Math.ceil(vis.x1 / CW) + 1, 0, q.m - 1) : -1;
    const px = CW * k; // on-screen width of one column

    ctx.setTransform(dpr * k, 0, 0, dpr * k, dpr * x, dpr * y);
    const lw = (n) => n / k; // n screen pixels, in world units

    // 1. selection bands
    const band = (colour, a, x0, y0, x1, y1) => {
      ctx.globalAlpha = a;
      ctx.fillStyle = C.sel[colour];
      ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
    };
    for (let i = r0; i <= r1; i++) {
      const s = this.selRows.get(i);
      const rel = s === undefined ? this.relRows.get(i) : undefined;
      if (s !== undefined) band(s, 0.3, vis.x0, ry(i), vis.x1, ry(i) + RH);
      else if (rel !== undefined) band(rel, 0.14, vis.x0, ry(i), vis.x1, ry(i) + RH);
      if (this.matches.has(i)) {
        ctx.globalAlpha = 1;
        ctx.strokeStyle = C.match;
        ctx.lineWidth = lw(2);
        ctx.strokeRect(vis.x0, ry(i) + 1, vis.x1 - vis.x0, RH - 2);
      }
    }
    for (const [j, colour] of this.selCols) if (j >= c0 && j <= c1) band(colour, 0.3, cx(j), vis.y0, cx(j) + CW, vis.y1);
    ctx.globalAlpha = 1;

    // 2. row lines and column lines
    ctx.strokeStyle = C.grid;
    ctx.lineWidth = Math.max(lw(1), 0.6);
    ctx.beginPath();
    for (let i = r0; i <= r1; i++) {
      const a = q.first[i];
      const b = q.last[i];
      if (a < 0 || a === b) continue;
      const yy = ry(i) + RH / 2;
      ctx.moveTo(cx(a) + CW / 2, yy);
      ctx.lineTo(cx(b) + CW / 2, yy);
    }
    for (let j = c0; j <= c1; j++) {
      ctx.moveTo(cx(j) + CW / 2, ry(q.colMin[j]) + RH / 2);
      ctx.lineTo(cx(j) + CW / 2, ry(q.colMax[j]) + RH / 2);
    }
    ctx.stroke();

    // 3. cells
    if (px >= 1.2) this.drawCells(q, c0, c1, r0, r1, ry, cx, px, lw);

    // 4. header band (screen space)
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.drawHeader(q, c0, c1, cx, px);
    // 5. labels
    this.drawLabels(q, r0, r1, ry, cx);
  }

  drawCells(q, c0, c1, r0, r1, ry, cx, px, lw) {
    const { ctx, colors: C } = this;
    const tiny = px < 4;
    const male = new Path2D();
    const female = new Path2D();
    const other = new Path2D();
    const hollowM = new Path2D();
    const hollowF = new Path2D();
    const hollowO = new Path2D();
    const pend = [];
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
      const mx = cx(j) + CW / 2;
      const drawOne = (r, hollow, pending) => {
        if (r < r0 || r > r1) return;
        const sex = q.people[r].s;
        const my = ry(r) + RH / 2;
        if (tiny) {
          ctx.fillStyle = pending ? C.pending : C.fg;
          ctx.fillRect(mx - CW * 0.3, my - RH * 0.3, CW * 0.6, RH * 0.6);
        } else if (pending) pend.push([sex, mx, my, hollow]);
        else shape(pick(sex, hollow), sex, mx, my, HALF);
      };
      // rows are sorted; skip to the first visible one
      for (const r of col.p) drawOne(r, false, !!q.people[r].p);
      for (const c of col.c) drawOne(c.r, !!c.o, !!c.pd || !!q.people[c.r].p);
    }
    if (!tiny) {
      ctx.fillStyle = C.fg;
      ctx.fill(male);
      ctx.fill(female);
      ctx.fill(other);
      ctx.strokeStyle = C.fg;
      ctx.lineWidth = lw(1.6);
      ctx.stroke(hollowM);
      ctx.stroke(hollowF);
      ctx.stroke(hollowO);
      // pending: amber hatch + dashed outline
      for (const [sex, mx, my, hollow] of pend) {
        const p = new Path2D();
        shape(p, sex, mx, my, HALF);
        ctx.save();
        ctx.globalAlpha = 0.22;
        ctx.fillStyle = C.pending;
        ctx.fill(p);
        ctx.globalAlpha = 1;
        ctx.clip(p);
        ctx.strokeStyle = C.pending;
        ctx.lineWidth = lw(1.5);
        ctx.beginPath();
        for (let d = -HALF * 2; d < HALF * 2; d += 4) {
          ctx.moveTo(mx + d, my + HALF);
          ctx.lineTo(mx + d + HALF * 2, my - HALF);
        }
        ctx.stroke();
        ctx.restore();
        ctx.setLineDash([lw(3), lw(2)]);
        ctx.lineWidth = lw(1.8);
        ctx.strokeStyle = C.pending;
        ctx.stroke(p);
        ctx.setLineDash([]);
        if (px >= 14) {
          ctx.fillStyle = C.pending;
          ctx.beginPath();
          ctx.arc(mx + HALF, my - HALF, 3.2, 0, Math.PI * 2);
          ctx.fill();
        }
        void hollow;
      }
    }
  }

  drawHeader(q, c0, c1, cx, px) {
    const { ctx, colors: C } = this;
    const { x, y, k } = this.tf;
    const top = y - HH * k; // screen y of the header band's top
    const bandH = HH * k;
    if (top > this.size.h || y < 0) return;
    ctx.fillStyle = C.head;
    ctx.fillRect(0, Math.max(0, top), this.size.w, Math.min(bandH, y - Math.max(0, top)));
    if (px < 8) return;
    const fs = Math.min(bandH * 0.7, 16);
    ctx.font = `600 ${fs}px ${this.font()}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = C.fg;
    for (let j = c0; j <= c1; j++) {
      const col = q.cols[j];
      const sxm = x + (cx(j) + CW / 2) * k;
      if (sxm < -20 || sxm > this.size.w + 20) continue;
      const sel = this.selCols.get(j);
      if (sel !== undefined) {
        ctx.fillStyle = C.sel[sel];
        ctx.globalAlpha = 0.5;
        ctx.fillRect(x + cx(j) * k, top, px, bandH);
        ctx.globalAlpha = 1;
        ctx.fillStyle = C.fg;
      }
      const letter = col.h === 'none' ? '' : t(STATE_LETTER[col.h] || 'state.unknown');
      if (letter) ctx.fillText(letter, sxm, top + bandH / 2);
    }
  }

  drawLabels(q, r0, r1, ry, cx) {
    const fs = this.fontSize();
    if (fs < 4) return;
    const { ctx, colors: C } = this;
    const { x, y, k } = this.tf;
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'right';
    ctx.font = `${fs}px ${this.font()}`;
    const greek = fs < 8;
    ctx.lineJoin = 'round';
    for (let i = r0; i <= r1; i++) {
      const p = q.people[i];
      const first = q.first[i] < 0 ? 0 : q.first[i];
      const sx = x + cx(first) * k - 5;
      const sy = y + (ry_(this, i) + RH / 2) * k;
      if (sy < -fs || sy > this.size.h + fs || sx < -20) continue;
      const text = q.label(i, this.labelMode);
      if (greek) {
        ctx.fillStyle = C.muted;
        ctx.globalAlpha = 0.35;
        const wBar = Math.min(text.length * fs * 0.55, 200);
        ctx.fillRect(sx - wBar, sy - fs * 0.3, wBar, fs * 0.6);
        ctx.globalAlpha = 1;
        continue;
      }
      const dead = !p.l || !!p.d;
      if (dead) {
        const wTxt = ctx.measureText(text).width;
        ctx.fillStyle = C.dead;
        ctx.fillRect(sx - wTxt - 3, sy - fs * 0.62, wTxt + 6, fs * 1.24);
      }
      if (this.outlines) {
        ctx.lineWidth = 3;
        ctx.strokeStyle = C.bg;
        ctx.strokeText(text, sx, sy);
      }
      ctx.fillStyle = C.fg;
      ctx.fillText(text, sx, sy);
      if (p.p) {
        ctx.fillStyle = C.pending;
        ctx.beginPath();
        ctx.arc(sx - ctx.measureText(text).width - fs * 0.5, sy, Math.max(2.5, fs * 0.2), 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }
}

function ry_(v, i) {
  return v.anim ? v.anim.rowY[i] : i * RH;
}
