//! Graphviz `dot`'s ordering phase (crossing minimisation), ported from Graphviz 16.1.2
//! (`lib/dotgen/class2.c` and `mincross.c`) for graphs without clusters, ports, labels, flat edges or
//! `ordering` attributes. Companion of [`crate::dotrank`]: given the ranks it returns the left-to-right
//! position of every node inside its rank, the way `dot` would place them.
//!
//! As in `dot`, an edge that spans several ranks becomes a chain of virtual nodes (`class2`), every
//! connected component is ordered on its own (`init_mccomp`), starting from the better of two
//! breadth-first orders (`build_ranks`) and improved by weighted-median sweeps with adjacent
//! transpositions (`mincross`), and the components are put side by side in the order `decompose` found them.
//!
//! Names of the C functions are given in comments. Where C uses pointers into the rank arrays, indices are used.

#![allow(clippy::needless_range_loop)]

use crate::dotrank::{decompose, out_edges_by_tail, Fast};

const MIN_QUIT: i32 = 8;
const MAX_ITER: i32 = 24;
const CONVERGENCE: f64 = 0.995;
const MC_SCALE: i32 = 256;

/// Rank and position (0 = leftmost) of every node, as `dot` lays them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotLayout {
    pub rank: Vec<i32>,
    pub pos: Vec<u32>,
}

/// Rank (`dot1_rank`) and order (`dot_mincross`) the graph. Nodes are numbered in the order `dot` would
/// create them (first mention), edges are in file order.
pub fn layout_nodes(n: usize, edges: &[(u32, u32)]) -> DotLayout {
    let rank = crate::dotrank::rank_nodes(n, edges);
    let pos = order_nodes(n, edges, &rank);
    DotLayout { rank, pos }
}

#[derive(Default, Clone)]
struct RankInfo {
    v: Vec<u32>,
    valid: bool,
    cache_nc: i64,
    candidate: bool,
}

struct Mincross {
    g: Fast, // fast graph of real and virtual nodes; Edge::weight holds the crossing penalty (xpenalty)
    rank: Vec<i32>,
    order: Vec<i32>,
    mval: Vec<f64>,
    save: Vec<i32>,
    mark: Vec<bool>,
    rk: Vec<RankInfo>, // ranks 0..=maxrank plus one empty rank
    minrank: usize,
    maxrank: usize,
}

/// Order the nodes of every rank. `rank` is the result of [`crate::dotrank::rank_nodes`].
pub fn order_nodes(n: usize, edges: &[(u32, u32)], rank: &[i32]) -> Vec<u32> {
    if n == 0 {
        return Vec::new();
    }
    let maxrank = rank.iter().copied().max().unwrap_or(0).max(0) as usize;
    let mut m = Mincross {
        g: Fast::new(n),
        rank: rank.to_vec(),
        order: vec![0; n],
        mval: vec![0.0; n],
        save: vec![0; n],
        mark: vec![false; n],
        rk: vec![RankInfo::default(); maxrank + 2],
        minrank: 0,
        maxrank,
    };
    m.class2(n, edges);
    let comps = decompose(&m.g, n);

    // Every component is ordered separately; the finished ranks are concatenated (merge2).
    let mut all: Vec<Vec<u32>> = vec![Vec::new(); maxrank + 2];
    for comp in &comps {
        for r in 0..m.rk.len() {
            m.rk[r].v.clear();
        }
        m.mincross(comp);
        for r in 0..=maxrank {
            all[r].extend_from_slice(&m.rk[r].v);
        }
    }
    let mut pos = vec![0u32; n];
    for r in 0..=maxrank {
        for (i, &v) in all[r].iter().enumerate() {
            if (v as usize) < n {
                pos[v as usize] = i as u32;
            }
        }
    }
    pos
}

impl Mincross {
    // ---- class2: virtual node chains ----

    fn new_vnode(&mut self, rank: i32) -> u32 {
        let v = self.g.add_node();
        self.rank.push(rank);
        self.order.push(0);
        self.mval.push(0.0);
        self.save.push(0);
        self.mark.push(false);
        v
    }

