//! Graphviz `dot`'s ranking phase, ported from Graphviz 16.1.2 (`lib/dotgen/{class1,decomp,acyclic,rank}.c`,
//! `lib/common/ns.c`, `lib/dotgen/fastgr.c`), for graphs without clusters, rank sets, edge labels or ports.
//!
//! The point of a faithful port is that ties are broken the way `dot` breaks them, so the layers (and,
//! later, the order inside a layer) come out the same as the original GeneaQuilts gets from `dot`.
//! Everything that depends on the order of nodes, of edges or of the lists in `dot`'s fast graph is
//! therefore kept: nodes in creation order, out-edges in creation order, an edge removed from a list is
//! replaced by the last one, and so on. Names of the C functions are given in comments.
//!
//! Input: `n` nodes numbered in the order `dot` would create them (first mention in the file) and the
//! edges in file order. Output: the rank of every node (0 = the top, i.e. the sources), every connected
//! component starting at 0.

#![allow(clippy::needless_range_loop)]

const NONE: u32 = u32::MAX;
const SEARCH_SIZE: usize = 30;

#[derive(Clone, Debug)]
struct Edge {
    tail: u32,
    head: u32,
    minlen: i32,
    weight: i32,
    cut: i32,
    tree_index: i32, // -1 = not a tree edge
}

/// The fast graph: nodes with ordered in/out lists over a pool of edges.
struct Fast {
    edges: Vec<Edge>,
    out: Vec<Vec<u32>>,
    inn: Vec<Vec<u32>>,
}

impl Fast {
    fn new(n: usize) -> Self {
        Fast { edges: Vec::new(), out: vec![Vec::new(); n], inn: vec![Vec::new(); n] }
    }

    /// `find_fast_edge`
    fn find(&self, u: u32, v: u32) -> Option<u32> {
        let (ul, vl) = (&self.out[u as usize], &self.inn[v as usize]);
        if ul.is_empty() || vl.is_empty() {
            return None;
        }
        if ul.len() < vl.len() {
            ul.iter().copied().find(|&e| self.edges[e as usize].head == v)
        } else {
            vl.iter().copied().find(|&e| self.edges[e as usize].tail == u)
        }
    }

    /// `virtual_edge` (`new_virtual_edge` + `fast_edge`): copies minlen and weight from `orig`.
    fn virtual_edge(&mut self, u: u32, v: u32, minlen: i32, weight: i32) -> u32 {
        let id = self.edges.len() as u32;
        self.edges.push(Edge { tail: u, head: v, minlen, weight, cut: 0, tree_index: -1 });
        self.out[u as usize].push(id);
        self.inn[v as usize].push(id);
        id
    }

    /// `merge_oneway` / `basic_merge`: fold an edge (minlen, weight) into the existing edge `rep`.
    fn merge(&mut self, rep: u32, minlen: i32, weight: i32) {
        let r = &mut self.edges[rep as usize];
        r.minlen = r.minlen.max(minlen);
        r.weight += weight;
    }

    /// `zapinlist`: remove `e` from the list and fill the hole with the last member.
    fn zap(list: &mut Vec<u32>, e: u32) {
        if let Some(i) = list.iter().position(|&x| x == e) {
            list.swap_remove(i);
        }
    }

    /// `delete_fast_edge`
    fn delete(&mut self, e: u32) {
        let (t, h) = (self.edges[e as usize].tail, self.edges[e as usize].head);
        Self::zap(&mut self.out[t as usize], e);
        Self::zap(&mut self.inn[h as usize], e);
    }

    /// `reverse_edge` (acyclic.c)
    fn reverse(&mut self, e: u32) {
        let (t, h, ml, w) = {
            let x = &self.edges[e as usize];
            (x.tail, x.head, x.minlen, x.weight)
        };
        self.delete(e);
        match self.find(h, t) {
            Some(f) => self.merge(f, ml, w),
            None => {
                self.virtual_edge(h, t, ml, w);
            }
        }
    }
}

/// Rank the nodes like `dot1_rank` does for a plain graph. See the module comment.
pub fn rank_nodes(n: usize, edges: &[(u32, u32)]) -> Vec<i32> {
    let mut g = Fast::new(n);
    // class1: one fast edge per (tail, head) pair; parallel edges are merged, self loops dropped.
    // dot walks the nodes in creation order and each node's out-edges in cgraph's order (see below).
    let mut by_tail: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (i, &(t, _)) in edges.iter().enumerate() {
        by_tail[t as usize].push(i as u32);
    }
    for t in 0..n {
        // cgraph keeps a node's out-edges ordered by the sequence number of the head, then of the edge
        // (`agedgeseqcmpf`), not by creation order alone.
        by_tail[t].sort_by_key(|&ei| (edges[ei as usize].1, ei));
        for &ei in &by_tail[t] {
            let (u, v) = edges[ei as usize];
            if u == v {
                continue;
            }
            match g.find(u, v) {
                Some(rep) => g.merge(rep, 1, 1),
                None => {
                    g.virtual_edge(u, v, 1, 1);
                }
            }
        }
    }
    let comps = decompose(&g, n);
    acyclic(&mut g, &comps);
    let mut rank = vec![0i32; n];
    for comp in &comps {
        let mut ns = NetSimplex::new(&mut g, comp, &mut rank);
        ns.run();
        ns.balance_tb();
    }
    rank
}

