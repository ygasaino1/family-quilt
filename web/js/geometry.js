// Where everything sits in the quilt, following the original GeneaQuilts (QuiltManager.layoutChildren).
//
// The picture is a staircase of alternating blocks along a diagonal:
//   people of generation g   (a block of ROWS, with their names in one slot)
//   families of generation g (a block of COLUMNS, with their header cells on top)
//   people of generation g+1 ...
// Each block starts right of and below the previous one. So the names of generation g sit between the
// columns where those people are children (on their left) and the columns where they are parents (on
// their right), and a column's header sits between its parent rows (above) and its child rows (below).
//
// There are no "cells": the grid is one line between consecutive rows and one between consecutive
// columns, each as long as the union of the two neighbours' extents. The squares you see are where a
// row strip and a column strip cross.

export const CW = 20; // width of a column
export const RH = 20; // height of a row
export const HH = 20; // height of the header band
export const WORLD_FONT = 15; // names are drawn at this size in world units and scale with the zoom
export const MARK_W = 20; // room before the name for the "waiting for review" marker
const NAME_PAD = 5; // space between the name slot and the cells next to it
const GAP_X = 2;

const BIG = 1e9;

/** Runs of consecutive rows that belong to one generation of one tree, and of columns likewise. */
function runs(n, keyOf) {
  const out = [];
  let a = 0;
  for (let i = 1; i <= n; i++) {
    if (i === n || keyOf(i) !== keyOf(a)) {
      out.push({ a, b: i - 1 });
      a = i;
    }
  }
  return n ? out : [];
}

/**
 * q: the Quilt model. measure(text) = width of a name in world units. labelMode as in Quilt.labelText.
 * The result holds typed arrays indexed by row (people) and by column (families).
 */
export function buildGeometry(q, measure, labelMode) {
  const n = q.n;
  const m = q.m;
  const mixed = q.payload.mode === 'layer';
  const tree = (r) => (mixed ? 0 : q.people[r].t);
  const unlinked = (r) => q.first[r] < 0;
  const rowRuns = runs(n, (i) => `${unlinked(i) ? 'u' : 'l'}|${tree(i)}|${q.people[i].g}`);
  const colRuns = runs(m, (j) => `${tree(q.cols[j].p[0] ?? 0)}|${q.cols[j].g}`);
  const rowKey = (r) => (unlinked(r.a) ? [BIG, 0, 0] : [tree(r.a), q.people[r.a].g, 0]);
  const colKey = (r) => [tree(q.cols[r.a].p[0] ?? 0), q.cols[r.a].g, 1];
  const less = (a, b) => a[0] < b[0] || (a[0] === b[0] && (a[1] < b[1] || (a[1] === b[1] && a[2] < b[2])));

  // The diagonal: people of generation g, then the families of g, then people of g + 1, ...
  const seq = [];
  let ri = 0;
  let ci = 0;
  while (ri < rowRuns.length || ci < colRuns.length) {
    const takeRows = ci >= colRuns.length || (ri < rowRuns.length && less(rowKey(rowRuns[ri]), colKey(colRuns[ci])));
    if (takeRows) seq.push({ rows: rowRuns[ri++] });
    else seq.push({ cols: colRuns[ci++] });
  }

  const tw = new Float64Array(n); // width of the name text
  const lo = new Float64Array(n); // offset of the text inside the slot
  for (let i = 0; i < n; i++) {
    tw[i] = measure(q.labelText(i, labelMode));
    lo[i] = q.people[i].p ? MARK_W : 3;
  }

  const ry = new Float64Array(n);
  const rnx = new Float64Array(n);
  const rrunA = new Int32Array(n);
  const rrunB = new Int32Array(n);
  const cx = new Float64Array(m);
  const chy = new Float64Array(m);
  const crunA = new Int32Array(m);
  const crunB = new Int32Array(m);
  let x = 0;
  let y = 0;
  for (const s of seq) {
    if (s.rows) {
      const { a, b } = s.rows;
      let w = 0;
      for (let i = a; i <= b; i++) w = Math.max(w, lo[i] + tw[i] + NAME_PAD);
      for (let i = a; i <= b; i++) {
        ry[i] = y + (i - a) * RH;
        rnx[i] = x;
        rrunA[i] = a;
        rrunB[i] = b;
      }
      x += w + GAP_X;
      y += (b - a + 1) * RH;
    } else {
      const { a, b } = s.cols;
      for (let j = a; j <= b; j++) {
        cx[j] = x + (j - a) * CW;
        chy[j] = y;
        crunA[j] = a;
        crunB[j] = b;
      }
      x += (b - a + 1) * CW + GAP_X;
      y += HH;
    }
  }

  // Extents: a row strip spans its name and all its marks; a column strip spans its header and all its marks.
  const rx0 = new Float64Array(n);
  const rx1 = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    rx0[i] = rnx[i];
    rx1[i] = rnx[i] + lo[i] + tw[i] + NAME_PAD;
  }
  const cy0 = new Float64Array(m);
  const cy1 = new Float64Array(m);
  for (let j = 0; j < m; j++) {
    cy0[j] = chy[j];
    cy1[j] = chy[j] + HH;
    const col = q.cols[j];
    const touch = (r) => {
      if (cx[j] < rx0[r]) rx0[r] = cx[j];
      if (cx[j] + CW > rx1[r]) rx1[r] = cx[j] + CW;
      if (ry[r] < cy0[j]) cy0[j] = ry[r];
      if (ry[r] + RH > cy1[j]) cy1[j] = ry[r] + RH;
    };
    for (const r of col.p) touch(r);
    for (const c of col.c) touch(c.r);
  }

  let maxX = 0;
  let maxY = 0;
  for (let i = 0; i < n; i++) {
    maxX = Math.max(maxX, rx1[i]);
    maxY = Math.max(maxY, ry[i] + RH);
  }
  for (let j = 0; j < m; j++) maxX = Math.max(maxX, cx[j] + CW);
  return { n, m, ry, rnx, rx0, rx1, rrunA, rrunB, tw, lo, cx, chy, cy0, cy1, crunA, crunB, width: maxX, height: maxY, labelMode };
}

