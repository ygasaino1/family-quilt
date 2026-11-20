//! Ordering inside each connected tree: depth-first start, then barycentre sweeps over the layers
//! person / family / person / ... The best sweep by the quilt score is kept.

use crate::rank::{find, rank_with, union};
use crate::score::{quilt_score, Score};
use crate::{cells_of, invert, Effort, FamilyIn, Layout, Progress, Sink, Snapshot, Stage};

/// Work budget (sum over evaluations of the cell count) for the neighbour-swap refinement on the real
/// score. Deterministic (no clock). A tree gets refined only if one full pass fits in the budget.
pub const REFINE_BUDGET: u64 = 200_000_000;

pub(crate) fn layout(people: usize, tiebreak: &[i64], families: &[FamilyIn], options: &crate::Options, hint: Option<&crate::Hint>) -> Layout {
    layout_run(people, tiebreak, families, options, hint, Effort::Full, &mut |_: Progress| true).expect("a sink that never stops cannot cancel")
}

pub(crate) fn layout_run(people: usize, tiebreak: &[i64], families: &[FamilyIn], options: &crate::Options, hint: Option<&crate::Hint>, effort: Effort, sink: &mut dyn Sink) -> Option<Layout> {
    let quick = effort == Effort::Quick;
    let ranks = rank_with(people, families, !quick);
    if !sink.report(Progress { stage: Stage::Ranking, fraction: 1.0, score: None, snapshot: None }) {
        return None;
    }
    let nf = families.len();

    // Connected trees: everyone in a family is joined.
    let mut uf: Vec<u32> = (0..people as u32).collect();
    for f in families {
        for w in f.parents.iter().chain(&f.children).collect::<Vec<_>>().windows(2) {
            union(&mut uf, *w[0], *w[1]);
        }
    }
    let root: Vec<u32> = (0..people as u32).map(|p| find(&mut uf, p)).collect();
    let mut comp_persons: Vec<Vec<u32>> = vec![Vec::new(); people];
    let mut comp_fams: Vec<Vec<u32>> = vec![Vec::new(); people];
    for p in 0..people as u32 {
        comp_persons[root[p as usize] as usize].push(p);
    }
    for (fi, f) in families.iter().enumerate() {
        if let Some(&p) = f.parents.first().or(f.children.first()) {
            comp_fams[root[p as usize] as usize].push(fi as u32);
        }
    }
    let mut linked: Vec<u32> = (0..people as u32).filter(|&r| root[r as usize] == r && !comp_fams[r as usize].is_empty()).collect();
    // Largest tree first; ties by the smallest person index.
    linked.sort_by_key(|&r| (std::cmp::Reverse(comp_persons[r as usize].len()), r));
    let mut singles: Vec<u32> = (0..people as u32).filter(|&r| root[r as usize] == r && comp_fams[r as usize].is_empty()).collect();
    singles.sort_by_key(|&r| (tiebreak[r as usize], r));

    // person -> families (as parent / as child), global, sorted by family index
    let mut as_parent: Vec<Vec<u32>> = vec![Vec::new(); people];
    let mut as_child: Vec<Vec<u32>> = vec![Vec::new(); people];
    for (fi, f) in families.iter().enumerate() {
        for &p in &f.parents {
            as_parent[p as usize].push(fi as u32);
        }
        for &c in &f.children {
            as_child[c as usize].push(fi as u32);
        }
    }

    let mut out = Layout { person_gen: vec![0; people], family_gen: vec![0; nf], person_tree: vec![0; people], family_tree: vec![0; nf], forced_nodes: ranks.forced, ..Default::default() };
    let mut local_p = vec![0u32; people];
    let mut local_f = vec![0u32; nf];
    let n_linked = linked.len();
    let total_nodes = (people + nf).max(1) as f32;
    let mut done_nodes = 0usize;
    let mut partials: Vec<Option<Partial>> = Vec::new();
    let mut singles_rows: Vec<Vec<u32>> = Vec::new();
    for (tree, &r) in linked.iter().chain(singles.iter()).enumerate() {
        let persons = &comp_persons[r as usize];
        let fams = &comp_fams[r as usize];
        let min_gen = persons.iter().map(|&p| ranks.person[p as usize]).chain(fams.iter().map(|&f| ranks.family[f as usize])).min().unwrap_or(0);
        for &p in persons {
            out.person_gen[p as usize] = ranks.person[p as usize] - min_gen;
            out.person_tree[p as usize] = tree as u32;
        }
        for &f in fams {
            out.family_gen[f as usize] = ranks.family[f as usize] - min_gen;
            out.family_tree[f as usize] = tree as u32;
        }
        if fams.is_empty() {
            partials.push(None);
            singles_rows.push(persons.clone());
            continue;
        }
        singles_rows.push(Vec::new());
        for (i, &p) in persons.iter().enumerate() {
            local_p[p as usize] = i as u32;
        }
        for (i, &f) in fams.iter().enumerate() {
            local_f[f as usize] = i as u32;
        }
        let ctx = Ctx { tiebreak, families, as_parent: &as_parent, as_child: &as_child, person_gen: &out.person_gen, family_gen: &out.family_gen, local_p: &local_p, local_f: &local_f, hint, quick };
        partials.push(Some(start_component(&ctx, persons, fams)));
        done_nodes += persons.len() + fams.len();
        let score = partials.iter().flatten().fold(crate::Score::default(), |a, p| a + p.score);
        if !sink.report(Progress { stage: Stage::Ordering, fraction: done_nodes as f32 / total_nodes, score: Some(score), snapshot: None }) {
            return None;
        }
    }
    let rows_cols = |partials: &[Option<Partial>]| -> (Vec<Vec<u32>>, Vec<Vec<u32>>) {
        let mut tr = Vec::new();
        let mut tc = Vec::new();
        for (t, p) in partials.iter().enumerate() {
            match p {
                Some(p) => {
                    let (r, c) = p.rows_cols();
                    tr.push(r);
                    tc.push(c);
                }
                None => {
                    tr.push(singles_rows[t].clone());
                    tc.push(Vec::new());
                }
            }
        }
        (tr, tc)
    };
    if !quick {
        // Everything is ordered; now polish tree by tree on the real score.
        let after_ordering = partials.iter().flatten().fold(crate::Score::default(), |a, p| a + p.score);
        let snap = sink.wants_snapshots().then(|| {
            let (tr, tc) = rows_cols(&partials);
            let mut tmp = out.clone();
            assemble(&mut tmp, options, tiebreak, n_linked, &tr, &tc);
            Snapshot { rows: tmp.rows, cols: tmp.cols }
        });
        if !sink.report(Progress { stage: Stage::Polishing, fraction: 0.0, score: Some(after_ordering), snapshot: snap }) {
            return None;
        }
        let mut polished = 0usize;
        let polish_total: usize = partials.iter().flatten().map(|p| p.np + p.nfm).sum::<usize>().max(1);
        for i in 0..partials.len() {
            let others = partials.iter().enumerate().filter(|(j, _)| *j != i).filter_map(|(_, p)| p.as_ref()).fold(crate::Score::default(), |a, p| a + p.score);
            let Some(part) = partials[i].as_mut() else { continue };
            let size = part.np + part.nfm;
            let base = polished as f32 / polish_total as f32;
            let mut keep = true;
            part.refine(&mut |s| {
                keep = sink.report(Progress { stage: Stage::Polishing, fraction: base, score: Some(others + s), snapshot: None });
                keep
            });
            if !keep {
                return None;
            }
            polished += size;
            let score = partials.iter().flatten().fold(crate::Score::default(), |a, p| a + p.score);
            if !sink.report(Progress { stage: Stage::Polishing, fraction: polished as f32 / polish_total as f32, score: Some(score), snapshot: None }) {
                return None;
            }
        }
    }
    let (tree_rows, tree_cols) = rows_cols(&partials);
    assemble(&mut out, options, tiebreak, n_linked, &tree_rows, &tree_cols);
    let cells = cells_of(families);
    out.score = quilt_score(&invert(&out.rows), &invert(&out.cols), &cells);
    let snap = sink.wants_snapshots().then(|| Snapshot { rows: out.rows.clone(), cols: out.cols.clone() });
    if !sink.report(Progress { stage: Stage::Done, fraction: 1.0, score: Some(out.score), snapshot: snap }) {
        return None;
    }
    Some(out)
}

