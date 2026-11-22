//! Optional engine: the layout the original GeneaQuilts uses, i.e. Graphviz `dot` itself.
//!
//! The original has no layering or ordering code of its own. `DOTWriter` writes the bipartite graph
//! (child -> family -> parent, invisible edges), `dot -Tplain` places it, `DOTLayersReader` turns the
//! sorted distinct y values into layers and keeps x, and `QuiltManager` sorts each layer by
//! (component, x). This module does the same so the quilt can be drawn in exactly that order.
//!
//! `dot` is an external program, never a requirement: [`find_dot`] returns `None` when it is absent and
//! the caller falls back to the built-in engine. Nothing but anonymous node numbers is sent to it.

#![allow(clippy::needless_range_loop)] // several parallel arrays are indexed together; the index loops read better

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::{cells_of, invert, quilt_score, FamilyIn, Layout, Mode, Options, Progress, Sink, Stage};

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// Where to look for `dot`: `FAMILY_DOT` (a file), then beside the app (`dot`, `graphviz/bin`, `graphviz`),
/// then the PATH.
pub fn find_dot() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FAMILY_DOT").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let name = exe_name("dot");
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        for sub in ["", "dot", "graphviz", "graphviz/bin", "Graphviz/bin"] {
            let c = if sub.is_empty() { dir.join(&name) } else { dir.join(sub).join(&name) };
            if c.is_file() {
                return Some(c);
            }
        }
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(&name)).find(|c| c.is_file()))
}

/// The edges the original writes, in its order: it walks the people and for each one adds the edge
/// person -> the family it is a child of, then family -> person for every family it is a parent of.
/// Numbers: person `p` is `p`, family `j` is `people + j`. (The original's own order is whatever its Java hash
/// map gives, so no order of ours can claim to be "the" original order; this one has the same shape.)
pub fn graph_edges(people: usize, families: &[FamilyIn]) -> Vec<(u32, u32)> {
    let mut child_of: Vec<Vec<u32>> = vec![Vec::new(); people];
    let mut parent_in: Vec<Vec<u32>> = vec![Vec::new(); people];
    for (j, f) in families.iter().enumerate() {
        for &c in &f.children {
            if !child_of[c as usize].contains(&(j as u32)) {
                child_of[c as usize].push(j as u32);
            }
        }
        for &p in &f.parents {
            if !parent_in[p as usize].contains(&(j as u32)) {
                parent_in[p as usize].push(j as u32);
            }
        }
    }
    let mut edges = Vec::new();
    for p in 0..people {
        for &j in &child_of[p] {
            edges.push((p as u32, (people as u32) + j));
        }
        for &j in &parent_in[p] {
            edges.push(((people as u32) + j, p as u32));
        }
    }
    edges
}

/// The graph as the original writes it. Node names are plain numbers: `p<person>` and `f<family>`.
pub fn dot_source(people: usize, families: &[FamilyIn]) -> String {
    let name = |v: u32| if (v as usize) < people { format!("p{v}") } else { format!("f{}", v as usize - people) };
    let mut s = String::from("digraph genealogy {\nranksep=1;\nnode [shape=point,width=0,height=0,label=\"\"];\nedge [style=invis];\n");
    for (a, b) in graph_edges(people, families) {
        s.push_str(&format!("\"{}\"->\"{}\";\n", name(a), name(b)));
    }
    s.push_str("}\n");
    s
}

struct Placed {
    x: f64,
    y: f64,
}

/// Run `dot -Tplain` on `source`; `None` if the sink asked to stop.
fn run_dot(program: &Path, source: String, sink: &mut dyn Sink) -> Result<Option<HashMap<String, Placed>>, String> {
    let mut child = Command::new(program).arg("-Tplain").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("cannot start dot: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    // Feed and drain on their own threads so a big graph cannot deadlock the pipes.
    std::thread::spawn(move || {
        let _ = stdin.write_all(source.as_bytes());
    });
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let r = stdout.read_to_end(&mut buf).map(|_| buf);
        let _ = tx.send(r);
    });
    let bytes = loop {
        match rx.recv_timeout(Duration::from_millis(60)) {
            Ok(r) => break r.map_err(|e| format!("cannot read dot output: {e}"))?,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !sink.report(Progress { stage: Stage::Ranking, fraction: 0.5, score: None, snapshot: None }) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(None);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("dot ended without output".into()),
        }
    };
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("dot failed ({status})"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut nodes = HashMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 || f[0] != "node" {
            continue;
        }
        let (Ok(x), Ok(y)) = (f[2].parse::<f64>(), f[3].parse::<f64>()) else { continue };
        nodes.insert(f[1].trim_matches('"').to_string(), Placed { x, y });
    }
    if nodes.is_empty() {
        return Err("dot returned no nodes".into());
    }
    Ok(Some(nodes))
}

