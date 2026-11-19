use family_families::derive_families;
use std::time::Instant;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let bytes: Vec<u8> = match String::from_utf8(bytes.clone()) {
        Ok(_) => bytes,
        Err(_) => bytes.iter().map(|&b| b as char).collect::<String>().into_bytes(),
    };
    let plan = family_gedcom::import_gedcom(&bytes[..], &family_gedcom::Limits::default()).unwrap();
    use family_store::*;
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
    let d = derive_families(&g);
    let fams = family_layout::input_from_derived(&d);
    let t = Instant::now();
    let r = family_layout::rank::rank(g.people.len(), &fams);
    println!("rank {:?} (forced {})", t.elapsed(), r.forced);
    let t = Instant::now();
    let l = family_layout::layout_graph(&g, &d);
    println!("whole layout {:?}: {}", t.elapsed(), l.score);
}