/// Put the trees' blocks into the final row and column order (see [`crate::Mode`]).
fn assemble(out: &mut Layout, options: &crate::Options, tiebreak: &[i64], n_linked: usize, tree_rows: &[Vec<u32>], tree_cols: &[Vec<u32>]) {
    use crate::{Align, Mode};
    let Mode::LayerByLayer(align) = options.mode else {
        out.rows = tree_rows.concat();
        out.cols = tree_cols.concat();
        return;
    };
    // Shift (in layers; one generation = 2 layers) per tree.
    let mut shift = vec![0u32; n_linked];
    if let Align::BirthDate { generation_years } = align {
        let gy = f64::from(generation_years.max(1));
        let year = |day: i64| 1970.0 + day as f64 / 365.2425;
        // estimated birth year of each tree's generation 0
        let est: Vec<Option<f64>> = (0..n_linked)
            .map(|t| {
                let xs: Vec<f64> = tree_rows[t].iter().filter(|&&p| tiebreak[p as usize] != i64::MAX).map(|&p| year(tiebreak[p as usize]) - gy * (out.person_gen[p as usize] as f64 / 2.0)).collect();
                (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64)
            })
            .collect();
        if let Some(base) = est.iter().flatten().copied().reduce(f64::min) {
            for t in 0..n_linked {
                if let Some(y0) = est[t] {
                    shift[t] = 2 * ((y0 - base) / gy).round() as u32;
                }
            }
        }
        for t in 0..n_linked {
            for &p in &tree_rows[t] {
                out.person_gen[p as usize] += shift[t];
            }
            for &f in &tree_cols[t] {
                out.family_gen[f as usize] += shift[t];
            }
        }
    }
    // rows by (generation, tree, position in tree); people in no family last
    let mut rows: Vec<(u32, u32, u32, u32)> = Vec::new();
    for (t, rs) in tree_rows.iter().enumerate() {
        for (i, &p) in rs.iter().enumerate() {
            let g = if t < n_linked { out.person_gen[p as usize] } else { u32::MAX };
            rows.push((g, t as u32, i as u32, p));
        }
    }
    rows.sort_unstable();
    out.rows = rows.iter().map(|r| r.3).collect();
    let mut cols: Vec<(u32, u32, u32, u32)> = Vec::new();
    for (t, cs) in tree_cols.iter().enumerate() {
        for (i, &f) in cs.iter().enumerate() {
            cols.push((out.family_gen[f as usize], t as u32, i as u32, f));
        }
    }
    cols.sort_unstable();
    out.cols = cols.iter().map(|r| r.3).collect();
}

