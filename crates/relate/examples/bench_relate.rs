//! `cargo run --release -p family-relate --example bench_relate -- [people] [pairs]`
//! Generates a synthetic tree, then times random pairs (related and unrelated).
use family_gedcom::gen::{generate_gedcom, GenOptions};
use family_gedcom::{import_gedcom, Limits};
use family_relate::*;
use family_store::{Graph, GraphLink, GraphPerson, Status};
use std::time::Instant;

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (n, pairs) = (args.first().copied().unwrap_or(100_000), args.get(1).copied().unwrap_or(2000));
    let t = Instant::now();
    let text = generate_gedcom(&GenOptions { people: n, seed: 7, ..Default::default() });
    let plan = import_gedcom(text.as_bytes(), &Limits::default()).unwrap();
    let people = plan
        .people
        .iter()
        .enumerate()
        .map(|(i, p)| GraphPerson { id: format!("p{i}"), display_name: format!("Person {i}"), sex: p.person.sex, is_living: true, birth: None, death: None, status: Status::Verified })
        .collect();
    let links = plan
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| GraphLink { id: format!("l{i}"), rel_type: l.rel_type, a: l.a as u32, b: l.b as u32, kind: l.kind, subtype: l.subtype, start_date: None, end_date: None, pending: false })
        .collect();
    let g = Graph::from_parts(people, links);
    println!("{} people, {} links; generate+parse {:?}", g.people.len(), g.links.len(), t.elapsed());
    let t = Instant::now();
    let rg = RelGraph::new(&g, false);
    println!("adjacency built in {:?}", t.elapsed());

    let mut s = 0x2545F4914F6CDD1Du64;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let np = g.people.len() as u64;
    let mut times = Vec::new();
    let (mut blood, mut conn, mut none) = (0, 0, 0);
    for _ in 0..pairs {
        let (a, b) = ((next() % np) as u32, (next() % np) as u32);
        let t = Instant::now();
        let r = rg.find(a, b, &FindOptions::default());
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        match r.kind {
            RelationKind::Blood | RelationKind::Same => blood += 1,
            RelationKind::Connection => conn += 1,
            RelationKind::None => none += 1,
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| times[((times.len() as f64 * p) as usize).min(times.len() - 1)];
    println!("{pairs} random pairs: blood {blood}, connection {conn}, none {none}");
    println!("ms: p50 {:.3}  p95 {:.3}  p99 {:.3}  max {:.3}", q(0.5), q(0.95), q(0.99), times[times.len() - 1]);
}
