//! Ordering inside each connected tree: depth-first start, then barycentre sweeps over the layers
//! person / family / person / ... The best sweep by the quilt score is kept.

use crate::rank::{find, rank, union};
use crate::score::{quilt_score, Score};
use crate::{cells_of, invert, FamilyIn, Layout};

/// Work budget (sum over evaluations of the cell count) for the neighbour-swap refinement on the real
/// score. Deterministic (no clock). A tree gets refined only if one full pass fits in the budget.
pub const REFINE_BUDGET: u64 = 200_000_000;

struct CompResult {
    rows: Vec<u32>,
    cols: Vec<u32>,
}

pub(crate) fn layout(people: usize, tiebreak: &[i64], families: &[FamilyIn], hint: Option<&crate::Hint>) -> Layout {
    let ranks = rank(people, families);
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
            out.rows.extend(persons);
            continue;
        }
        for (i, &p) in persons.iter().enumerate() {
            local_p[p as usize] = i as u32;
        }
        for (i, &f) in fams.iter().enumerate() {
            local_f[f as usize] = i as u32;
        }
        let ctx = Ctx { tiebreak, families, as_parent: &as_parent, as_child: &as_child, person_gen: &out.person_gen, family_gen: &out.family_gen, local_p: &local_p, local_f: &local_f, hint };
        let res = order_component(&ctx, persons, fams);
        out.rows.extend(res.rows);
        out.cols.extend(res.cols);
    }
    let cells = cells_of(families);
    out.score = quilt_score(&invert(&out.rows), &invert(&out.cols), &cells);
    out
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
}

fn order_component(c: &Ctx, persons: &[u32], fams: &[u32]) -> CompResult {
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
    let eval = |layers: &[Vec<u32>]| -> Score {
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
        quilt_score(&rp, &cp, &cells)
    };
    let variants: &[(bool, bool)] = if n > 60_000 {
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
    let (mut best_score, mut best_layers) = best.unwrap();
    // Local refinement on the real objective: swap neighbours inside a layer while the quilt score improves.
    let evals_allowed = REFINE_BUDGET / (cells.len() as u64 + 1);
    if evals_allowed >= n as u64 {
        let mut evals = 0u64;
        'passes: for _ in 0..12 {
            let mut improved = false;
            for li in 0..best_layers.len() {
                for i in 0..best_layers[li].len().saturating_sub(1) {
                    if evals >= evals_allowed {
                        break 'passes;
                    }
                    evals += 1;
                    best_layers[li].swap(i, i + 1);
                    let s = eval(&best_layers);
                    if s.key() < best_score.key() {
                        best_score = s;
                        improved = true;
                    } else {
                        best_layers[li].swap(i, i + 1);
                    }
                }
            }
            if !improved {
                break;
            }
        }
    }
    let mut rows = Vec::with_capacity(np);
    let mut cols = Vec::with_capacity(nfm);
    for l in &best_layers {
        for &v in l {
            if (v as usize) < np {
                rows.push(persons[v as usize]);
            } else {
                cols.push(fams[v as usize - np]);
            }
        }
    }
    CompResult { rows, cols }
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