struct Ctx<'a> {
    tiebreak: &'a [i64],
    families: &'a [FamilyIn],
    as_parent: &'a [Vec<u32>],
    as_child: &'a [Vec<u32>],
    person_gen: &'a [u32],
    family_gen: &'a [u32],
    local_p: &'a [u32],
    local_f: &'a [u32],
    hint: Option<&'a crate::Hint>,
    quick: bool,
}

fn start_component(c: &Ctx, persons: &[u32], fams: &[u32]) -> Partial {
    let (np, nfm) = (persons.len(), fams.len());
    let n = np + nfm;
    let loc_p = |g: u32| c.local_p[g as usize];
    let loc_f = |g: u32| np as u32 + c.local_f[g as usize];
    // up / down neighbours in local node ids (persons 0..np, families np..)
    let mut up: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut down: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (i, &p) in persons.iter().enumerate() {
        down[i] = c.as_parent[p as usize].iter().map(|&f| loc_f(f)).collect();
        up[i] = c.as_child[p as usize].iter().map(|&f| loc_f(f)).collect();
    }
    for (j, &f) in fams.iter().enumerate() {
        let fam = &c.families[f as usize];
        up[np + j] = fam.parents.iter().map(|&p| loc_p(p)).collect();
        down[np + j] = fam.children.iter().map(|&p| loc_p(p)).collect();
    }
    let layer_of: Vec<u32> = (0..n).map(|v| if v < np { 2 * c.person_gen[persons[v] as usize] } else { 2 * c.family_gen[fams[v - np] as usize] + 1 }).collect();
    let nlayers = layer_of.iter().max().copied().unwrap_or(0) as usize + 1;
    let key = |v: usize| -> (i64, u32) {
        if v < np {
            (c.tiebreak[persons[v] as usize], persons[v])
        } else {
            (i64::MIN, fams[v - np])
        }
    };

    let cells: Vec<(u32, u32)> = {
        let mut v = Vec::new();
        for (j, &f) in fams.iter().enumerate() {
            let fam = &c.families[f as usize];
            for &p in fam.parents.iter().chain(&fam.children) {
                v.push((loc_p(p), j as u32));
            }
        }
        v
    };
    let eval = |layers: &[Vec<u32>]| -> Score { eval_layers(layers, np, nfm, &cells) };
    let variants: &[(bool, bool)] = if c.quick || n > 60_000 {
        &[(false, false)]
    } else if n > 8_000 {
        &[(false, false), (true, false)]
    } else {
        &[(false, false), (true, false), (false, true), (true, true)]
    };
    let mut best: Option<(Score, Vec<Vec<u32>>)> = None;
    if let Some(h) = c.hint {
        let hp = |v: usize| -> Option<u32> {
            if v < np {
                h.person_pos.get(persons[v] as usize).copied().flatten()
            } else {
                h.family_pos.get(fams[v - np] as usize).copied().flatten()
            }
        };
        let l = hinted_layers(&up, &down, &layer_of, nlayers, n, &hp, &key);
        best = Some((eval(&l), l));
    } else {
        for &(median, reverse) in variants {
            let (s, l) = run_variant(&up, &down, &layer_of, nlayers, np, n, &key, &eval, median, reverse);
            if best.as_ref().is_none_or(|(bs, _)| s.key() < bs.key()) {
                best = Some((s, l));
            }
        }
    }
    let (score, layers) = best.unwrap();
    Partial { persons: persons.to_vec(), fams: fams.to_vec(), np, nfm, cells, layers, score }
}