    /// `make_chain`: a chain of virtual nodes from rank(from)+1 to rank(to)-1; returns its first edge.
    fn make_chain(&mut self, from: u32, to: u32) -> u32 {
        let mut u = from;
        let mut first = None;
        for r in (self.rank[from as usize] + 1)..=self.rank[to as usize] {
            let v = if r < self.rank[to as usize] { self.new_vnode(r) } else { to };
            let e = self.g.virtual_edge(u, v, 1, 1); // weight = xpenalty = 1
            first.get_or_insert(e);
            u = v;
        }
        first.expect("make_chain: head not below tail")
    }

    /// `merge_chain`: another edge runs along the chain starting with `f`; its crossing penalty is added.
    fn merge_chain(&mut self, e_tail: u32, e_head: u32, f: u32) {
        let lastrank = self.rank[e_tail as usize].max(self.rank[e_head as usize]);
        let mut rep = f;
        loop {
            self.g.edges[rep as usize].weight += 1;
            let h = self.g.edges[rep as usize].head;
            if self.rank[h as usize] == lastrank {
                break;
            }
            rep = self.g.out[h as usize][0];
        }
    }

    fn class2(&mut self, n: usize, edges: &[(u32, u32)]) {
        let by_tail = out_edges_by_tail(n, edges);
        let mut to_virt: Vec<Option<u32>> = vec![None; edges.len()];
        for t in 0..n {
            let mut prev: Option<usize> = None;
            for &ei in &by_tail[t] {
                let ei = ei as usize;
                let (et, eh) = edges[ei];
                if to_virt[ei].is_some() {
                    prev = Some(ei);
                    continue;
                }
                // merge multi-edges
                if let Some(p) = prev {
                    if edges[p] == (et, eh) {
                        if self.rank[et as usize] == self.rank[eh as usize] {
                            continue; // merge_oneway + other_edge (a self loop)
                        }
                        let f = to_virt[p].expect("previous edge has a chain");
                        self.merge_chain(et, eh, f);
                        to_virt[ei] = Some(f);
                        continue;
                    }
                }
                // self edges
                if et == eh {
                    prev = Some(ei);
                    continue;
                }
                // forward edges
                if self.rank[eh as usize] > self.rank[et as usize] {
                    to_virt[ei] = Some(self.make_chain(et, eh));
                    prev = Some(ei);
                    continue;
                }
                // backward edges: share the chain of an opposite edge if there is one
                let mut shared = false;
                for &oi in &by_tail[eh as usize] {
                    let oi = oi as usize;
                    let (ot, oh) = edges[oi];
                    if oh != et || oh == eh {
                        continue;
                    }
                    if to_virt[oi].is_none() {
                        to_virt[oi] = Some(self.make_chain(ot, oh));
                    }
                    let f = to_virt[oi].expect("chain");
                    self.merge_chain(et, eh, f);
                    to_virt[ei] = Some(f);
                    shared = true;
                    break;
                }
                if shared {
                    continue;
                }
                to_virt[ei] = Some(self.make_chain(eh, et));
                prev = Some(ei);
            }
        }
    }

    // ---- mincross ----

    fn install_in_rank(&mut self, v: u32) {
        let r = self.rank[v as usize] as usize;
        self.order[v as usize] = self.rk[r].v.len() as i32;
        self.rk[r].v.push(v);
    }

