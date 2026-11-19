//! Stability probe: add / edit one person and measure how much the order of everyone else changes.
use family_layout::*;

fn lcg(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *s >> 33
}

/// Fraction of pairs (among items present in both orders) that swap relative order.
fn discordant(a: &[u32], b: &[u32], keep: impl Fn(u32) -> bool) -> (f64, usize) {
    let pos_b: std::collections::HashMap<u32, usize> = b.iter().enumerate().map(|(i, &x)| (x, i)).collect();
    let seq: Vec<usize> = a.iter().filter(|&&x| keep(x)).filter_map(|x| pos_b.get(x).copied()).collect();
    // count inversions (merge sort)
    fn inv(v: &mut Vec<usize>) -> u64 {
        if v.len() < 2 {
            return 0;
        }
        let mut r = v.split_off(v.len() / 2);
        let mut c = inv(v) + inv(&mut r);
        let (mut i, mut j) = (0, 0);
        let mut m = Vec::with_capacity(v.len() + r.len());
        while i < v.len() && j < r.len() {
            if v[i] <= r[j] {
                m.push(v[i]);
                i += 1
            } else {
                m.push(r[j]);
                j += 1;
                c += (v.len() - i) as u64
            }
        }
        m.extend_from_slice(&v[i..]);
        m.extend_from_slice(&r[j..]);
        *v = m;
        c
    }
    let n = seq.len();
    let mut s = seq;
    let inversions = inv(&mut s);
    (inversions as f64 / (n as f64 * (n as f64 - 1.0) / 2.0).max(1.0), n)
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(600);
    let text = family_gedcom::gen::generate_gedcom(&family_gedcom::gen::GenOptions { people: n, seed: 3, ..Default::default() });
    let plan = family_gedcom::import_gedcom(text.as_bytes(), &family_gedcom::Limits::default()).unwrap();
    let np = plan.people.len();
    let tb: Vec<i64> = plan.people.iter().map(|p| p.person.birth.map(|b| b.earliest_day() as i64).unwrap_or(i64::MAX)).collect();
    // families straight from the plan links (derive_families is covered elsewhere): group by child's parent set
    let mut graph_people = Vec::new();
    for (i, p) in plan.people.iter().enumerate() {
        graph_people.push(family_store::GraphPerson {
            id: format!("p{i:07}"),
            display_name: String::new(),
            sex: p.person.sex,
            is_living: true,
            birth: p.person.birth,
            death: None,
            status: family_store::Status::Verified,
        });
    }
    let links = plan
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| family_store::GraphLink {
            id: format!("l{i}"),
            rel_type: l.rel_type,
            a: l.a as u32,
            b: l.b as u32,
            kind: l.kind,
            subtype: l.subtype,
            start_date: None,
            end_date: None,
            pending: false,
        })
        .collect();
    let g = family_store::Graph::from_parts(graph_people, links);
    let d = family_families::derive_families(&g);
    let fams = input_from_derived(&d);
    let base = layout(np, &tb, &fams);
    let mut sizes = std::collections::HashMap::new();
    for &t in &base.person_tree {
        *sizes.entry(t).or_insert(0usize) += 1;
    }
    println!("{np} people, {} families, {} trees (largest {}); base score {}", fams.len(), sizes.len(), sizes.values().max().unwrap(), base.score);
    let mut seed = 99u64;
    let mut worst = (0f64, 0f64);
    let trials = 20;
    let mut sum = (0f64, 0f64);
    let mut hsum = (0f64, 0f64);
    let mut hworst = (0f64, 0f64);
    for t in 0..trials {
        let mut f2 = fams.clone();
        let mut tb2 = tb.clone();
        let mut people = np;
        let what;
        match t % 3 {
            0 => {
                // add a child to a random family
                let fi = lcg(&mut seed) as usize % f2.len();
                f2[fi].children.push(np as u32);
                tb2.push(i64::MAX);
                people += 1;
                what = "add child";
            }
            1 => {
                // add a new spouse for a random person (new single-pair family) with one new child
                let p = lcg(&mut seed) as u32 % np as u32;
                f2.push(FamilyIn { parents: vec![p, np as u32], children: vec![np as u32 + 1] });
                tb2.push(i64::MAX);
                tb2.push(i64::MAX);
                people += 2;
                what = "add spouse+child";
            }
            _ => {
                // edit one person's birth date (tie-break key)
                let p = lcg(&mut seed) as usize % np;
                tb2[p] = (lcg(&mut seed) % 20000) as i64;
                what = "edit birth date";
            }
        }
        let l2 = layout(people, &tb2, &f2);
        let hint = Hint {
            person_pos: (0..people).map(|i| base.row_pos().get(i).map(|_| base.row_pos()[i]).filter(|_| i < np)).collect(),
            family_pos: (0..f2.len()).map(|j| if j < fams.len() { Some(base.col_pos()[j]) } else { None }).collect(),
        };
        let l3 = layout_with_hint(people, &tb2, &f2, &hint);
        let (hr, _) = discordant(&base.rows, &l3.rows, |x| (x as usize) < np);
        let (hc, _) = discordant(&base.cols, &l3.cols, |x| (x as usize) < fams.len());
        hsum = (hsum.0 + hr, hsum.1 + hc);
        hworst = (hworst.0.max(hr), hworst.1.max(hc));
        let _ = (l3.score, l2.score);
        let (dr, _) = discordant(&base.rows, &l2.rows, |x| (x as usize) < np);
        let (dc, _) = discordant(&base.cols, &l2.cols, |x| (x as usize) < fams.len());
        worst = (worst.0.max(dr), worst.1.max(dc));
        sum = (sum.0 + dr, sum.1 + dc);
        println!("{what:<18} rows moved {:6.3}%  cols moved {:6.3}%", dr * 100.0, dc * 100.0);
    }
    println!(
        "with previous layout as hint: mean rows {:.3}% cols {:.3}%; worst rows {:.3}% cols {:.3}%",
        hsum.0 / trials as f64 * 100.0,
        hsum.1 / trials as f64 * 100.0,
        hworst.0 * 100.0,
        hworst.1 * 100.0
    );
    println!("without hint: mean rows {:.3}% cols {:.3}%; worst rows {:.3}% cols {:.3}%", sum.0 / trials as f64 * 100.0, sum.1 / trials as f64 * 100.0, worst.0 * 100.0, worst.1 * 100.0);
}