/// One tree after ordering, before the swap refinement.
struct Partial {
    persons: Vec<u32>,
    fams: Vec<u32>,
    np: usize,
    nfm: usize,
    cells: Vec<(u32, u32)>,
    layers: Vec<Vec<u32>>,
    score: Score,
}

fn eval_layers(layers: &[Vec<u32>], np: usize, nfm: usize, cells: &[(u32, u32)]) -> Score {
    let mut rp = vec![0u32; np];
    let mut cp = vec![0u32; nfm];
    let (mut r, mut k) = (0u32, 0u32);
    for l in layers {
        for &v in l {
            if (v as usize) < np {
                rp[v as usize] = r;
                r += 1;
            } else {
                cp[v as usize - np] = k;
                k += 1;
            }
        }
    }
    quilt_score(&rp, &cp, cells)
}

impl Partial {
    fn rows_cols(&self) -> (Vec<u32>, Vec<u32>) {
        let mut rows = Vec::with_capacity(self.np);
        let mut cols = Vec::with_capacity(self.nfm);
        for l in &self.layers {
            for &v in l {
                if (v as usize) < self.np {
                    rows.push(self.persons[v as usize]);
                } else {
                    cols.push(self.fams[v as usize - self.np]);
                }
            }
        }
        (rows, cols)
    }

