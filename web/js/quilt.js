// The quilt data model: rows are people, columns are nuclear families.
// Built from the /api/quilt payload; the server has already ordered rows and columns.

export class Quilt {
  constructor(payload) {
    this.payload = payload;
    this.people = payload.people;
    this.cols = payload.columns;
    this.n = this.people.length;
    this.m = this.cols.length;
    this.stats = payload.stats;
    this.key = payload.key;
    this.idRow = new Map();
    this.people.forEach((p, i) => this.idRow.set(p.id, i));
    this.colIndexByKey = new Map();
    this.first = new Int32Array(this.n).fill(-1);
    this.last = new Int32Array(this.n).fill(-1);
    this.colMin = new Int32Array(this.m);
    this.colMax = new Int32Array(this.m);
    this.parentsOf = Array.from({ length: this.n }, () => []);
    this.childrenOf = Array.from({ length: this.n }, () => []);
    for (let j = 0; j < this.m; j++) {
      const col = this.cols[j];
      this.colIndexByKey.set(col.k, j);
      col.p.sort((a, b) => a - b);
      col.c.sort((a, b) => a.r - b.r);
      let lo = Infinity;
      let hi = -Infinity;
      const touch = (r) => {
        if (this.first[r] < 0) this.first[r] = j;
        this.last[r] = j;
        if (r < lo) lo = r;
        if (r > hi) hi = r;
      };
      for (const r of col.p) touch(r);
      for (const c of col.c) {
        touch(c.r);
        if (c.o) {
          for (const l of c.l) {
            this.parentsOf[c.r].push({ r: l.p, k: l.k });
            this.childrenOf[l.p].push({ r: c.r, k: l.k });
          }
        } else {
          for (const pr of col.p) {
            this.parentsOf[c.r].push({ r: pr, k: 'birth' });
            this.childrenOf[pr].push({ r: c.r, k: 'birth' });
          }
        }
      }
      this.colMin[j] = lo === Infinity ? 0 : lo;
      this.colMax[j] = hi === -Infinity ? 0 : hi;
    }
  }

  /** The cell of column j at row r: {role:'parent'} | {role:'child', cell} | null. */
  cellAt(j, r) {
    const col = this.cols[j];
    if (!col) return null;
    let lo = 0;
    let hi = col.p.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (col.p[mid] === r) return { role: 'parent' };
      if (col.p[mid] < r) lo = mid + 1;
      else hi = mid - 1;
    }
    lo = 0;
    hi = col.c.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (col.c[mid].r === r) return { role: 'child', cell: col.c[mid] };
      if (col.c[mid].r < r) lo = mid + 1;
      else hi = mid - 1;
    }
    return null;
  }

  /** Rows of every member of column j. */
  members(j) {
    const col = this.cols[j];
    return [...col.p, ...col.c.map((c) => c.r)];
  }

  /** Ancestors (dir -1) or descendants (dir +1) of a set of rows. Blood follows birth links only. */
  reach(rows, dir, blood) {
    const out = new Set();
    const queue = [...rows];
    const next = dir < 0 ? this.parentsOf : this.childrenOf;
    while (queue.length) {
      const r = queue.pop();
      for (const e of next[r]) {
        if (blood && e.k !== 'birth') continue;
        if (!out.has(e.r) && !rows.has(e.r)) {
          out.add(e.r);
          queue.push(e.r);
        }
      }
    }
    return out;
  }

  /** Surname = last word, given = the rest. (The data has one display name; see docs/decisions.md.) */
  label(i, mode) {
    const name = this.people[i].n || '';
    if (mode === 'full') return name;
    const parts = name.trim().split(/\s+/);
    if (parts.length < 2) return name;
    return mode === 'surname' ? parts[parts.length - 1] : parts.slice(0, -1).join(' ');
  }
}
