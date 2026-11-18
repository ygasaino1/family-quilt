use family_families::testing::*;
use family_families::{derive_families, Warning};

#[test]
fn table_driven_cases() {
    for case in CASES {
        let (got, d, g) = run(case);
        let want: Vec<String> = case.expect.iter().map(|s| s.to_string()).collect();
        assert_eq!(got, want, "case '{}'", case.name);
        assert_eq!(d.warnings.len(), case.expect_warnings, "warnings in '{}'", case.name);
        let unlinked: Vec<String> = d.unlinked.iter().map(|i| g.person(*i).display_name.clone()).collect();
        assert_eq!(unlinked, case.expect_unlinked, "unlinked in '{}'", case.name);
    }
}

#[test]
fn three_parent_warning_names_the_parents() {
    let case = CASES.iter().find(|c| c.name == "three parents").unwrap();
    let (_, d, _) = run(case);
    assert_eq!(d.warnings, vec![Warning::MoreThanTwoParents { parents: vec![0, 1, 2] }]);
}

/// Tiny deterministic generator (no extra dependency).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

const KINDS: [&str; 7] = ["birth", "birth", "birth", "adopted", "foster", "step", "sealing"];

/// Random acyclic trees (parents always have a smaller index than children).
fn random_case(seed: u64) -> (Vec<String>, Vec<String>) {
    let mut r = Lcg(seed);
    let n = 6 + r.next(10) as usize;
    let names: Vec<String> = (0..n).map(|i| format!("P{i:02}")).collect();
    let mut links = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for c in 1..n {
        for _ in 0..r.next(4) {
            let p = r.next(c as u64) as usize;
            if seen.insert(format!("{p}>{c}")) {
                links.push(format!("{}>{}:{}", names[p], names[c], KINDS[r.next(7) as usize]));
            }
        }
    }
    for _ in 0..r.next(5) {
        let (a, b) = (r.next(n as u64) as usize, r.next(n as u64) as usize);
        if a != b && seen.insert(format!("{}+{}", a.min(b), a.max(b))) {
            links.push(format!("{}+{}:{}", names[a], names[b], ["m", "u", "d", "e", "?"][r.next(5) as usize]));
        }
    }
    (names, links)
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

#[test]
fn invariants_hold_on_random_trees() {
    for seed in 0..400 {
        let (names, links) = random_case(seed);
        let g = build_graph(&refs(&names), &refs(&links));
        let d = derive_families(&g);

        // every parent_child link is in exactly one cell; every partnership in exactly one column
        let mut link_hits = std::collections::HashMap::new();
        for f in &d.families {
            for c in &f.children {
                for l in &c.links {
                    *link_hits.entry(l.link_id.clone()).or_insert(0) += 1;
                    assert!(f.parents.contains(&l.parent), "seed {seed}: link parent is a parent of its column");
                    assert_eq!(c.outlined, c.links.iter().any(|l| l.kind != family_store::LinkKind::Birth));
                }
            }
            if let Some(p) = &f.partnership {
                *link_hits.entry(p.link_id.clone()).or_insert(0) += 1;
                assert_eq!(f.parents.len(), 2);
            }
            assert!(f.parents.windows(2).all(|w| w[0] < w[1]), "parents sorted and unique");
            assert!(!f.parents.is_empty());
        }
        for l in &g.links {
            assert_eq!(link_hits.get(&l.id), Some(&1), "seed {seed}: link {} placed exactly once", l.id);
        }
        // keys are unique
        let mut keys: Vec<_> = d.families.iter().map(|f| f.parents.clone()).collect();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "columns have unique parent sets");
        // a child is never twice in one column
        for f in &d.families {
            let mut ch: Vec<_> = f.children.iter().map(|c| c.child).collect();
            ch.dedup();
            assert_eq!(ch.len(), f.children.len());
        }
        // order of the input links does not change the result
        let mut rev = links.clone();
        rev.reverse();
        let g2 = build_graph(&refs(&names), &refs(&rev));
        assert_eq!(render(&derive_families(&g2), &g2), render(&d, &g), "seed {seed}: input order must not matter");
    }
}

#[test]
fn derive_runs_fast_on_a_big_tree() {
    // 20k people in a simple chain-of-couples shape
    let n = 20_000usize;
    let names: Vec<String> = (0..n).map(|i| format!("P{i:05}")).collect();
    let mut links = Vec::new();
    for c in 2..n {
        links.push(format!("{}>{}", names[(c - 2) / 2 * 2 / 2 * 2 / 2], names[c]));
    }
    let g = build_graph(&refs(&names), &refs(&links));
    let t = std::time::Instant::now();
    let d = derive_families(&g);
    assert!(!d.families.is_empty());
    assert!(t.elapsed().as_millis() < 2000, "took {:?}", t.elapsed());
}