    /// Local refinement on the real objective: swap neighbours inside a layer while the quilt score improves.
    /// `tick` gets the best score after each pass; returning `false` stops the passes (the caller handles the cancel).
    fn refine(&mut self, tick: &mut dyn FnMut(Score) -> bool) {
        let (np, nfm, n) = (self.np, self.nfm, self.np + self.nfm);
        let evals_allowed = REFINE_BUDGET / (self.cells.len() as u64 + 1);
        if evals_allowed < n as u64 {
            return;
        }
        let mut evals = 0u64;
        'passes: for _ in 0..12 {
            let mut improved = false;
            for li in 0..self.layers.len() {
                for i in 0..self.layers[li].len().saturating_sub(1) {
                    if evals >= evals_allowed {
                        break 'passes;
                    }
                    evals += 1;
                    self.layers[li].swap(i, i + 1);
                    let s = eval_layers(&self.layers, np, nfm, &self.cells);
                    if s.key() < self.score.key() {
                        self.score = s;
                        improved = true;
                    } else {
                        self.layers[li].swap(i, i + 1);
                    }
                }
            }
            if !tick(self.score) || !improved {
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_variant(
    up: &[Vec<u32>],
    down: &[Vec<u32>],
    layer_of: &[u32],
    nlayers: usize,
    np: usize,
    n: usize,
    key: &dyn Fn(usize) -> (i64, u32),
    eval: &dyn Fn(&[Vec<u32>]) -> Score,
    median: bool,
    reverse: bool,
) -> (Score, Vec<Vec<u32>>) {
    // Depth-first start order.
    let mut ord = vec![u32::MAX; n];
    let mut counter = 0u32;
    let mut starts: Vec<usize> = (0..np).collect();
    starts.sort_by_key(|&v| (layer_of[v], key(v)));
    let mut stack: Vec<usize> = Vec::new();
    for &s in starts.iter() {
        stack.push(s);
        while let Some(v) = stack.pop() {
            if ord[v] != u32::MAX {
                continue;
            }
            ord[v] = counter;
            counter += 1;
            if v < np {
                for &f in down[v].iter().rev() {
                    stack.push(f as usize);
                }
            } else {
                let mut kids: Vec<usize> = down[v].iter().map(|&x| x as usize).collect();
                kids.sort_by_key(|&x| key(x));
                if reverse {
                    kids.reverse();
                }
                for &k in kids.iter().rev() {
                    stack.push(k);
                }
                for &p in up[v].iter().rev() {
                    stack.push(p as usize);
                }
            }
        }
    }
    for o in ord.iter_mut() {
        if *o == u32::MAX {
            *o = counter;
            counter += 1;
        }
    }
    let mut layers: Vec<Vec<u32>> = vec![Vec::new(); nlayers];
    for v in 0..n {
        layers[layer_of[v] as usize].push(v as u32);
    }
    for l in layers.iter_mut() {
        l.sort_by_key(|&v| ord[v as usize]);
    }

    let mut pos = vec![0f64; n]; // normalised position in its layer
    let refresh = |layers: &[Vec<u32>], pos: &mut [f64]| {
        for l in layers {
            let len = l.len() as f64;
            for (i, &v) in l.iter().enumerate() {
                pos[v as usize] = (i as f64 + 0.5) / len;
            }
        }
    };
    refresh(&layers, &mut pos);
    let mut best = eval(&layers);
    let mut best_layers = layers.clone();
    let max_sweeps = if n > 60_000 { 6 } else { 30 };
    let mut stale = 0;
    for _ in 0..max_sweeps {
        for pass in 0..2 {
            let idx: Vec<usize> = if pass == 0 { (0..nlayers).collect() } else { (0..nlayers).rev().collect() };
            for li in idx {
                let nb = if pass == 0 { up } else { down };
                let l = &mut layers[li];
                let mut keyed: Vec<(f64, usize, u32)> = l
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| {
                        let ns = &nb[v as usize];
                        let k = if ns.is_empty() {
                            pos[v as usize]
                        } else if median {
                            let mut xs: Vec<f64> = ns.iter().map(|&u| pos[u as usize]).collect();
                            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                            let m = xs.len() / 2;
                            if xs.len() % 2 == 1 {
                                xs[m]
                            } else {
                                (xs[m - 1] + xs[m]) / 2.0
                            }
                        } else {
                            ns.iter().map(|&u| pos[u as usize]).sum::<f64>() / ns.len() as f64
                        };
                        (k, i, v)
                    })
                    .collect();
                keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
                let len = l.len() as f64;
                for (i, (_, _, v)) in keyed.iter().enumerate() {
                    l[i] = *v;
                    pos[*v as usize] = (i as f64 + 0.5) / len;
                }
            }
        }
        let s = eval(&layers);
        if s.key() < best.key() {
            best = s;
            best_layers = layers.clone();
            stale = 0;
        } else {
            stale += 1;
            if stale >= 5 {
                break;
            }
        }
    }
    (best, best_layers)
}

/// Layers ordered by the previous positions; new nodes go to the mean position of their placed neighbours.
fn hinted_layers(up: &[Vec<u32>], down: &[Vec<u32>], layer_of: &[u32], nlayers: usize, n: usize, hp: &dyn Fn(usize) -> Option<u32>, key: &dyn Fn(usize) -> (i64, u32)) -> Vec<Vec<u32>> {
    // normalised position among the old nodes of the same layer
    let mut by_layer: Vec<Vec<u32>> = vec![Vec::new(); nlayers];
    for v in 0..n {
        by_layer[layer_of[v] as usize].push(v as u32);
    }
    let mut norm = vec![f64::NAN; n];
    for l in &by_layer {
        let mut old: Vec<u32> = l.iter().copied().filter(|&v| hp(v as usize).is_some()).collect();
        old.sort_by_key(|&v| (hp(v as usize).unwrap(), key(v as usize)));
        let len = old.len() as f64;
        for (i, &v) in old.iter().enumerate() {
            norm[v as usize] = (i as f64 + 0.5) / len;
        }
    }
    // new nodes: mean of placed neighbours, several rounds so chains of new nodes settle
    for _ in 0..4 {
        for v in 0..n {
            if hp(v).is_some() {
                continue;
            }
            let xs: Vec<f64> = up[v].iter().chain(&down[v]).map(|&u| norm[u as usize]).filter(|x| !x.is_nan()).collect();
            if !xs.is_empty() {
                norm[v] = xs.iter().sum::<f64>() / xs.len() as f64;
            }
        }
    }
    let mut layers = by_layer;
    for l in layers.iter_mut() {
        l.sort_by(|&a, &b| {
            let (ka, kb) = (if norm[a as usize].is_nan() { 2.0 } else { norm[a as usize] }, if norm[b as usize].is_nan() { 2.0 } else { norm[b as usize] });
            ka.partial_cmp(&kb).unwrap().then(hp(a as usize).is_none().cmp(&hp(b as usize).is_none())).then(key(a as usize).cmp(&key(b as usize)))
        });
    }
    layers
}
