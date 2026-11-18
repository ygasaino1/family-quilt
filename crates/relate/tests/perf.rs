//! p95 < 50 ms on a large synthetic tree (spec section 13, phase 3). The full 100k run is
//! `cargo run --release -p family-relate --example bench_relate`; this keeps a guard in `cargo test`.
use family_gedcom::gen::{generate_gedcom, GenOptions};
use family_gedcom::{import_gedcom, Limits};
use family_relate::*;
use family_store::{Graph, GraphLink, GraphPerson, Status};
use std::time::Instant;

#[test]
fn p95_under_50ms_on_a_large_tree() {
    let n = if cfg!(debug_assertions) { 30_000 } else { 100_000 };
    let plan = import_gedcom(generate_gedcom(&GenOptions { people: n, seed: 11, ..Default::default() }).as_bytes(), &Limits::default()).unwrap();
    let people = plan
        .people
        .iter()
        .enumerate()
        .map(|(i, p)| GraphPerson { id: format!("p{i}"), display_name: format!("P{i}"), sex: p.person.sex, is_living: true, birth: None, death: None, status: Status::Verified })
        .collect();
    let links = plan
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| GraphLink { id: format!("l{i}"), rel_type: l.rel_type, a: l.a as u32, b: l.b as u32, kind: l.kind, subtype: l.subtype, start_date: None, end_date: None, pending: false })
        .collect();
    let g = Graph::from_parts(people, links);
    let rg = RelGraph::new(&g, false);
    let mut s = 88172645463325252u64;
    let mut times = Vec::new();
    for _ in 0..500 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let (a, b) = ((s % g.people.len() as u64) as u32, ((s >> 20) % g.people.len() as u64) as u32);
        let t = Instant::now();
        let r = rg.find(a, b, &FindOptions { all: true, ..Default::default() });
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        // Same answer both ways round.
        let back = rg.find(b, a, &FindOptions::default());
        assert_eq!(r.kind, back.kind);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = times[(times.len() as f64 * 0.95) as usize];
    println!("p95 {p95:.2} ms on {} people", g.people.len());
    assert!(p95 < 50.0, "p95 {p95} ms");
}
