//! The fixed scoring function for the quality gate (Gate E).
//!
//! It is defined on the finished quilt only: a row order for people and a column order for
//! families, plus which person is in which family. It does not care how the order was made, so
//! it is applied identically to our layout and to the Graphviz reference.
//!
//! * **crossings**: a person's row line runs from their first to their last cell; a family's
//!   column line runs from its first to its last cell. A crossing is a row line and a column line
//!   that meet somewhere that is not a cell of that person in that family.
//! * **row_line_length**: sum over people of (last column - first column).
//! * **col_line_length**: sum over families of (last row - first row). Reported, and used as the
//!   last tie-break.
//!
//! "No worse" means: crossings and row-line length are each less than or equal to the reference.

use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Score {
    pub crossings: u64,
    pub row_line_length: u64,
    pub col_line_length: u64,
}

impl Score {
    /// Lexicographic key used to pick the best of several candidate layouts.
    pub fn key(&self) -> (u64, u64, u64) {
        (self.crossings, self.row_line_length, self.col_line_length)
    }
    /// Gate rule: not worse on the two scored numbers.
    pub fn no_worse_than(&self, reference: &Score) -> bool {
        self.crossings <= reference.crossings && self.row_line_length <= reference.row_line_length
    }
}

impl std::ops::Add for Score {
    type Output = Score;
    fn add(self, o: Score) -> Score {
        Score { crossings: self.crossings + o.crossings, row_line_length: self.row_line_length + o.row_line_length, col_line_length: self.col_line_length + o.col_line_length }
    }
}

impl fmt::Display for Score {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "crossings {}, row-line length {}, column-line length {}", self.crossings, self.row_line_length, self.col_line_length)
    }
}

struct Bit(Vec<i32>);
impl Bit {
    fn new(n: usize) -> Bit {
        Bit(vec![0; n + 2])
    }
    fn add(&mut self, i: usize, d: i32) {
        let mut i = i + 1;
        while i < self.0.len() {
            self.0[i] += d;
            i += i & i.wrapping_neg();
        }
    }
    /// Sum of positions 0..=i
    fn prefix(&self, i: usize) -> i64 {
        let mut i = i + 1;
        let mut s = 0i64;
        while i > 0 {
            s += self.0[i] as i64;
            i -= i & i.wrapping_neg();
        }
        s
    }
}

/// `row_pos[p]` and `col_pos[f]` are positions (0..n) in the final row / column order;
/// `cells` lists every (person, family) cell. O((cells) log n).
pub fn quilt_score(row_pos: &[u32], col_pos: &[u32], cells: &[(u32, u32)]) -> Score {
    let (np, nf) = (row_pos.len(), col_pos.len());
    let (mut cmin, mut cmax) = (vec![u32::MAX; np], vec![0u32; np]);
    let (mut rmin, mut rmax) = (vec![u32::MAX; nf], vec![0u32; nf]);
    for &(p, f) in cells {
        let (r, c) = (row_pos[p as usize], col_pos[f as usize]);
        cmin[p as usize] = cmin[p as usize].min(c);
        cmax[p as usize] = cmax[p as usize].max(c);
        rmin[f as usize] = rmin[f as usize].min(r);
        rmax[f as usize] = rmax[f as usize].max(r);
    }
    let mut s = Score::default();
    for p in 0..np {
        if cmin[p] != u32::MAX {
            s.row_line_length += (cmax[p] - cmin[p]) as u64;
        }
    }
    for f in 0..nf {
        if rmin[f] != u32::MAX {
            s.col_line_length += (rmax[f] - rmin[f]) as u64;
        }
    }
    // Sweep over columns. A row line is "active" at column c when cmin < c < cmax.
    let mut col_family = vec![u32::MAX; nf];
    for f in 0..nf {
        col_family[col_pos[f] as usize] = f as u32;
    }
    let mut ins: Vec<(u32, u32)> = (0..np).filter(|&p| cmin[p] != u32::MAX && cmax[p] > cmin[p]).map(|p| (cmin[p], row_pos[p])).collect();
    let mut rem: Vec<(u32, u32)> = (0..np).filter(|&p| cmin[p] != u32::MAX && cmax[p] > cmin[p]).map(|p| (cmax[p], row_pos[p])).collect();
    ins.sort_unstable();
    rem.sort_unstable();
    let mut bit = Bit::new(np);
    let (mut i, mut j) = (0, 0);
    let mut total: i64 = 0;
    for c in 0..nf as u32 {
        while j < rem.len() && rem[j].0 == c {
            bit.add(rem[j].1 as usize, -1);
            j += 1;
        }
        let f = col_family[c as usize];
        if f != u32::MAX && rmin[f as usize] != u32::MAX && rmax[f as usize] > rmin[f as usize] + 1 {
            let (lo, hi) = (rmin[f as usize] as usize, rmax[f as usize] as usize);
            total += bit.prefix(hi - 1) - bit.prefix(lo);
        }
        while i < ins.len() && ins[i].0 == c {
            bit.add(ins[i].1 as usize, 1);
            i += 1;
        }
    }
    // Cells that sit strictly inside both lines were counted above but are joints, not crossings.
    for &(p, f) in cells {
        let (r, c) = (row_pos[p as usize], col_pos[f as usize]);
        if cmin[p as usize] < c && c < cmax[p as usize] && rmin[f as usize] < r && r < rmax[f as usize] {
            total -= 1;
        }
    }
    s.crossings = total.max(0) as u64;
    s
}

/// Brute force version for tests: compares every row line with every column line.
pub fn quilt_score_slow(row_pos: &[u32], col_pos: &[u32], cells: &[(u32, u32)]) -> Score {
    let (np, nf) = (row_pos.len(), col_pos.len());
    let mut member = std::collections::HashSet::new();
    let (mut cmin, mut cmax) = (vec![u32::MAX; np], vec![0u32; np]);
    let (mut rmin, mut rmax) = (vec![u32::MAX; nf], vec![0u32; nf]);
    for &(p, f) in cells {
        member.insert((p, f));
        let (r, c) = (row_pos[p as usize], col_pos[f as usize]);
        cmin[p as usize] = cmin[p as usize].min(c);
        cmax[p as usize] = cmax[p as usize].max(c);
        rmin[f as usize] = rmin[f as usize].min(r);
        rmax[f as usize] = rmax[f as usize].max(r);
    }
    let mut s = Score::default();
    for p in 0..np {
        if cmin[p] == u32::MAX {
            continue;
        }
        s.row_line_length += (cmax[p] - cmin[p]) as u64;
        for f in 0..nf {
            if rmin[f] == u32::MAX || member.contains(&(p as u32, f as u32)) {
                continue;
            }
            let (r, c) = (row_pos[p], col_pos[f]);
            if cmin[p] < c && c < cmax[p] && rmin[f] < r && r < rmax[f] {
                s.crossings += 1;
            }
        }
    }
    for f in 0..nf {
        if rmin[f] != u32::MAX {
            s.col_line_length += (rmax[f] - rmin[f]) as u64;
        }
    }
    s
}
