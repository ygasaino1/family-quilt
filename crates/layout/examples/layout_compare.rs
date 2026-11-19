//! Gate E tool (development only): scores our layout and the Graphviz `dot` reference with the
//! same scoring function. `dot` is NOT used by the library.
//!   cargo run --release -p family-layout --example layout_compare -- file.ged [more.ged ...]
use family_families::derive_families;
use family_gedcom::{import_gedcom, Limits};
use family_layout::*;
use family_store::{Graph, GraphLink, GraphPerson, Status};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

pub fn load(path: &str) -> Graph {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let bytes = match String::from_utf8(bytes.clone()) {
        Ok(_) => bytes,
        Err(_) => bytes.iter().map(|&b| b as char).collect::<String>().into_bytes(),
    }; // old 8-bit files: read as Latin-1
    let plan = import_gedcom(&bytes[..], &Limits::default()).unwrap_or_else(|e| panic!("{path}: {e}"));
    let people = plan
        .people
        .iter()
        .enumerate()
        .map(|(i, p)| GraphPerson { id: format!("p{i:07}"), display_name: format!("P{i}"), sex: p.person.sex, is_living: true, birth: p.person.birth, death: None, status: Status::Verified })
        .collect();
    let links = plan
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| GraphLink { id: format!("l{i}"), rel_type: l.rel_type, a: l.a as u32, b: l.b as u32, kind: l.kind, subtype: l.subtype, start_date: None, end_date: None, pending: false })
        .collect();
    Graph::from_parts(people, links)
}

/// rows/cols of the Graphviz reference. `component_major` = false reproduces the original
/// (sort each layer by (tree, x)); true puts each tree's blocks together (the stronger reference).
fn dot_reference(fams: &[FamilyIn], people: usize, ours: &Layout, component_major: bool) -> (Vec<u32>, Vec<u32>) {
    let mut s = String::from("digraph g { ranksep=1; node [shape=point,width=0,height=0,label=\"\"]; edge [style=invis];\n");
    for (j, f) in fams.iter().enumerate() {
        for p in &f.parents {
            s.push_str(&format!("p{p} -> f{j};\n"));
        }
        for c in &f.children {
            s.push_str(&format!("f{j} -> p{c};\n"));
        }
    }
    s.push_str("}\n");
    let mut child = Command::new("dot").arg("-Tplain").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("dot not installed");
    child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let mut nodes: Vec<(char, u32, f64, f64)> = Vec::new();
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() >= 4 && t[0] == "node" {
            let kind = t[1].chars().next().unwrap();
            nodes.push((kind, t[1][1..].parse().unwrap(), t[2].parse().unwrap(), t[3].parse().unwrap()));
        }
    }
    let mut ys: Vec<i64> = nodes.iter().map(|n| (n.3 * 1000.0) as i64).collect();
    ys.sort_unstable();
    ys.dedup();
    ys.reverse(); // top rank first
    let layer = |y: f64| ys.iter().position(|&v| v == (y * 1000.0) as i64).unwrap() as u32;
    {
        let mut pl = vec![0u32; people];
        let mut fl = vec![0u32; fams.len()];
        for &(k, id, _, y) in &nodes {
            if k == 'p' {
                pl[id as usize] = layer(y)
            } else {
                fl[id as usize] = layer(y)
            }
        }
        let tot: u64 = fams
            .iter()
            .enumerate()
            .map(|(j, f)| f.parents.iter().map(|&p| fl[j].abs_diff(pl[p as usize]) as u64).sum::<u64>() + f.children.iter().map(|&c| fl[j].abs_diff(pl[c as usize]) as u64).sum::<u64>())
            .sum();
        let ours_tot: u64 = fams
            .iter()
            .enumerate()
            .map(|(j, f)| {
                let fl = ours.family_gen[j];
                f.parents.iter().chain(&f.children).map(|&p| ours.person_gen[p as usize].abs_diff(fl) as u64).sum::<u64>()
            })
            .sum();
        eprintln!("layers: dot {} (total edge length {}), ours {} (total edge length {})", ys.len(), tot, ours.person_gen.iter().chain(&ours.family_gen).max().map_or(0, |m| m + 1), ours_tot);
    }
    let tree = |kind: char, id: u32| if kind == 'p' { ours.person_tree[id as usize] } else { ours.family_tree[id as usize] };
    let mut rows: Vec<(u32, u32, i64, u32)> = Vec::new();
    let mut cols: Vec<(u32, u32, i64, u32)> = Vec::new();
    for &(k, id, x, y) in &nodes {
        let (t, l) = (tree(k, id), layer(y));
        let key = if component_major { (t, l, (x * 1000.0) as i64, id) } else { (l, t, (x * 1000.0) as i64, id) };
        if k == 'p' {
            rows.push(key)
        } else {
            cols.push(key)
        }
    }
    rows.sort();
    cols.sort();
    let mut r: Vec<u32> = rows.iter().map(|k| k.3).collect();
    let mut seen = vec![false; people];
    for &p in &r {
        seen[p as usize] = true;
    }
    for p in 0..people as u32 {
        if !seen[p as usize] {
            r.push(p);
        }
    }
    (r, cols.iter().map(|k| k.3).collect())
}

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    println!(
        "{:<28} {:>6} {:>5} | {:>20} {:>7} | {:>20} | {:>20} | {:>20} {:>5} | {:>20}",
        "file", "people", "fams", "ours tree-by-tree", "ms", "dot per-tree", "dot original", "ours layer-by-layer", "", "(vs dot original)"
    );
    for path in files {
        let g = load(&path);
        let d = derive_families(&g);
        let fams = input_from_derived(&d);
        let t = Instant::now();
        let ours = layout_graph(&g, &d);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let cells = cells_of(&fams);
        let score = |rows: &[u32], cols: &[u32]| quilt_score(&family_layout::invert_pub(rows), &family_layout::invert_pub(cols), &cells);
        let (ra, ca) = dot_reference(&fams, g.people.len(), &ours, false);
        let (rb, cb) = dot_reference(&fams, g.people.len(), &ours, true);
        let (sa, sb) = (score(&ra, &ca), score(&rb, &cb));
        let verdict = |s: &Score| if ours.score.no_worse_than(s) { "ok" } else { "WORSE" };
        let layer = layout_graph_with(&g, &d, &Options { mode: Mode::LayerByLayer(Align::Top) });
        let lv = if layer.score.no_worse_than(&sa) { "ok" } else { "WORSE" };
        let name = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().to_string();
        println!(
            "{:<28} {:>6} {:>5} | {:>9}/{:<10} {:>7.0} | {:>9}/{:<10} | {:>9}/{:<10} | {:>9}/{:<10} {:<5}",
            name.chars().take(27).collect::<String>(),
            g.people.len(),
            fams.len(),
            ours.score.crossings,
            ours.score.row_line_length,
            ms,
            sb.crossings,
            sb.row_line_length,
            sa.crossings,
            sa.row_line_length,
            layer.score.crossings,
            layer.score.row_line_length,
            lv
        );
        let _ = verdict;
        let _ = HashMap::<u32, u32>::new();
    }
}
