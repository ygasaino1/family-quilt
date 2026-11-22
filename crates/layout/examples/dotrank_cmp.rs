//! Developer tool: rank the nodes of a plain `"a"->"b";` file with the dot port and print `name rank`,
//! to compare with `dot -Tplain`. Nodes are numbered by first mention, edges in file order.
use std::collections::HashMap;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dotrank_cmp FILE.dot");
    let text = std::fs::read_to_string(path).expect("read");
    let mut id: HashMap<String, u32> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut edges: Vec<(u32, u32)> = Vec::new();
    let intern = |s: &str, id: &mut HashMap<String, u32>, names: &mut Vec<String>| -> u32 {
        *id.entry(s.to_string()).or_insert_with(|| {
            names.push(s.to_string());
            (names.len() - 1) as u32
        })
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some((a, b)) = line.split_once("->") {
            let a = a.trim().trim_matches('"').to_string();
            let b = b.trim().trim_end_matches(';').trim().trim_matches('"').to_string();
            let ia = intern(&a, &mut id, &mut names);
            let ib = intern(&b, &mut id, &mut names);
            edges.push((ia, ib));
        } else if line.starts_with('"') && line.ends_with(';') {
            intern(line.trim_end_matches(';').trim_matches('"'), &mut id, &mut names);
        }
    }
    let r = family_layout::dotrank::rank_nodes(names.len(), &edges);
    for (n, r) in names.iter().zip(r) {
        println!("{n} {r}");
    }
}
