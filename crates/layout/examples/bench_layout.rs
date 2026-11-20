//! `cargo run --release -p family-layout --example bench_layout -- [people]`
use family_families::derive_families;
use family_store::*;
use std::time::Instant;
fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(100_000);
    let text = family_gedcom::gen::generate_gedcom(&family_gedcom::gen::GenOptions { people: n, seed: 5, ..Default::default() });
    let plan = family_gedcom::import_gedcom(text.as_bytes(), &family_gedcom::Limits::default()).unwrap();
    let people = plan
        .people
        .iter()
        .enumerate()
        .map(|(i, p)| GraphPerson { id: format!("p{i:07}"), display_name: String::new(), sex: p.person.sex, is_living: true, birth: p.person.birth, death: None, status: Status::Verified })
        .collect();
    let links = plan
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| GraphLink { id: format!("l{i}"), rel_type: l.rel_type, a: l.a as u32, b: l.b as u32, kind: l.kind, subtype: l.subtype, start_date: None, end_date: None, pending: false })
        .collect();
    let g = Graph::from_parts(people, links);
    let t = Instant::now();
    let d = derive_families(&g);
    println!("{} people, {} columns; derive_families {:?}", g.people.len(), d.families.len(), t.elapsed());
    let tb: Vec<i64> = g.people.iter().map(|p| p.birth.map(|b| b.earliest_day() as i64).unwrap_or(i64::MAX)).collect();
    let t = Instant::now();
    let q =
        family_layout::layout_with_progress(g.people.len(), &tb, &family_layout::input_from_derived(&d), &Default::default(), None, family_layout::Effort::Quick, &mut |_: family_layout::Progress| {
            true
        })
        .unwrap();
    println!("quick layout {:?} (crossings {})", t.elapsed(), q.score.crossings);
    let t = Instant::now();
    let l = family_layout::layout_graph(&g, &d);
    let el = t.elapsed();
    let mut sizes: std::collections::HashMap<u32, usize> = Default::default();
    for &x in &l.person_tree {
        *sizes.entry(x).or_default() += 1;
    }
    println!("layout {:?}; {} trees (largest {}); layers {}; forced {}; {}", el, sizes.len(), sizes.values().max().unwrap(), l.person_gen.iter().max().unwrap() + 1, l.forced_nodes, l.score);
}