/// Where an engine put every person (`p_*`; layer -1 = in no family) and every family (`f_*`): the layer
/// (0 = oldest) and the x that orders the nodes of one layer.
struct Placement {
    p_layer: Vec<i32>,
    p_x: Vec<f64>,
    f_layer: Vec<i32>,
    f_x: Vec<f64>,
}

/// Lay out with `dot`. Same output shape as the built-in engine. `Err` = could not use dot (the caller
/// falls back); `Ok(None)` = cancelled.
pub fn layout_with_dot(people: usize, tiebreak: &[i64], families: &[FamilyIn], options: &Options, program: &Path, sink: &mut dyn Sink) -> Result<Option<Layout>, String> {
    let nf = families.len();
    let Some(placed) = run_dot(program, dot_source(people, families), sink)? else { return Ok(None) };

    // layer = index of the node's y among the sorted distinct y values (smallest y = oldest ancestors)
    let mut ys: Vec<f64> = placed.values().map(|p| p.y).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let layer_of = |y: f64| ys.iter().position(|v| (*v - y).abs() < 1e-6).unwrap_or(0) as i32;
    let mut pl = Placement { p_layer: vec![-1; people], p_x: vec![0.0; people], f_layer: vec![0; nf], f_x: vec![0.0; nf] };
    for p in 0..people {
        if let Some(n) = placed.get(&format!("p{p}")) {
            pl.p_layer[p] = layer_of(n.y);
            pl.p_x[p] = n.x;
        }
    }
    for f in 0..nf {
        let n = placed.get(&format!("f{f}")).ok_or_else(|| format!("dot did not place family {f}"))?;
        pl.f_layer[f] = layer_of(n.y);
        pl.f_x[f] = n.x;
    }
    finish(people, tiebreak, families, options, pl, sink)
}

/// Lay out with our own port of dot's ranking and ordering ([`crate::dotrank`], [`crate::dotorder`]): the same
/// layers and the same order inside them as `dot` gives, with nothing to install. `Ok(None)` = cancelled.
pub fn layout_with_port(people: usize, tiebreak: &[i64], families: &[FamilyIn], options: &Options, sink: &mut dyn Sink) -> Result<Option<Layout>, String> {
    let nf = families.len();
    let edges = graph_edges(people, families);
    // number the nodes in the order of first mention, as dot creates them
    let mut node_of: Vec<u32> = vec![u32::MAX; people + nf];
    let mut graph_id: Vec<u32> = Vec::new();
    let mut local: Vec<(u32, u32)> = Vec::with_capacity(edges.len());
    for &(a, b) in &edges {
        for v in [a, b] {
            if node_of[v as usize] == u32::MAX {
                node_of[v as usize] = graph_id.len() as u32;
                graph_id.push(v);
            }
        }
        local.push((node_of[a as usize], node_of[b as usize]));
    }
    if !sink.report(Progress { stage: Stage::Ranking, fraction: 0.1, score: None, snapshot: None }) {
        return Ok(None);
    }
    let l = crate::dotorder::layout_nodes(graph_id.len(), &local);
    // dot's rank 0 is the top (the youngest here); layer 0 is the oldest, as with `-Tplain` y sorted upwards
    let maxrank = l.rank.iter().copied().max().unwrap_or(0);
    let mut pl = Placement { p_layer: vec![-1; people], p_x: vec![0.0; people], f_layer: vec![0; nf], f_x: vec![0.0; nf] };
    for p in 0..people {
        let n = node_of[p];
        if n != u32::MAX {
            pl.p_layer[p] = maxrank - l.rank[n as usize];
            pl.p_x[p] = f64::from(l.pos[n as usize]);
        }
    }
    for f in 0..nf {
        let n = node_of[people + f];
        if n == u32::MAX {
            return Err(format!("family {f} has no members"));
        }
        pl.f_layer[f] = maxrank - l.rank[n as usize];
        pl.f_x[f] = f64::from(l.pos[n as usize]);
    }
    finish(people, tiebreak, families, options, pl, sink)
}

