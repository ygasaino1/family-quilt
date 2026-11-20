//! Layer assignment on the bipartite graph person -> family -> child.
//!
//! Every edge points down at least one layer. First the longest path from the top, then a few
//! compaction passes that move a node inside its allowed range towards the side that has more
//! edges (this shortens the total edge length, which is what Graphviz's network simplex
//! minimises; this is the simple version). Partners end up on the same layer in the common case
//! because both are pulled down to just above their shared family.

use crate::FamilyIn;

pub(crate) fn find(p: &mut [u32], mut x: u32) -> u32 {
    while p[x as usize] != x {
        p[x as usize] = p[p[x as usize] as usize];
        x = p[x as usize];
    }
    x
}

pub(crate) fn union(p: &mut [u32], a: u32, b: u32) -> bool {
    let (ra, rb) = (find(p, a), find(p, b));
    if ra == rb {
        return false;
    }
    let (lo, hi) = (ra.min(rb), ra.max(rb));
    p[hi as usize] = lo;
    true
}

/// Trees with more nodes than this use the simple compaction instead of network simplex.
pub const SIMPLEX_MAX_NODES: usize = 12_000;

pub struct Ranks {
    /// Layer of each person and each family (absolute over the whole input; callers normalise per tree).
    pub person: Vec<u32>,
    pub family: Vec<u32>,
    /// Number of nodes that had to be forced because the parent-child data contains a cycle.
    pub forced: usize,
}

pub fn rank(people: usize, families: &[FamilyIn]) -> Ranks {
    rank_with(people, families, true)
}

/// `use_simplex = false` skips the network simplex pass (longest path + compaction only): much faster on big trees, a little less compact.
pub fn rank_with(people: usize, families: &[FamilyIn], use_simplex: bool) -> Ranks {
    let nf = families.len();
    let n = people + nf;
    // Edges: person p -> family f (parent), family f -> person c (child). CSR for succ and pred.
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for (j, f) in families.iter().enumerate() {
        let fv = (people + j) as u32;
        for &p in &f.parents {
            edges.push((p, fv));
        }
        for &c in &f.children {
            edges.push((fv, c));
        }
    }
    type Edge = (u32, u32);
    let csr = |key: &dyn Fn(&Edge) -> Edge| {
        let mut start = vec![0u32; n + 1];
        for e in &edges {
            start[key(e).0 as usize + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut list = vec![0u32; edges.len()];
        for e in &edges {
            let (a, b) = key(e);
            list[fill[a as usize] as usize] = b;
            fill[a as usize] += 1;
        }
        (start, list)
    };
    let (ss, sl) = csr(&|e| (e.0, e.1));
    let (ps, pl) = csr(&|e| (e.1, e.0));
    let succ = |v: usize| &sl[ss[v] as usize..ss[v + 1] as usize];
    let pred = |v: usize| &pl[ps[v] as usize..ps[v + 1] as usize];

    // Longest path (Kahn, deterministic). Cycles: force the unfinished node with the fewest unmet preds.
    let mut layer = vec![0u32; n];
    let mut deg: Vec<u32> = (0..n).map(|v| pred(v).len() as u32).collect();
    let mut done = vec![false; n];
    let mut queue: Vec<u32> = (0..n as u32).filter(|&v| deg[v as usize] == 0).collect();
    let mut topo = Vec::with_capacity(n);
    let (mut qi, mut forced, mut scan) = (0, 0, 0usize);
    loop {
        while qi < queue.len() {
            let v = queue[qi] as usize;
            qi += 1;
            if done[v] {
                continue;
            }
            done[v] = true;
            topo.push(v as u32);
            for &w in succ(v) {
                let w = w as usize;
                if done[w] {
                    continue;
                }
                layer[w] = layer[w].max(layer[v] + 1);
                deg[w] -= 1;
                if deg[w] == 0 {
                    queue.push(w as u32);
                }
            }
        }
        if topo.len() == n {
            break;
        }
        while done[scan] {
            scan += 1;
        }
        let v = (scan..n).filter(|&v| !done[v]).min_by_key(|&v| (deg[v], v)).unwrap();
        forced += 1;
        deg[v] = 0;
        queue.push(v as u32);
    }
    // Position in the topological order, to ignore the back edges that a forced node leaves behind.
    let mut rank_of = vec![0u32; n];
    for (i, &v) in topo.iter().enumerate() {
        rank_of[v as usize] = i as u32;
    }

    // Compaction: each node sits in [lo, hi]; go to lo if it has more incoming edges, to hi if more outgoing.
    for pass in 0..40 {
        let mut changed = false;
        let it: Box<dyn Iterator<Item = &u32>> = if pass % 2 == 0 { Box::new(topo.iter().rev()) } else { Box::new(topo.iter()) };
        for &v in it {
            let v = v as usize;
            let preds: Vec<usize> = pred(v).iter().map(|&u| u as usize).filter(|&u| rank_of[u] < rank_of[v]).collect();
            let succs: Vec<usize> = succ(v).iter().map(|&u| u as usize).filter(|&u| rank_of[u] > rank_of[v]).collect();
            let lo = preds.iter().map(|&u| layer[u] + 1).max().unwrap_or(0);
            let hi = succs.iter().map(|&u| layer[u] - 1).min();
            let target = match (preds.len().cmp(&succs.len()), hi) {
                (std::cmp::Ordering::Greater, _) => lo,
                (std::cmp::Ordering::Less, Some(h)) => h.max(lo),
                _ => layer[v],
            };
            if target != layer[v] && target >= lo && hi.is_none_or(|h| target <= h) {
                layer[v] = target;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Network simplex per connected tree (only for DAGs of moderate size; see `simplex`).
    if forced == 0 {
        let mut comp = vec![u32::MAX; n];
        for s in 0..n {
            if comp[s] != u32::MAX {
                continue;
            }
            let mut members = vec![s as u32];
            comp[s] = s as u32;
            let mut i = 0;
            while i < members.len() {
                let v = members[i] as usize;
                i += 1;
                for &w in succ(v).iter().chain(pred(v)) {
                    if comp[w as usize] == u32::MAX {
                        comp[w as usize] = s as u32;
                        members.push(w);
                    }
                }
            }
            if members.len() < 3 || members.len() > SIMPLEX_MAX_NODES || !use_simplex {
                continue;
            }
            members.sort_unstable();
            let local = |g: u32| members.binary_search(&g).unwrap() as u32;
            let mut le: Vec<(u32, u32)> = Vec::new();
            for &v in &members {
                for &w in succ(v as usize) {
                    le.push((local(v), local(w)));
                }
            }
            let mut r: Vec<i64> = members.iter().map(|&v| layer[v as usize] as i64).collect();
            let before = r.clone();
            let t0 = std::time::Instant::now();
            let res = crate::simplex::network_simplex(members.len(), &le, &mut r, 40 * members.len() + 100);
            if std::env::var_os("FQ_DEBUG").is_some() {
                eprintln!("simplex: {} nodes, {} edges, pivots {:?}, {:?}", members.len(), le.len(), res, t0.elapsed());
            }
            if res.is_some() {
                let len = |r: &[i64]| le.iter().map(|&(a, b)| r[b as usize] - r[a as usize]).sum::<i64>();
                if len(&r) <= len(&before) {
                    for (i, &v) in members.iter().enumerate() {
                        layer[v as usize] = r[i] as u32;
                    }
                }
            }
        }
    }
    Ranks { person: layer[..people].to_vec(), family: layer[people..].to_vec(), forced }
}
