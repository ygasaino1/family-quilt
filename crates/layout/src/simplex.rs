//! Network simplex layer assignment (Gansner, Koutsofios, North, Vo: "A Technique for Drawing
//! Directed Graphs", 1993): minimise the sum of edge lengths, every edge at least 1 long.
//! Written from the paper. Each pivot recomputes the cut values in O(n + e), which is fine for the
//! tree sizes we use it on (callers fall back to a simpler method for huge trees).

/// `edges[i] = (tail, head)`; `rank` must already be feasible (rank[head] >= rank[tail] + 1) and the
/// graph connected. Returns the number of pivots, or `None` if `max_pivots` was hit (ranks are still feasible).
pub fn network_simplex(m: usize, edges: &[(u32, u32)], rank: &mut [i64], max_pivots: usize) -> Option<usize> {
    if m <= 1 || edges.is_empty() {
        return Some(0);
    }
    let mut out: Vec<Vec<u32>> = vec![Vec::new(); m];
    let mut inn: Vec<Vec<u32>> = vec![Vec::new(); m];
    for (i, &(u, v)) in edges.iter().enumerate() {
        out[u as usize].push(i as u32);
        inn[v as usize].push(i as u32);
    }
    // adj[v]: (edge id, other end, v is the tail of the edge)
    let mut adj: Vec<Vec<(u32, u32, bool)>> = vec![Vec::new(); m];
    for (i, &(u, v)) in edges.iter().enumerate() {
        adj[u as usize].push((i as u32, v, true));
        adj[v as usize].push((i as u32, u, false));
    }
    let slack = |rank: &[i64], e: usize| rank[edges[e].1 as usize] - rank[edges[e].0 as usize] - 1;

    // ── feasible tight spanning tree ──
    let mut in_tree = vec![false; m];
    let mut tree_edge = vec![false; edges.len()];
    let mut size = 1;
    in_tree[0] = true;
    let grow = |start: usize, rank: &[i64], in_tree: &mut Vec<bool>, tree_edge: &mut Vec<bool>, size: &mut usize| {
        let mut stack = vec![start];
        while let Some(v) = stack.pop() {
            for &e in out[v].iter().chain(inn[v].iter()) {
                let e = e as usize;
                let o = if edges[e].0 as usize == v { edges[e].1 } else { edges[e].0 } as usize;
                if !in_tree[o] && slack(rank, e) == 0 {
                    in_tree[o] = true;
                    tree_edge[e] = true;
                    *size += 1;
                    stack.push(o);
                }
            }
        }
    };
    grow(0, rank, &mut in_tree, &mut tree_edge, &mut size);
    while size < m {
        let mut best: Option<(i64, usize)> = None;
        for e in 0..edges.len() {
            let (a, b) = (in_tree[edges[e].0 as usize], in_tree[edges[e].1 as usize]);
            if a != b {
                let s = slack(rank, e);
                if best.is_none_or(|(bs, _)| s < bs) {
                    best = Some((s, e));
                }
            }
        }
        let (s, e) = best?;
        let delta = if in_tree[edges[e].1 as usize] { -s } else { s };
        for v in 0..m {
            if in_tree[v] {
                rank[v] += delta;
            }
        }
        let outside = if in_tree[edges[e].0 as usize] { edges[e].1 } else { edges[e].0 } as usize;
        in_tree[outside] = true;
        tree_edge[e] = true;
        size += 1;
        // grow from every node already in the tree that touches the new tight edges
        for v in 0..m {
            if in_tree[v] {
                grow(v, rank, &mut in_tree, &mut tree_edge, &mut size);
            }
        }
    }

    // ── pivots ──
    let mut par_e = vec![u32::MAX; m];
    let (mut low, mut lim) = (vec![0u32; m], vec![0u32; m]);
    let mut cut = vec![0i64; edges.len()];
    let mut post: Vec<u32> = Vec::with_capacity(m);
    let mut pivots = 0;
    let mut visited = vec![false; m];
    loop {
        // low / lim / parent edge by DFS from node 0 over tree edges
        post.clear();
        par_e.iter_mut().for_each(|x| *x = u32::MAX);
        let mut counter = 1u32;
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        low[0] = 1;
        visited.iter_mut().for_each(|x| *x = false);
        visited[0] = true;
        while let Some(&mut (v, ref mut i)) = stack.last_mut() {
            let mut pushed = false;
            while *i < adj[v].len() {
                let (e, o, _) = adj[v][*i];
                *i += 1;
                if !tree_edge[e as usize] || visited[o as usize] {
                    continue;
                }
                visited[o as usize] = true;
                par_e[o as usize] = e;
                low[o as usize] = counter;
                stack.push((o as usize, 0));
                pushed = true;
                break;
            }
            if !pushed {
                lim[v] = counter;
                counter += 1;
                post.push(v as u32);
                stack.pop();
            }
        }
        // cut values, children before parents (Gansner et al., x_cutval)
        for &v in post.iter() {
            let v = v as usize;
            let pe = par_e[v];
            if pe == u32::MAX {
                continue;
            }
            let f = pe as usize;
            let v_is_tail_of_f = edges[f].0 as usize == v;
            let mut sum = 0i64;
            for &(e, o, v_tail) in &adj[v] {
                let (e, o) = (e as usize, o as usize);
                let inside = low[v] <= lim[o] && lim[o] <= lim[v];
                let (flip, mut rv) = if !inside { (true, 1i64) } else { (false, if tree_edge[e] { cut[e] } else { 0 } - 1) };
                // direction of the edge relative to v, as seen from the cut edge f
                let mut d: i64 = if v_is_tail_of_f {
                    if !v_tail {
                        1
                    } else {
                        -1
                    }
                } else if v_tail {
                    1
                } else {
                    -1
                };
                if flip {
                    d = -d;
                }
                if d < 0 {
                    rv = -rv;
                }
                sum += rv;
            }
            cut[f] = sum;
        }
        // leaving edge: a tree edge with a negative cut value
        let Some(leave) = (0..edges.len()).find(|&e| tree_edge[e] && cut[e] < 0) else { break };
        if pivots >= max_pivots {
            return None;
        }
        pivots += 1;
        let (lt, lh) = (edges[leave].0 as usize, edges[leave].1 as usize);
        // The subtree side is the child endpoint (the one whose parent edge is `leave`).
        let child = if par_e[lt] == leave as u32 { lt } else { lh };
        let tail_in_sub = child == lt;
        let in_sub = |x: usize| low[child] <= lim[x] && lim[x] <= lim[child];
        // entering edge: non-tree edge from the head component to the tail component, minimal slack
        let mut enter: Option<(i64, usize)> = None;
        for e in 0..edges.len() {
            if tree_edge[e] {
                continue;
            }
            let (a, b) = (edges[e].0 as usize, edges[e].1 as usize);
            // head component = subtree if !tail_in_sub (head is child), else the rest
            let from_head_comp = in_sub(a) != tail_in_sub;
            let to_tail_comp = in_sub(b) == tail_in_sub;
            if from_head_comp && to_tail_comp {
                let s = slack(rank, e);
                if enter.is_none_or(|(bs, _)| s < bs) {
                    enter = Some((s, e));
                }
            }
        }
        let (s, e) = enter?;
        // tighten e by moving the tail component down by s... (tail component nodes: rank -= s)
        if s != 0 {
            for (x, r) in rank.iter_mut().enumerate() {
                if in_sub(x) == tail_in_sub {
                    *r -= s;
                }
            }
        }
        tree_edge[leave] = false;
        tree_edge[e] = true;
    }
    let min = rank.iter().copied().min().unwrap_or(0);
    rank.iter_mut().for_each(|r| *r -= min);
    Some(pivots)
}