/// Everything after the placement, shared by both engines: trees, generations, rows and columns, score.
///
/// Generations follow the original: `layer / 2` for both people and families, so a person on an odd
/// layer (a long edge) shares a block with the people on the even layer before it. In our units one
/// generation is two layers: people are `2 * (layer / 2)`, families `2 * (layer / 2) + 1`.
fn finish(people: usize, tiebreak: &[i64], families: &[FamilyIn], options: &Options, pl: Placement, sink: &mut dyn Sink) -> Result<Option<Layout>, String> {
    let nf = families.len();
    let Placement { mut p_layer, p_x, mut f_layer, f_x } = pl;
    // Connected trees (everyone in a family is joined), largest first, as the built-in engine numbers them.
    let mut uf: Vec<u32> = (0..people as u32).collect();
    for f in families {
        let all: Vec<u32> = f.parents.iter().chain(&f.children).copied().collect();
        for w in all.windows(2) {
            let (a, b) = (find(&mut uf, w[0]), find(&mut uf, w[1]));
            if a != b {
                uf[a as usize] = b;
            }
        }
    }
    let root: Vec<u32> = (0..people as u32).map(|p| find(&mut uf, p)).collect();
    let linked_person: Vec<bool> = p_layer.iter().map(|&l| l >= 0).collect();
    let mut size: HashMap<u32, usize> = HashMap::new();
    for p in 0..people {
        if linked_person[p] {
            *size.entry(root[p]).or_default() += 1;
        }
    }
    let mut trees: Vec<u32> = size.keys().copied().collect();
    trees.sort_by_key(|r| (std::cmp::Reverse(size[r]), *r));
    let tree_no: HashMap<u32, u32> = trees.iter().enumerate().map(|(i, r)| (*r, i as u32)).collect();
    let fam_root = |f: usize| root[*families[f].parents.first().or(families[f].children.first()).expect("a placed family has members") as usize];

    // fixLayers: with several trees, a tree whose deepest node is of the other kind than the deepest
    // tree's is moved one layer up, so people line up with people.
    if trees.len() > 1 {
        let mut depth: HashMap<u32, (i32, i32, bool)> = HashMap::new(); // tree -> (min, max, deepest node is a family)
        let mut note = |r: u32, layer: i32, fam: bool| {
            let e = depth.entry(r).or_insert((layer, layer, fam));
            e.0 = e.0.min(layer);
            if layer > e.1 {
                e.1 = layer;
                e.2 = fam;
            }
        };
        for p in 0..people {
            if linked_person[p] {
                note(root[p], p_layer[p], false);
            }
        }
        for f in 0..nf {
            note(fam_root(f), f_layer[f], true);
        }
        let deepest = trees.iter().copied().max_by_key(|r| (depth[r].1 - depth[r].0, std::cmp::Reverse(*r))).unwrap_or(trees[0]);
        let want = depth[&deepest].2;
        let shift: HashMap<u32, i32> = trees.iter().map(|&r| (r, if r != deepest && depth[&r].2 != want { 1 } else { 0 })).collect();
        for p in 0..people {
            if linked_person[p] {
                p_layer[p] -= shift[&root[p]];
            }
        }
        for f in 0..nf {
            f_layer[f] -= shift[&fam_root(f)];
        }
    }

    // generations in our units (see above). A shifted layer may be -1: the original clamps it to 0.
    let half = |l: i32| (l / 2).max(0) as u32;
    // familyFirst: when layer 0 holds families and no person (a family without parents), the diagonal starts with a
    // block of columns, so the parity flips: families are even generations, people odd.
    let family_first = f_layer.contains(&0) && !p_layer.contains(&0);
    let (pg, fg) = if family_first { (1u32, 0u32) } else { (0, 1) };
    let mut out = Layout { person_gen: vec![0; people], family_gen: vec![0; nf], person_tree: vec![0; people], family_tree: vec![0; nf], ..Default::default() };
    let n_trees = trees.len() as u32;
    let mut singles: Vec<u32> = (0..people as u32).filter(|&p| !linked_person[p as usize]).collect();
    singles.sort_by_key(|&p| (tiebreak[p as usize], p));
    for p in 0..people {
        if linked_person[p] {
            out.person_gen[p] = 2 * half(p_layer[p]) + pg;
            out.person_tree[p] = tree_no[&root[p]];
        }
    }
    for (i, &p) in singles.iter().enumerate() {
        out.person_tree[p as usize] = n_trees + i as u32;
    }
    for f in 0..nf {
        out.family_gen[f] = 2 * half(f_layer[f]) + fg;
        out.family_tree[f] = tree_no[&fam_root(f)];
    }
    // Tree by tree: each tree starts at generation 0 of its own.
    let layered = matches!(options.mode, Mode::LayerByLayer(_));
    if !layered {
        let mut min_gen = vec![u32::MAX; trees.len()];
        for p in 0..people {
            if linked_person[p] {
                let t = out.person_tree[p] as usize;
                min_gen[t] = min_gen[t].min(out.person_gen[p]);
            }
        }
        for f in 0..nf {
            let t = out.family_tree[f] as usize;
            min_gen[t] = min_gen[t].min(out.family_gen[f]);
        }
        for p in 0..people {
            if linked_person[p] {
                out.person_gen[p] -= min_gen[out.person_tree[p] as usize];
            }
        }
        for f in 0..nf {
            out.family_gen[f] -= min_gen[out.family_tree[f] as usize];
        }
    }

    // Rows and columns: by x inside a generation (the original sorts each layer by component, then x).
    let by_x = |a: f64, b: f64| a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal);
    let mut rows: Vec<u32> = (0..people as u32).filter(|&p| linked_person[p as usize]).collect();
    let mut cols: Vec<u32> = (0..nf as u32).collect();
    if layered {
        rows.sort_by(|&a, &b| {
            (out.person_gen[a as usize], out.person_tree[a as usize]).cmp(&(out.person_gen[b as usize], out.person_tree[b as usize])).then(by_x(p_x[a as usize], p_x[b as usize])).then(a.cmp(&b))
        });
        cols.sort_by(|&a, &b| {
            (out.family_gen[a as usize], out.family_tree[a as usize]).cmp(&(out.family_gen[b as usize], out.family_tree[b as usize])).then(by_x(f_x[a as usize], f_x[b as usize])).then(a.cmp(&b))
        });
    } else {
        rows.sort_by(|&a, &b| {
            (out.person_tree[a as usize], out.person_gen[a as usize]).cmp(&(out.person_tree[b as usize], out.person_gen[b as usize])).then(by_x(p_x[a as usize], p_x[b as usize])).then(a.cmp(&b))
        });
        cols.sort_by(|&a, &b| {
            (out.family_tree[a as usize], out.family_gen[a as usize]).cmp(&(out.family_tree[b as usize], out.family_gen[b as usize])).then(by_x(f_x[a as usize], f_x[b as usize])).then(a.cmp(&b))
        });
    }
    rows.extend(singles);
    out.rows = rows;
    out.cols = cols;
    out.score = quilt_score(&invert(&out.rows), &invert(&out.cols), &cells_of(families));
    if !sink.report(Progress { stage: Stage::Done, fraction: 1.0, score: Some(out.score), snapshot: None }) {
        return Ok(None);
    }
    Ok(Some(out))
}