    /// `build_ranks`: breadth-first from the sources (pass 0) or the sinks (pass 1).
    fn build_ranks(&mut self, nlist: &[u32], pass: i32) {
        for &v in nlist {
            self.mark[v as usize] = false;
        }
        for r in self.minrank..=self.maxrank {
            self.rk[r].v.clear();
        }
        let mut q: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
        for &s in nlist {
            let other = if pass == 0 { &self.g.inn[s as usize] } else { &self.g.out[s as usize] };
            if !other.is_empty() {
                continue;
            }
            if !self.mark[s as usize] {
                self.mark[s as usize] = true;
                q.push_back(s);
                while let Some(n0) = q.pop_front() {
                    self.install_in_rank(n0);
                    // enqueue_neighbors
                    if pass == 0 {
                        for i in 0..self.g.out[n0 as usize].len() {
                            let h = self.g.edges[self.g.out[n0 as usize][i] as usize].head;
                            if !self.mark[h as usize] {
                                self.mark[h as usize] = true;
                                q.push_back(h);
                            }
                        }
                    } else {
                        for i in 0..self.g.inn[n0 as usize].len() {
                            let t = self.g.edges[self.g.inn[n0 as usize][i] as usize].tail;
                            if !self.mark[t as usize] {
                                self.mark[t as usize] = true;
                                q.push_back(t);
                            }
                        }
                    }
                }
            }
        }
        for r in self.minrank..=self.maxrank {
            self.rk[r].valid = false;
        }
        if self.ncross() > 0 {
            self.transpose(false);
        }
    }

    fn ord(&self, v: u32) -> i32 {
        self.order[v as usize]
    }

    /// `rcross`: crossings between rank r and rank r + 1.
    fn rcross(&self, r: usize) -> i64 {
        let mut cross = 0i64;
        let mut max = 0usize;
        let mut count = vec![0i64; self.rk[r + 1].v.len() + 1];
        for &v in &self.rk[r].v {
            if max > 0 {
                for &e in &self.g.out[v as usize] {
                    let x = &self.g.edges[e as usize];
                    let h = self.ord(x.head) as usize;
                    for k in (h + 1)..=max {
                        cross += count[k] * x.weight as i64;
                    }
                }
            }
            for &e in &self.g.out[v as usize] {
                let x = &self.g.edges[e as usize];
                let inv = self.ord(x.head) as usize;
                if inv > max {
                    max = inv;
                }
                count[inv] += x.weight as i64;
            }
        }
        cross
    }

    fn ncross(&mut self) -> i64 {
        let mut count = 0;
        for r in self.minrank..self.maxrank {
            if self.rk[r].valid {
                count += self.rk[r].cache_nc;
            } else {
                let nc = self.rcross(r);
                self.rk[r].cache_nc = nc;
                self.rk[r].valid = true;
                count += nc;
            }
        }
        count
    }

    fn in_cross(&self, v: u32, w: u32) -> i64 {
        let mut cross = 0;
        for &e2 in &self.g.inn[w as usize] {
            let x2 = &self.g.edges[e2 as usize];
            let cnt = x2.weight as i64;
            let inv = self.ord(x2.tail);
            for &e1 in &self.g.inn[v as usize] {
                let x1 = &self.g.edges[e1 as usize];
                if self.ord(x1.tail) - inv > 0 {
                    cross += x1.weight as i64 * cnt;
                }
            }
        }
        cross
    }

    fn out_cross(&self, v: u32, w: u32) -> i64 {
        let mut cross = 0;
        for &e2 in &self.g.out[w as usize] {
            let x2 = &self.g.edges[e2 as usize];
            let cnt = x2.weight as i64;
            let inv = self.ord(x2.head);
            for &e1 in &self.g.out[v as usize] {
                let x1 = &self.g.edges[e1 as usize];
                if self.ord(x1.head) - inv > 0 {
                    cross += x1.weight as i64 * cnt;
                }
            }
        }
        cross
    }

    /// `exchange`: swap the nodes at positions `i` and `j` of rank `r`.
    fn exchange(&mut self, r: usize, i: usize, j: usize) {
        let (v, w) = (self.rk[r].v[i], self.rk[r].v[j]);
        self.order[v as usize] = j as i32;
        self.order[w as usize] = i as i32;
        self.rk[r].v.swap(i, j);
    }