/** Index of the last element of the ascending array `a` that is <= v, or -1. */
export function floorIndex(a, v, n = a.length) {
  let lo = 0;
  let hi = n - 1;
  let r = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (a[mid] <= v) {
      r = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return r;
}

/** A copy of `to` whose moving arrays are blended from `from` (already mapped to the same indices). */
export function blank(to) {
  return {
    ...to,
    ry: new Float64Array(to.n), rnx: new Float64Array(to.n), rx0: new Float64Array(to.n), rx1: new Float64Array(to.n),
    cx: new Float64Array(to.m), chy: new Float64Array(to.m), cy0: new Float64Array(to.m), cy1: new Float64Array(to.m),
  };
}

export function blend(out, from, to, e) {
  const mix = (o, a, b) => {
    for (let i = 0; i < o.length; i++) o[i] = a[i] + (b[i] - a[i]) * e;
  };
  mix(out.ry, from.ry, to.ry);
  mix(out.rnx, from.rnx, to.rnx);
  mix(out.rx0, from.rx0, to.rx0);
  mix(out.rx1, from.rx1, to.rx1);
  mix(out.cx, from.cx, to.cx);
  mix(out.chy, from.chy, to.chy);
  mix(out.cy0, from.cy0, to.cy0);
  mix(out.cy1, from.cy1, to.cy1);
}

/** Where each row / column of the new quilt used to be (by person id / family key); new ones start in place. */
export function mappedFrom(oldQ, oldG, q, g) {
  const f = blank(g);
  for (let i = 0; i < q.n; i++) {
    const o = oldQ.idRow.get(q.people[i].id);
    f.ry[i] = o === undefined ? g.ry[i] : oldG.ry[o];
    f.rnx[i] = o === undefined ? g.rnx[i] : oldG.rnx[o];
    f.rx0[i] = o === undefined ? g.rx0[i] : oldG.rx0[o];
    f.rx1[i] = o === undefined ? g.rx1[i] : oldG.rx1[o];
  }
  for (let j = 0; j < q.m; j++) {
    const o = oldQ.colIndexByKey.get(q.cols[j].k);
    f.cx[j] = o === undefined ? g.cx[j] : oldG.cx[o];
    f.chy[j] = o === undefined ? g.chy[j] : oldG.chy[o];
    f.cy0[j] = o === undefined ? g.cy0[j] : oldG.cy0[o];
    f.cy1[j] = o === undefined ? g.cy1[j] : oldG.cy1[o];
  }
  return f;
}