fn find(uf: &mut [u32], mut x: u32) -> u32 {
    while uf[x as usize] != x {
        uf[x as usize] = uf[uf[x as usize] as usize];
        x = uf[x as usize];
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fam(parents: &[u32], children: &[u32]) -> FamilyIn {
        FamilyIn { parents: parents.to_vec(), children: children.to_vec() }
    }

    #[test]
    fn source_is_the_originals_graph() {
        let s = dot_source(3, &[fam(&[0, 1], &[2])]);
        assert!(s.contains("\"p2\"->\"f0\""));
        assert!(s.contains("\"f0\"->\"p0\""));
        assert!(s.contains("\"f0\"->\"p1\""));
        assert!(s.contains("ranksep=1"));
    }

    #[test]
    fn lays_out_with_dot_when_present() {
        let Some(dot) = find_dot() else { return }; // dot is optional; nothing to test without it
        let fams = vec![fam(&[0, 1], &[2, 3]), fam(&[2, 4], &[5])];
        let tb = vec![i64::MAX; 7];
        let l = layout_with_dot(7, &tb, &fams, &Options::default(), &dot, &mut |_: Progress| true).unwrap().unwrap();
        // person 6 is in no family: last
        assert_eq!(*l.rows.last().unwrap(), 6);
        assert_eq!(l.rows.len(), 7);
        assert_eq!(l.cols.len(), 2);
        // parents sit in an earlier generation than their children, the family in between
        assert!(l.person_gen[0] < l.family_gen[0] && l.family_gen[0] < l.person_gen[2]);
        assert!(l.person_gen[2] < l.family_gen[1] && l.family_gen[1] < l.person_gen[5]);
    }

    #[test]
    fn the_port_lays_out_without_any_program() {
        let fams = vec![fam(&[0, 1], &[2, 3]), fam(&[2, 4], &[5])];
        let tb = vec![i64::MAX; 7];
        let l = layout_with_port(7, &tb, &fams, &Options::default(), &mut |_: Progress| true).unwrap().unwrap();
        assert_eq!(*l.rows.last().unwrap(), 6);
        assert_eq!((l.rows.len(), l.cols.len()), (7, 2));
        assert!(l.person_gen[0] < l.family_gen[0] && l.family_gen[0] < l.person_gen[2]);
        assert!(l.person_gen[2] < l.family_gen[1] && l.family_gen[1] < l.person_gen[5]);
    }

    #[test]
    fn the_port_agrees_with_dot_when_dot_is_present() {
        let Some(dot) = find_dot() else { return };
        // two trees, a long edge (grandparent -> grandchild through a skipped generation), a single person
        let fams = vec![fam(&[0, 1], &[2, 3]), fam(&[2, 4], &[5, 6]), fam(&[3, 7], &[8]), fam(&[9, 10], &[11]), fam(&[0], &[6])];
        let tb = vec![i64::MAX; 13];
        let a = layout_with_port(13, &tb, &fams, &Options::default(), &mut |_: Progress| true).unwrap().unwrap();
        let b = layout_with_dot(13, &tb, &fams, &Options::default(), &dot, &mut |_: Progress| true).unwrap().unwrap();
        assert_eq!(a.rows, b.rows);
        assert_eq!(a.cols, b.cols);
        assert_eq!(a.person_gen, b.person_gen);
    }

    #[test]
    fn a_family_without_parents_comes_first_in_the_diagonal() {
        let fams = vec![fam(&[], &[0, 1]), fam(&[0, 2], &[3])];
        let tb = vec![i64::MAX; 4];
        let l = layout_with_port(4, &tb, &fams, &Options::default(), &mut |_: Progress| true).unwrap().unwrap();
        assert!(l.family_gen[0] < l.person_gen[0], "the orphan family is before its children");
        assert!(l.person_gen[0] < l.family_gen[1] && l.family_gen[1] < l.person_gen[3]);
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        let r = layout_with_dot(2, &[0, 0], &[fam(&[0], &[1])], &Options::default(), Path::new("/no/such/dot"), &mut |_: Progress| true);
        assert!(r.is_err());
    }

    #[test]
    fn cancel_stops_the_work() {
        let Some(dot) = find_dot() else { return };
        // a graph big enough that dot takes a few polling intervals; the sink refuses at once
        let fams: Vec<FamilyIn> = (0..1500u32).map(|i| fam(&[i * 2, i * 2 + 1], &[(i * 2 + 2) % 3000, (i * 2 + 3) % 3000])).collect();
        let r = layout_with_dot(3000, &vec![0; 3000], &fams, &Options::default(), &dot, &mut |_: Progress| false);
        // either cancelled (None) or dot was quicker than one polling interval (Some): never an error
        assert!(matches!(r, Ok(None) | Ok(Some(_))));
    }
}
