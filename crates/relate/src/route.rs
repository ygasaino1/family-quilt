//! Cheapest route: bidirectional Dijkstra over up / down / partner steps.
//!
//! Every edge has the same weight in both directions, so the backward search simply walks the
//! reversed edges. Costs are integers (1 step = 1000) so results are exact and deterministic.

use crate::{Dir, Path, RelGraph, Step};
use family_store::LinkKind;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

const UNIT: u32 = 1000;
const INF: u32 = u32::MAX;

fn weight(step: &Step) -> u32 {
    match step.dir {
        Dir::Partner => UNIT + UNIT / 2 + if step.ended { UNIT / 2 } else { 0 },
        // +1: the same as a birth step for all practical purposes, but birth wins exact ties.
        _ => UNIT + step.non_birth() as u32,
    }
}

impl RelGraph<'_> {
    /// Edges leaving `n` in forward direction: (to, step from n to `to`).
    fn forward(&self, n: u32, out: &mut Vec<(u32, Step)>) {
        out.clear();
        for e in &self.parents[n as usize] {
            out.push((e.other, Step { dir: Dir::Up, kind: Some(e.kind), ended: false, pending: e.pending }));
        }
        for e in &self.children[n as usize] {
            out.push((e.other, Step { dir: Dir::Down, kind: Some(e.kind), ended: false, pending: e.pending }));
        }
        for e in &self.partners[n as usize] {
            out.push((e.other, Step { dir: Dir::Partner, kind: None, ended: e.ended, pending: e.pending }));
        }
    }

    /// Edges entering `n`: (from, step from `from` to n).
    fn backward(&self, n: u32, out: &mut Vec<(u32, Step)>) {
        self.forward(n, out);
        for (_, s) in out.iter_mut() {
            *s = s.reversed();
        }
    }

    /// Cheapest path from `a` to `b`, or `None` if there is none within `max_cost`.
    pub(crate) fn route(&self, a: u32, b: u32, max_cost: f32) -> Option<Path> {
        if a == b {
            return Some(Path { nodes: vec![a], steps: vec![] });
        }
        let limit = (max_cost * UNIT as f32) as u32;
        let n = self.parents.len();
        let (mut df, mut db) = (vec![INF; n], vec![INF; n]);
        // prev_f[v] = (u, step u->v); next_b[u] = (v, step u->v) (v is one step closer to b)
        let mut prev_f: Vec<(u32, Option<Step>)> = vec![(INF, None); n];
        let mut next_b: Vec<(u32, Option<Step>)> = vec![(INF, None); n];
        let (mut hf, mut hb) = (BinaryHeap::new(), BinaryHeap::new());
        df[a as usize] = 0;
        db[b as usize] = 0;
        hf.push(Reverse((0u32, a)));
        hb.push(Reverse((0u32, b)));
        let mut mu = INF;
        let mut meet: Option<(u32, u32, Step)> = None;
        let mut buf = Vec::new();

        loop {
            let top_f = hf.peek().map(|r| r.0 .0);
            let top_b = hb.peek().map(|r| r.0 .0);
            let (Some(tf), Some(tb)) = (top_f, top_b) else { break };
            if mu != INF && tf as u64 + tb as u64 >= mu as u64 {
                break;
            }
            if tf as u64 + tb as u64 > 2 * limit as u64 + 1 {
                break;
            }
            if tf <= tb {
                let Reverse((c, u)) = hf.pop().unwrap();
                if c > df[u as usize] {
                    continue;
                }
                self.forward(u, &mut buf);
                for &(v, step) in &buf {
                    let nc = c + weight(&step);
                    if nc > limit {
                        continue;
                    }
                    if nc < df[v as usize] {
                        df[v as usize] = nc;
                        prev_f[v as usize] = (u, Some(step));
                        hf.push(Reverse((nc, v)));
                    }
                    if db[v as usize] != INF && nc + db[v as usize] < mu {
                        mu = nc + db[v as usize];
                        meet = Some((u, v, step));
                    }
                }
            } else {
                let Reverse((c, v)) = hb.pop().unwrap();
                if c > db[v as usize] {
                    continue;
                }
                self.backward(v, &mut buf);
                for &(u, step) in &buf {
                    let nc = c + weight(&step);
                    if nc > limit {
                        continue;
                    }
                    if nc < db[u as usize] {
                        db[u as usize] = nc;
                        next_b[u as usize] = (v, Some(step));
                        hb.push(Reverse((nc, u)));
                    }
                    if df[u as usize] != INF && nc + df[u as usize] < mu {
                        mu = nc + df[u as usize];
                        meet = Some((u, v, step));
                    }
                }
            }
        }
        let (u, v, mid) = meet?;
        if mu > limit {
            return None;
        }
        // a ... u
        let mut nodes = vec![u];
        let mut steps = Vec::new();
        let mut x = u;
        while x != a {
            let (p, s) = prev_f[x as usize];
            steps.push(s.expect("forward link"));
            nodes.push(p);
            x = p;
        }
        nodes.reverse();
        steps.reverse();
        // u -> v
        steps.push(mid);
        nodes.push(v);
        // v ... b
        x = v;
        while x != b {
            let (nx, s) = next_b[x as usize];
            steps.push(s.expect("backward link"));
            nodes.push(nx);
            x = nx;
        }
        debug_assert!(steps.iter().all(|s| s.kind != Some(LinkKind::Birth) || s.dir != Dir::Partner));
        Some(Path { nodes, steps })
    }
}