    fn transpose_step(&mut self, r: usize, reverse: bool) -> i64 {
        let mut rv = 0;
        self.rk[r].candidate = false;
        let n = self.rk[r].v.len();
        for i in 0..n.saturating_sub(1) {
            let v = self.rk[r].v[i];
            let w = self.rk[r].v[i + 1];
            let mut c0 = 0;
            let mut c1 = 0;
            if r > 0 {
                c0 += self.in_cross(v, w);
                c1 += self.in_cross(w, v);
            }
            if !self.rk[r + 1].v.is_empty() {
                c0 += self.out_cross(v, w);
                c1 += self.out_cross(w, v);
            }
            if c1 < c0 || (c0 > 0 && reverse && c1 == c0) {
                self.exchange(r, i, i + 1);
                rv += c0 - c1;
                self.rk[r].valid = false;
                self.rk[r].candidate = true;
                if r > self.minrank {
                    self.rk[r - 1].valid = false;
                    self.rk[r - 1].candidate = true;
                }
                if r < self.maxrank {
                    self.rk[r + 1].valid = false;
                    self.rk[r + 1].candidate = true;
                }
            }
        }
        rv
    }

    fn transpose(&mut self, reverse: bool) {
        for r in self.minrank..=self.maxrank {
            self.rk[r].candidate = true;
        }
        loop {
            let mut delta = 0;
            for r in self.minrank..=self.maxrank {
                if self.rk[r].candidate {
                    delta += self.transpose_step(r, reverse);
                }
            }
            if delta < 1 {
                break;
            }
        }
    }

    fn save_best(&mut self) {
        for r in self.minrank..=self.maxrank {
            for &v in &self.rk[r].v {
                self.save[v as usize] = self.order[v as usize];
            }
        }
    }

    fn restore_best(&mut self) {
        for r in self.minrank..=self.maxrank {
            for &v in &self.rk[r].v {
                self.order[v as usize] = self.save[v as usize];
            }
        }
        for r in self.minrank..=self.maxrank {
            self.rk[r].valid = false;
            let order = &self.order;
            self.rk[r].v.sort_by_key(|&v| order[v as usize]);
        }
    }

    /// `medians`: the median position of each node's neighbours in rank `r1`, stored in `mval`.
    /// Returns whether some node has no neighbours at all (it keeps its place).
    fn medians(&mut self, r0: usize, r1: usize) -> bool {
        let mut hasfixed = false;
        let mut list: Vec<i32> = Vec::new();
        for i in 0..self.rk[r0].v.len() {
            let n = self.rk[r0].v[i];
            list.clear();
            if r1 > r0 {
                for &e in &self.g.out[n as usize] {
                    let x = &self.g.edges[e as usize];
                    if x.weight > 0 {
                        list.push(MC_SCALE * self.ord(x.head));
                    }
                }
            } else {
                for &e in &self.g.inn[n as usize] {
                    let x = &self.g.edges[e as usize];
                    if x.weight > 0 {
                        list.push(MC_SCALE * self.ord(x.tail));
                    }
                }
            }
            self.mval[n as usize] = match list.len() {
                0 => -1.0,
                1 => list[0] as f64,
                2 => ((list[0] + list[1]) / 2) as f64,
                len => {
                    list.sort_unstable();
                    if len % 2 == 1 {
                        list[len / 2] as f64
                    } else {
                        let rm = len / 2;
                        let lm = rm - 1;
                        let rspan = list[len - 1] - list[rm];
                        let lspan = list[lm] - list[0];
                        if lspan == rspan {
                            ((list[lm] + list[rm]) / 2) as f64
                        } else {
                            let w = list[lm] as f64 * rspan as f64 + list[rm] as f64 * lspan as f64;
                            w / (lspan + rspan) as f64
                        }
                    }
                }
            };
        }
        for i in 0..self.rk[r0].v.len() {
            let n = self.rk[r0].v[i] as usize;
            if self.g.out[n].is_empty() && self.g.inn[n].is_empty() {
                hasfixed = true; // flat_mval: no flat edges here, so the node stays fixed
            }
        }
        hasfixed
    }