/// `decompose` (decomp.c): connected components, nodes listed in the order dot's iterative DFS adds them.
fn decompose(g: &Fast, n: usize) -> Vec<Vec<u32>> {
    // 0 = unseen, 1 = on the stack, 2 = done
    let mut mark = vec![0u8; n];
    let mut comps = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    for s in 0..n as u32 {
        if mark[s as usize] == 2 {
            continue;
        }
        let mut comp = Vec::new();
        mark[s as usize] = 1;
        stack.push(s);
        while let Some(v) = stack.pop() {
            if mark[v as usize] == 2 {
                continue;
            }
            mark[v as usize] = 2;
            comp.push(v);
            // lists in this order: flat_in, flat_out (none here), in, out; each from its last element to its first
            for &e in g.inn[v as usize].iter().rev() {
                let o = g.edges[e as usize].tail;
                if mark[o as usize] != 2 {
                    mark[o as usize] = 1;
                    stack.push(o);
                }
            }
            for &e in g.out[v as usize].iter().rev() {
                let o = g.edges[e as usize].head;
                if mark[o as usize] != 2 {
                    mark[o as usize] = 1;
                    stack.push(o);
                }
            }
        }
        comps.push(comp);
    }
    comps
}

/// `acyclic` (acyclic.c): depth first search, every edge to a node on the stack is reversed.
fn acyclic(g: &mut Fast, comps: &[Vec<u32>]) {
    let n = g.out.len();
    let mut mark = vec![false; n];
    let mut onstack = vec![false; n];
    for comp in comps {
        for &v in comp {
            mark[v as usize] = false;
        }
        for &s in comp {
            if mark[s as usize] {
                continue;
            }
            // iterative version of dfs(); `i` indexes the live out list, which reverse_edge edits
            mark[s as usize] = true;
            onstack[s as usize] = true;
            let mut frames: Vec<(u32, usize)> = vec![(s, 0)];
            while let Some(&mut (v, ref mut i)) = frames.last_mut() {
                if *i < g.out[v as usize].len() {
                    let e = g.out[v as usize][*i];
                    let w = g.edges[e as usize].head;
                    if onstack[w as usize] {
                        g.reverse(e);
                        // i stays: the list shifted (the last member filled the hole)
                    } else {
                        *i += 1;
                        if !mark[w as usize] {
                            mark[w as usize] = true;
                            onstack[w as usize] = true;
                            frames.push((w, 0));
                        }
                    }
                } else {
                    onstack[v as usize] = false;
                    frames.pop();
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Sub {
    rep: u32,
    size: i32,
    heap_index: usize,
    par: usize,
}

/// Network simplex on one component (`rank2` in ns.c, balance = 1).
struct NetSimplex<'a> {
    g: &'a mut Fast,
    nodes: &'a [u32],
    rank: &'a mut Vec<i32>,
    tree_edge: Vec<u32>,
    tree_out: Vec<Vec<u32>>,
    tree_in: Vec<Vec<u32>>,
    par: Vec<u32>,
    low: Vec<i32>,
    lim: Vec<i32>,
    s_i: usize,
}

const INF: usize = usize::MAX;

impl<'a> NetSimplex<'a> {
    fn new(g: &'a mut Fast, nodes: &'a [u32], rank: &'a mut Vec<i32>) -> Self {
        let n = g.out.len();
        NetSimplex { g, nodes, rank, tree_edge: Vec::new(), tree_out: vec![Vec::new(); n], tree_in: vec![Vec::new(); n], par: vec![NONE; n], low: vec![0; n], lim: vec![0; n], s_i: 0 }
    }

    fn e(&self, e: u32) -> &Edge {
        &self.g.edges[e as usize]
    }
    fn length(&self, e: u32) -> i32 {
        let x = self.e(e);
        self.rank[x.head as usize] - self.rank[x.tail as usize]
    }
    fn slack(&self, e: u32) -> i32 {
        self.length(e) - self.e(e).minlen
    }
    fn is_tree(&self, e: u32) -> bool {
        self.e(e).tree_index >= 0
    }

    /// `add_tree_edge`
    fn add_tree_edge(&mut self, e: u32) {
        self.g.edges[e as usize].tree_index = self.tree_edge.len() as i32;
        self.tree_edge.push(e);
        let (t, h) = (self.e(e).tail, self.e(e).head);
        self.tree_out[t as usize].push(e);
        self.tree_in[h as usize].push(e);
    }

    fn run(&mut self) {
        // init_graph
        for &v in self.nodes {
            for &e in &self.g.inn[v as usize] {
                self.g.edges[e as usize].cut = 0;
                self.g.edges[e as usize].tree_index = -1;
            }
        }
        let mut feasible = true;
        for &v in self.nodes {
            for &e in &self.g.inn[v as usize] {
                let x = &self.g.edges[e as usize];
                if self.rank[x.head as usize] - self.rank[x.tail as usize] < x.minlen {
                    feasible = false;
                }
            }
        }
        if !feasible {
            self.init_rank();
        }
        self.feasible_tree();
        self.debug("after feasible");
        while let Some(e) = self.leave_edge() {
            let f = self.enter_edge(e);
            self.update(e, f);
        }
        self.debug("after pivots");
    }

    fn debug(&self, tag: &str) {
        if std::env::var_os("DOTRANK_DEBUG").is_some() {
            eprintln!("== {tag}");
            for &v in self.nodes {
                eprintln!("N {v} {}", self.rank[v as usize]);
            }
        }
    }

    /// `init_rank`: longest path from the sources, nodes taken from a FIFO queue.
    fn init_rank(&mut self) {
        let mut priority: Vec<i32> = vec![0; self.rank.len()];
        for &v in self.nodes {
            priority[v as usize] = self.g.inn[v as usize].len() as i32;
        }
        let mut q: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
        for &v in self.nodes {
            if priority[v as usize] == 0 {
                q.push_back(v);
            }
        }
        while let Some(v) = q.pop_front() {
            let mut r = 0;
            for &e in &self.g.inn[v as usize] {
                let x = &self.g.edges[e as usize];
                r = r.max(self.rank[x.tail as usize] + x.minlen);
            }
            self.rank[v as usize] = r;
            for &e in &self.g.out[v as usize] {
                let h = self.g.edges[e as usize].head;
                priority[h as usize] -= 1;
                if priority[h as usize] <= 0 {
                    q.push_back(h);
                }
            }
        }
    }

    // ---- initial tight tree (feasible_tree) ----

    /// `tight_subtree_search`: grow a tree of tight edges from `v`; returns its size.
    fn tight_subtree_search(&mut self, v: u32, st: usize, owner: &mut [usize]) -> i32 {
        owner[v as usize] = st;
        // frames: (node, in_i, out_i)
        let mut size = 1;
        let mut todo: Vec<(u32, usize, usize)> = vec![(v, 0, 0)];
        while let Some(&mut (tv, ref mut in_i, ref mut out_i)) = todo.last_mut() {
            let mut pushed: Option<u32> = None;
            while *in_i < self.g.inn[tv as usize].len() {
                let e = self.g.inn[tv as usize][*in_i];
                if self.g.edges[e as usize].tree_index >= 0 {
                    *in_i += 1;
                    continue;
                }
                let t = self.g.edges[e as usize].tail;
                let sl = self.rank[self.g.edges[e as usize].head as usize] - self.rank[t as usize] - self.g.edges[e as usize].minlen;
                if owner[t as usize] == INF && sl == 0 {
                    *in_i += 1;
                    pushed = Some(e);
                    break;
                }
                *in_i += 1;
            }
            if pushed.is_none() {
                while *out_i < self.g.out[tv as usize].len() {
                    let e = self.g.out[tv as usize][*out_i];
                    if self.g.edges[e as usize].tree_index >= 0 {
                        *out_i += 1;
                        continue;
                    }
                    let h = self.g.edges[e as usize].head;
                    let sl = self.rank[h as usize] - self.rank[self.g.edges[e as usize].tail as usize] - self.g.edges[e as usize].minlen;
                    if owner[h as usize] == INF && sl == 0 {
                        *out_i += 1;
                        pushed = Some(e);
                        break;
                    }
                    *out_i += 1;
                }
            }
            match pushed {
                Some(e) => {
                    let ex = self.g.edges[e as usize].clone();
                    let other = if ex.head == tv { ex.tail } else { ex.head };
                    self.add_tree_edge(e);
                    owner[other as usize] = st;
                    size += 1;
                    todo.push((other, 0, 0));
                }
                None => {
                    todo.pop();
                }
            }
        }
        size
    }

    fn st_find(subs: &mut [Sub], owner: &[usize], n0: u32) -> usize {
        let mut s0 = owner[n0 as usize];
        while subs[s0].par != s0 {
            let p = subs[s0].par;
            if subs[p].par != p {
                subs[s0].par = subs[p].par;
            }
            s0 = subs[s0].par;
        }
        s0
    }

    fn st_union(subs: &mut [Sub], s0: usize, s1: usize) -> usize {
        let mut r0 = s0;
        while subs[r0].par != r0 {
            r0 = subs[r0].par;
        }
        let mut r1 = s1;
        while subs[r1].par != r1 {
            r1 = subs[r1].par;
        }
        if r0 == r1 {
            return r0;
        }
        let on = |s: &Sub| s.heap_index != INF;
        let r = if !on(&subs[r1]) {
            r0
        } else if !on(&subs[r0]) {
            r1
        } else if subs[r1].size < subs[r0].size {
            r0
        } else {
            r1
        };
        subs[r0].par = r;
        subs[r1].par = r;
        subs[r].size = subs[r0].size + subs[r1].size;
        r
    }

    /// `inter_tree_edge_search`: the tightest non-tree edge leaving the merged subtree of `v`.
    fn inter_tree_edge(&self, subs: &mut [Sub], owner: &[usize], v: u32) -> Option<u32> {
        struct St {
            v: u32,
            ts: usize,
            from: u32,
            out_i: usize,
            in_i: usize,
        }
        let mut best: Option<u32> = None;
        let mut todo = vec![St { v, ts: Self::st_find(subs, owner, v), from: NONE, out_i: 0, in_i: 0 }];
        while !todo.is_empty() {
            let top = todo.len() - 1;
            if todo[top].out_i == 0 && todo[top].in_i == 0 {
                if let Some(b) = best {
                    if self.slack(b) == 0 {
                        todo.pop();
                        continue;
                    }
                }
            }
            let sv = todo[top].v;
            let mut pushed = false;
            while todo[top].out_i < self.g.out[sv as usize].len() {
                let e = self.g.out[sv as usize][todo[top].out_i];
                let h = self.e(e).head;
                if self.is_tree(e) {
                    if h == todo[top].from {
                        todo[top].out_i += 1;
                        continue;
                    }
                    todo[top].out_i += 1;
                    let ts = Self::st_find(subs, owner, h);
                    todo.push(St { v: h, ts, from: sv, out_i: 0, in_i: 0 });
                    pushed = true;
                    break;
                } else {
                    if Self::st_find(subs, owner, h) != todo[top].ts && best.is_none_or(|b| self.slack(e) < self.slack(b)) {
                        best = Some(e);
                    }
                    todo[top].out_i += 1;
                }
            }
            if pushed {
                continue;
            }
            while todo[top].in_i < self.g.inn[sv as usize].len() {
                let e = self.g.inn[sv as usize][todo[top].in_i];
                let t = self.e(e).tail;
                if self.is_tree(e) {
                    if t == todo[top].from {
                        todo[top].in_i += 1;
                        continue;
                    }
                    todo[top].in_i += 1;
                    let ts = Self::st_find(subs, owner, t);
                    todo.push(St { v: t, ts, from: sv, out_i: 0, in_i: 0 });
                    pushed = true;
                    break;
                } else {
                    if Self::st_find(subs, owner, t) != todo[top].ts && best.is_none_or(|b| self.slack(e) < self.slack(b)) {
                        best = Some(e);
                    }
                    todo[top].in_i += 1;
                }
            }
            if pushed {
                continue;
            }
            todo.pop();
        }
        best
    }

    fn heapify(heap: &mut [usize], size: usize, subs: &mut [Sub], mut i: usize) {
        loop {
            let left = 2 * (i + 1) - 1;
            let right = 2 * (i + 1);
            let mut smallest = i;
            if left < size && subs[heap[left]].size < subs[heap[smallest]].size {
                smallest = left;
            }
            if right < size && subs[heap[right]].size < subs[heap[smallest]].size {
                smallest = right;
            }
            if smallest != i {
                heap.swap(i, smallest);
                subs[heap[i]].heap_index = i;
                subs[heap[smallest]].heap_index = smallest;
                i = smallest;
            } else {
                break;
            }
            if i >= size {
                break;
            }
        }
    }

    /// Add `delta` to the rank of every node of the tree that contains `v` (`tree_adjust`).
    fn tree_adjust(&mut self, v: u32, delta: i32) {
        let mut stack = vec![(v, NONE)];
        while let Some((x, from)) = stack.pop() {
            self.rank[x as usize] += delta;
            for &e in &self.tree_in[x as usize] {
                let w = self.g.edges[e as usize].tail;
                if w != from {
                    stack.push((w, x));
                }
            }
            for &e in &self.tree_out[x as usize] {
                let w = self.g.edges[e as usize].head;
                if w != from {
                    stack.push((w, x));
                }
            }
        }
    }

    fn feasible_tree(&mut self) {
        let n = self.rank.len();
        let mut owner = vec![INF; n];
        let mut subs: Vec<Sub> = Vec::new();
        let nodes: Vec<u32> = self.nodes.to_vec();
        for &v in &nodes {
            if owner[v as usize] == INF {
                let id = subs.len();
                subs.push(Sub { rep: v, size: 0, heap_index: 0, par: id });
                let size = self.tight_subtree_search(v, id, &mut owner);
                subs[id].size = size;
            }
        }
        let count = subs.len();
        let mut heap: Vec<usize> = (0..count).collect();
        for i in 0..count {
            subs[heap[i]].heap_index = i;
        }
        let mut size = count;
        for i in (0..=size / 2).rev() {
            Self::heapify(&mut heap, size, &mut subs, i);
        }
        while size > 1 {
            // STextractmin
            let t0 = heap[0];
            subs[t0].heap_index = INF;
            heap[0] = heap[size - 1];
            subs[heap[0]].heap_index = 0;
            heap[size - 1] = t0;
            size -= 1;
            Self::heapify(&mut heap, size, &mut subs, 0);

            let rep = subs[t0].rep;
            let Some(ee) = self.inter_tree_edge(&mut subs, &owner, rep) else {
                // not connected: cannot happen for a component
                break;
            };
            // merge_trees
            let (et, eh) = (self.e(ee).tail, self.e(ee).head);
            let a = Self::st_find(&mut subs, &owner, et);
            let b = Self::st_find(&mut subs, &owner, eh);
            if subs[a].heap_index == INF {
                let delta = self.slack(ee);
                if delta != 0 {
                    self.tree_adjust(subs[a].rep, delta);
                }
            } else {
                let delta = -self.slack(ee);
                if delta != 0 {
                    self.tree_adjust(subs[b].rep, delta);
                }
            }
            self.add_tree_edge(ee);
            let r = Self::st_union(&mut subs, a, b);
            let hi = subs[r].heap_index;
            Self::heapify(&mut heap, size, &mut subs, hi);
        }
        // init_cutvalues
        self.dfs_range_init(self.nodes[0]);
        self.dfs_cutval(self.nodes[0]);
    }

    // ---- cut values and DFS ranges ----

    /// `dfs_range_init`: par / low / lim by a DFS over the tree edges (tree_out first, then tree_in).
    fn dfs_range_init(&mut self, root: u32) {
        self.par[root as usize] = NONE;
        self.low[root as usize] = 1;
        // frames: (node, par edge, lim counter, out_i, in_i)
        let mut todo: Vec<(u32, u32, i32, usize, usize)> = vec![(root, NONE, 1, 0, 0)];
        while let Some(&mut (v, pe, ref mut lim, ref mut oi, ref mut ii)) = todo.last_mut() {
            let mut next: Option<(u32, u32)> = None;
            while *oi < self.tree_out[v as usize].len() {
                let e = self.tree_out[v as usize][*oi];
                *oi += 1;
                if e != pe {
                    next = Some((self.g.edges[e as usize].head, e));
                    break;
                }
            }
            if next.is_none() {
                while *ii < self.tree_in[v as usize].len() {
                    let e = self.tree_in[v as usize][*ii];
                    *ii += 1;
                    if e != pe {
                        next = Some((self.g.edges[e as usize].tail, e));
                        break;
                    }
                }
            }
            if let Some((nn, e)) = next {
                self.par[nn as usize] = e;
                self.low[nn as usize] = *lim;
                let l = *lim;
                todo.push((nn, e, l, 0, 0));
            } else {
                self.lim[v as usize] = *lim;
                let l = *lim;
                todo.pop();
                if let Some(top) = todo.last_mut() {
                    top.2 = l + 1;
                }
            }
        }
    }

    /// `dfs_range`: the incremental version, recomputes only below `v` where something changed.
    fn dfs_range(&mut self, v: u32, par: u32, low: i32) {
        if self.par[v as usize] == par && self.low[v as usize] == low {
            return;
        }
        self.par[v as usize] = par;
        self.low[v as usize] = low;
        let mut todo: Vec<(u32, u32, i32, usize, usize)> = vec![(v, par, low, 0, 0)];
        while let Some(&mut (x, pe, ref mut lim, ref mut oi, ref mut ii)) = todo.last_mut() {
            let mut processed = false;
            let mut push: Option<(u32, u32, i32)> = None;
            while *oi < self.tree_out[x as usize].len() {
                let e = self.tree_out[x as usize][*oi];
                *oi += 1;
                if e != pe {
                    let nn = self.g.edges[e as usize].head;
                    if self.par[nn as usize] == e && self.low[nn as usize] == *lim {
                        *lim = self.lim[nn as usize] + 1;
                    } else {
                        self.par[nn as usize] = e;
                        self.low[nn as usize] = *lim;
                        push = Some((nn, e, *lim));
                    }
                    processed = true;
                    break;
                }
            }
            if !processed {
                while *ii < self.tree_in[x as usize].len() {
                    let e = self.tree_in[x as usize][*ii];
                    *ii += 1;
                    if e != pe {
                        let nn = self.g.edges[e as usize].tail;
                        if self.par[nn as usize] == e && self.low[nn as usize] == *lim {
                            *lim = self.lim[nn as usize] + 1;
                        } else {
                            self.par[nn as usize] = e;
                            self.low[nn as usize] = *lim;
                            push = Some((nn, e, *lim));
                        }
                        processed = true;
                        break;
                    }
                }
            }
            if processed {
                if let Some((nn, e, l)) = push {
                    todo.push((nn, e, l, 0, 0));
                }
                continue;
            }
            self.lim[x as usize] = *lim;
            let l = *lim;
            todo.pop();
            if let Some(top) = todo.last_mut() {
                top.2 = l + 1;
            }
        }
    }

    /// `dfs_cutval`: post-order cut values of all tree edges.
    fn dfs_cutval(&mut self, root: u32) {
        let mut todo: Vec<(u32, u32, usize, usize)> = vec![(root, NONE, 0, 0)];
        while let Some(&mut (v, pe, ref mut oi, ref mut ii)) = todo.last_mut() {
            let mut next: Option<(u32, u32)> = None;
            while *oi < self.tree_out[v as usize].len() {
                let e = self.tree_out[v as usize][*oi];
                *oi += 1;
                if e != pe {
                    next = Some((self.g.edges[e as usize].head, e));
                    break;
                }
            }
            if next.is_none() {
                while *ii < self.tree_in[v as usize].len() {
                    let e = self.tree_in[v as usize][*ii];
                    *ii += 1;
                    if e != pe {
                        next = Some((self.g.edges[e as usize].tail, e));
                        break;
                    }
                }
            }
            match next {
                Some((nn, e)) => todo.push((nn, e, 0, 0)),
                None => {
                    if pe != NONE {
                        self.x_cutval(pe);
                    }
                    todo.pop();
                }
            }
        }
    }

    fn x_cutval(&mut self, f: u32) {
        let (ft, fh) = (self.e(f).tail, self.e(f).head);
        let (v, dir) = if self.par[ft as usize] == f { (ft, 1) } else { (fh, -1) };
        let mut sum = 0i32;
        for &e in &self.g.out[v as usize] {
            sum += self.x_val(e, v, dir);
        }
        for &e in &self.g.inn[v as usize] {
            sum += self.x_val(e, v, dir);
        }
        self.g.edges[f as usize].cut = sum;
    }

    fn x_val(&self, e: u32, v: u32, dir: i32) -> i32 {
        let x = self.e(e);
        let other = if x.tail == v { x.head } else { x.tail };
        let inside = self.low[v as usize] <= self.lim[other as usize] && self.lim[other as usize] <= self.lim[v as usize];
        let (f, mut rv) = if !inside { (true, x.weight) } else { (false, (if self.is_tree(e) { x.cut } else { 0 }) - x.weight) };
        let mut d = if dir > 0 {
            if x.head == v {
                1
            } else {
                -1
            }
        } else if x.tail == v {
            1
        } else {
            -1
        };
        if f {
            d = -d;
        }
        if d < 0 {
            rv = -rv;
        }
        rv
    }

    // ---- pivoting ----

    /// `leave_edge`: a tree edge with a negative cut value, searching from where the last search stopped.
    fn leave_edge(&mut self) -> Option<u32> {
        let mut rv: Option<u32> = None;
        let mut cnt = 0;
        let j = self.s_i;
        while self.s_i < self.tree_edge.len() {
            let f = self.tree_edge[self.s_i];
            if self.e(f).cut < 0 {
                match rv {
                    Some(r) => {
                        if self.e(r).cut > self.e(f).cut {
                            rv = Some(f);
                        }
                    }
                    None => rv = Some(f),
                }
                cnt += 1;
                if cnt >= SEARCH_SIZE {
                    return rv;
                }
            }
            self.s_i += 1;
        }
        if j > 0 {
            self.s_i = 0;
            while self.s_i < j {
                let f = self.tree_edge[self.s_i];
                if self.e(f).cut < 0 {
                    match rv {
                        Some(r) => {
                            if self.e(r).cut > self.e(f).cut {
                                rv = Some(f);
                            }
                        }
                        None => rv = Some(f),
                    }
                    cnt += 1;
                    if cnt >= SEARCH_SIZE {
                        return rv;
                    }
                }
                self.s_i += 1;
            }
        }
        rv
    }

    fn in_range(&self, low: i32, x: u32, lim: i32) -> bool {
        low <= self.lim[x as usize] && self.lim[x as usize] <= lim
    }

    /// `enter_edge`: the non-tree edge with least slack that reconnects the two halves of the tree.
    fn enter_edge(&self, e: u32) -> u32 {
        let (t, h) = (self.e(e).tail, self.e(e).head);
        let (v, outsearch) = if self.lim[t as usize] < self.lim[h as usize] { (t, false) } else { (h, true) };
        let (low, lim) = (self.low[v as usize], self.lim[v as usize]);
        let mut enter: Option<u32> = None;
        let mut slack = i32::MAX;
        let mut todo = vec![v];
        while let Some(x) = todo.pop() {
            if outsearch {
                for &e in &self.g.out[x as usize] {
                    let h = self.e(e).head;
                    if !self.is_tree(e) {
                        if !self.in_range(low, h, lim) {
                            let s = self.slack(e);
                            if s < slack || enter.is_none() {
                                enter = Some(e);
                                slack = s;
                            }
                        }
                    } else if self.lim[h as usize] < self.lim[x as usize] {
                        todo.push(h);
                    }
                }
                for &e in &self.tree_in[x as usize] {
                    if slack <= 0 {
                        break;
                    }
                    let t = self.e(e).tail;
                    if self.lim[t as usize] < self.lim[x as usize] {
                        todo.push(t);
                    }
                }
            } else {
                for &e in &self.g.inn[x as usize] {
                    let t = self.e(e).tail;
                    if !self.is_tree(e) {
                        if !self.in_range(low, t, lim) {
                            let s = self.slack(e);
                            if s < slack || enter.is_none() {
                                enter = Some(e);
                                slack = s;
                            }
                        }
                    } else if self.lim[t as usize] < self.lim[x as usize] {
                        todo.push(t);
                    }
                }
                for &e in &self.tree_out[x as usize] {
                    if slack <= 0 {
                        break;
                    }
                    let h = self.e(e).head;
                    if self.lim[h as usize] < self.lim[x as usize] {
                        todo.push(h);
                    }
                }
            }
        }
        enter.expect("network simplex: no entering edge")
    }

    /// `rerank`: subtract `delta` from the rank of everything below `v` (away from its parent edge).
    fn rerank(&mut self, v: u32, delta: i32) {
        let mut stack = vec![v];
        while let Some(x) = stack.pop() {
            self.rank[x as usize] -= delta;
            let pe = self.par[x as usize];
            for &e in &self.tree_out[x as usize] {
                if e != pe {
                    stack.push(self.e(e).head);
                }
            }
            for &e in &self.tree_in[x as usize] {
                if e != pe {
                    stack.push(self.e(e).tail);
                }
            }
        }
    }

    fn treeupdate(&mut self, mut v: u32, w: u32, cutvalue: i32, dir: bool) -> u32 {
        while !(self.low[v as usize] <= self.lim[w as usize] && self.lim[w as usize] <= self.lim[v as usize]) {
            let e = self.par[v as usize];
            let d = if v == self.e(e).tail { dir } else { !dir };
            if d {
                self.g.edges[e as usize].cut += cutvalue;
            } else {
                self.g.edges[e as usize].cut -= cutvalue;
            }
            let (t, h) = (self.e(e).tail, self.e(e).head);
            v = if self.lim[t as usize] > self.lim[h as usize] { t } else { h };
        }
        v
    }

    fn invalidate_path(&mut self, lca: u32, mut to: u32) {
        loop {
            if self.low[to as usize] == -1 {
                break;
            }
            self.low[to as usize] = -1;
            let e = self.par[to as usize];
            if e == NONE {
                break;
            }
            if self.lim[to as usize] >= self.lim[lca as usize] {
                break;
            }
            let (t, h) = (self.e(e).tail, self.e(e).head);
            to = if self.lim[t as usize] > self.lim[h as usize] { t } else { h };
        }
    }

    /// `update`: `e` leaves the tree, `f` enters.
    fn update(&mut self, e: u32, f: u32) {
        let delta = self.slack(f);
        let (et, eh) = (self.e(e).tail, self.e(e).head);
        if delta > 0 {
            let s = self.tree_in[et as usize].len() + self.tree_out[et as usize].len();
            if s == 1 {
                self.rerank(et, delta);
            } else {
                let s = self.tree_in[eh as usize].len() + self.tree_out[eh as usize].len();
                if s == 1 {
                    self.rerank(eh, -delta);
                } else if self.lim[et as usize] < self.lim[eh as usize] {
                    self.rerank(et, delta);
                } else {
                    self.rerank(eh, -delta);
                }
            }
        }
        let cutvalue = self.e(e).cut;
        let (ft, fh) = (self.e(f).tail, self.e(f).head);
        let lca = self.treeupdate(ft, fh, cutvalue, true);
        let lca2 = self.treeupdate(fh, ft, cutvalue, false);
        debug_assert_eq!(lca, lca2);
        let lca_low = self.low[lca as usize];
        self.invalidate_path(lca, fh);
        self.invalidate_path(lca, ft);
        self.g.edges[f as usize].cut = -cutvalue;
        self.g.edges[e as usize].cut = 0;
        // exchange_tree_edges
        let ti = self.e(e).tree_index;
        self.g.edges[f as usize].tree_index = ti;
        self.tree_edge[ti as usize] = f;
        self.g.edges[e as usize].tree_index = -1;
        let (t, h) = (self.e(e).tail, self.e(e).head);
        let j = self.tree_out[t as usize].iter().position(|&x| x == e).expect("tree_out");
        self.tree_out[t as usize].swap_remove(j);
        let j = self.tree_in[h as usize].iter().position(|&x| x == e).expect("tree_in");
        self.tree_in[h as usize].swap_remove(j);
        self.tree_out[ft as usize].push(f);
        self.tree_in[fh as usize].push(f);
        let lca_par = self.par[lca as usize];
        self.dfs_range(lca, lca_par, lca_low);
    }

    /// `TB_balance`: normalise, then move nodes whose in-weight equals their out-weight to the least
    /// populated rank they may take.
    fn balance_tb(&mut self) {
        let nodes: Vec<u32> = self.nodes.to_vec();
        let min = nodes.iter().map(|&v| self.rank[v as usize]).min().unwrap_or(0);
        let mut max = i32::MIN;
        for &v in &nodes {
            self.rank[v as usize] -= min;
            max = max.max(self.rank[v as usize]);
        }
        let mut nrank = vec![0i32; (max.max(0) + 1) as usize];
        let mut order = nodes.clone();
        order.sort_by_key(|&v| self.rank[v as usize]); // increasing, stable
        for &v in &order {
            nrank[self.rank[v as usize] as usize] += 1;
        }
        for &v in &order {
            let (mut inw, mut outw) = (0, 0);
            let mut low = 0;
            let mut high = max;
            for &e in &self.g.inn[v as usize] {
                let x = &self.g.edges[e as usize];
                inw += x.weight;
                low = low.max(self.rank[x.tail as usize] + x.minlen);
            }
            for &e in &self.g.out[v as usize] {
                let x = &self.g.edges[e as usize];
                outw += x.weight;
                high = high.min(self.rank[x.head as usize] - x.minlen);
            }
            if low < 0 {
                low = 0;
            }
            if inw == outw {
                let mut choice = low;
                for i in (low + 1)..=high {
                    if nrank[i as usize] < nrank[choice as usize] {
                        choice = i;
                    }
                }
                nrank[self.rank[v as usize] as usize] -= 1;
                nrank[choice as usize] += 1;
                self.rank[v as usize] = choice;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_and_diamond() {
        // a->b->c, a->c
        assert_eq!(rank_nodes(3, &[(0, 1), (1, 2), (0, 2)]), vec![0, 1, 2]);
    }

    #[test]
    fn cycle_is_broken() {
        let r = rank_nodes(3, &[(0, 1), (1, 2), (2, 0)]);
        assert_eq!(r.iter().max().copied(), Some(2));
    }

    #[test]
    fn components_start_at_zero() {
        let r = rank_nodes(4, &[(0, 1), (2, 3)]);
        assert_eq!(r, vec![0, 1, 0, 1]);
    }

    #[test]
    fn ties_break_like_dot() {
        // Found by comparing with dot 16.1.2: two equally short layouts exist; dot picks this one because a
        // node's out-edges are ordered by the head's creation number, not by the edge's.
        // nodes by first mention: 83 173 58 114 134 198 21 157 140 89
        let e = [(0, 1), (2, 3), (4, 5), (6, 7), (2, 0), (8, 7), (9, 8), (9, 1), (6, 4), (3, 5)];
        assert_eq!(rank_nodes(10, &e), vec![1, 2, 0, 1, 1, 2, 0, 3, 2, 1]);
    }
}