    /// `reorder`: bubble nodes with a defined median to where it says, leaving fixed nodes in place.
    fn reorder(&mut self, r: usize, reverse: bool, hasfixed: bool) {
        let mut changed = false;
        let mut ep = self.rk[r].v.len();
        let n = ep as i64;
        let mut nelt = n - 1;
        while nelt >= 0 {
            let mut lp = 0usize;
            while lp < ep {
                // leftmost node that can be compared
                while lp < ep && self.mval[self.rk[r].v[lp] as usize] < 0.0 {
                    lp += 1;
                }
                if lp >= ep {
                    break;
                }
                // the next node that can be compared
                let mut rp = lp + 1;
                while rp < ep {
                    if self.mval[self.rk[r].v[rp] as usize] >= 0.0 {
                        break;
                    }
                    rp += 1;
                }
                if rp >= ep {
                    break;
                }
                let p1 = self.mval[self.rk[r].v[lp] as usize];
                let p2 = self.mval[self.rk[r].v[rp] as usize];
                if p1 > p2 || (p1 >= p2 && reverse) {
                    self.exchange(r, lp, rp);
                    changed = true;
                }
                lp = rp;
            }
            if !hasfixed && !reverse {
                ep = ep.saturating_sub(1);
            }
            nelt -= 1;
        }
        if changed {
            self.rk[r].valid = false;
            if r > 0 {
                self.rk[r - 1].valid = false;
            }
        }
    }

    fn mincross_step(&mut self, pass: i32) {
        let reverse = pass % 4 < 2;
        let (first, last, dir): (i64, i64, i64) = if pass % 2 == 0 { (self.minrank as i64 + 1, self.maxrank as i64, 1) } else { (self.maxrank as i64 - 1, self.minrank as i64, -1) };
        let mut r = first;
        while r != last + dir {
            let other = r - dir;
            let hasfixed = self.medians(r as usize, other as usize);
            self.reorder(r as usize, reverse, hasfixed);
            r += dir;
        }
        self.transpose(!reverse);
    }

    /// `mincross` on one component (`startpass` = 0). Leaves the best order in the ranks.
    fn mincross(&mut self, nlist: &[u32]) -> i64 {
        let mut cur_cross = i64::MAX;
        let mut best_cross = i64::MAX;
        for pass in 0..=2 {
            let maxthispass;
            if pass <= 1 {
                maxthispass = 4.min(MAX_ITER);
                self.build_ranks(nlist, pass);
                cur_cross = self.ncross();
                if cur_cross <= best_cross {
                    self.save_best();
                    best_cross = cur_cross;
                }
            } else {
                maxthispass = MAX_ITER;
                if cur_cross > best_cross {
                    self.restore_best();
                }
                cur_cross = best_cross;
            }
            let mut trying = 0;
            for iter in 0..maxthispass {
                let t = trying;
                trying += 1;
                if t >= MIN_QUIT {
                    break;
                }
                if cur_cross == 0 {
                    break;
                }
                self.mincross_step(iter);
                cur_cross = self.ncross();
                if cur_cross <= best_cross {
                    self.save_best();
                    if (cur_cross as f64) < CONVERGENCE * best_cross as f64 {
                        trying = 0;
                    }
                    best_cross = cur_cross;
                }
            }
            if cur_cross == 0 {
                break;
            }
        }
        if cur_cross > best_cross {
            self.restore_best();
        }
        if best_cross > 0 {
            self.transpose(false);
            best_cross = self.ncross();
        }
        best_cross
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_is_in_order() {
        let l = layout_nodes(3, &[(0, 1), (1, 2)]);
        assert_eq!(l.rank, vec![0, 1, 2]);
        assert_eq!(l.pos, vec![0, 0, 0]);
    }

    #[test]
    fn two_components_side_by_side() {
        let l = layout_nodes(4, &[(0, 1), (2, 3)]);
        assert_eq!(l.pos, vec![0, 0, 1, 1]);
    }
}
