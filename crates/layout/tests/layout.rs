use family_families::derive_families;
use family_families::testing::build_graph;
use family_layout::score::{quilt_score, quilt_score_slow};
use family_layout::*;

fn lcg(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *s >> 33
}

fn run(people: &[&str], links: &[&str]) -> (Layout, Vec<FamilyIn>) {
    let g = build_graph(people, links);
    let d = derive_families(&g);
    let fams = input_from_derived(&d);
    (layout_graph(&g, &d), fams)
}

/// Properties every layout must have.
fn check(l: &Layout, people: usize, fams: &[FamilyIn]) {
    let mut seen = vec![false; people];
    assert_eq!(l.rows.len(), people);
    for &p in &l.rows {
        assert!(!seen[p as usize], "person {p} twice");
        seen[p as usize] = true;
    }
    let mut seen = vec![false; fams.len()];
    assert_eq!(l.cols.len(), fams.len());
    for &f in &l.cols {
        assert!(!seen[f as usize], "family {f} twice");
        seen[f as usize] = true;
    }
    if l.forced_nodes == 0 {
        for (j, f) in fams.iter().enumerate() {
            for &p in &f.parents {
                assert!(l.person_gen[p as usize] < l.family_gen[j], "parent above its column");
            }
            for &c in &f.children {
                assert!(l.family_gen[j] < l.person_gen[c as usize], "child below its column");
            }
        }
    }
    // Trees are contiguous: tree numbers never decrease along rows / columns, and within a tree rows go down by layer.
    for w in l.rows.windows(2) {
        let (a, b) = (w[0] as usize, w[1] as usize);
        assert!(l.person_tree[a] <= l.person_tree[b]);
        if l.person_tree[a] == l.person_tree[b] {
            assert!(l.person_gen[a] <= l.person_gen[b]);
        }
    }
    for w in l.cols.windows(2) {
        let (a, b) = (w[0] as usize, w[1] as usize);
        assert!(l.family_tree[a] <= l.family_tree[b]);
        if l.family_tree[a] == l.family_tree[b] {
            assert!(l.family_gen[a] <= l.family_gen[b]);
        }
    }
    // The stored score is the score of the stored order.
    assert_eq!(l.score, quilt_score(&l.row_pos(), &l.col_pos(), &cells_of(fams)));
}

#[test]
fn fast_score_equals_brute_force() {
    let mut s = 12345u64;
    for _ in 0..400 {
        let np = 2 + lcg(&mut s) as usize % 14;
        let nf = 1 + lcg(&mut s) as usize % 10;
        let perm = |n: usize, s: &mut u64| {
            let mut v: Vec<u32> = (0..n as u32).collect();
            for i in (1..n).rev() {
                v.swap(i, lcg(s) as usize % (i + 1));
            }
            v
        };
        let (rp, cp) = (perm(np, &mut s), perm(nf, &mut s));
        let mut cells = std::collections::BTreeSet::new();
        for _ in 0..(2 + lcg(&mut s) % 25) {
            cells.insert((lcg(&mut s) as u32 % np as u32, lcg(&mut s) as u32 % nf as u32));
        }
        let cells: Vec<_> = cells.into_iter().collect();
        assert_eq!(quilt_score(&rp, &cp, &cells), quilt_score_slow(&rp, &cp, &cells));
    }
}

#[test]
fn score_known_values() {
    // rows: p1=0, p2=1, p0=2, p3=3. cols: fA=0, fB=1, fC=2.
    // p0 and p2 are each in fA and fC, so their row lines run over column 1. fB holds p1 (row 0) and p3 (row 3),
    // so its column line covers rows 1 and 2: two crossings.
    let rows = [2u32, 0, 1, 3]; // position of p0,p1,p2,p3
    let cols = [0u32, 1, 2];
    let cells = [(0, 0), (0, 2), (1, 1), (3, 1), (2, 0), (2, 2)];
    let s = quilt_score(&rows, &cols, &cells);
    assert_eq!(s.crossings, 2);
    assert_eq!(s.row_line_length, 2 + 2); // p0 spans 2; p2 spans 2
}

#[test]
fn nuclear_family_is_one_block() {
    let (l, f) = run(&["Dad", "Mum", "A", "B"], &["Dad+Mum:m", "Dad>A", "Mum>A", "Dad>B", "Mum>B"]);
    check(&l, 4, &f);
    assert_eq!(l.score.crossings, 0);
    assert_eq!(l.person_gen[0], l.person_gen[1], "partners on one layer");
    assert!(l.person_gen[2] > l.person_gen[0]);
}

#[test]
fn hard_cases_hold_the_invariants() {
    // cousin marriage: X and Y are first cousins and have a child Z
    let (l, f) = run(&["Gp1", "Gp2", "S1", "S2", "X", "Y", "Z"], &["Gp1+Gp2:m", "Gp1>S1", "Gp2>S1", "Gp1>S2", "Gp2>S2", "S1>X", "S2>Y", "X+Y:m", "X>Z", "Y>Z"]);
    check(&l, 7, &f);
    assert_eq!(l.person_gen[4], l.person_gen[5], "the cousins who marry share a layer");
    // remarriage: A marries B, divorces, marries C; one child each
    let (l, f) = run(&["A", "B", "C", "K1", "K2"], &["A+B:d", "A+C:m", "A>K1", "B>K1", "A>K2", "C>K2"]);
    check(&l, 5, &f);
    assert_eq!(f.len(), 2);
    // many half-siblings: one father, five mothers
    let mut people = vec!["Dad".to_string()];
    let mut links = Vec::new();
    for i in 0..5 {
        people.push(format!("M{i}"));
        people.push(format!("K{i}"));
        links.push(format!("Dad+M{i}:u"));
        links.push(format!("Dad>K{i}"));
        links.push(format!("M{i}>K{i}"));
    }
    let (p, li): (Vec<&str>, Vec<&str>) = (people.iter().map(String::as_str).collect(), links.iter().map(String::as_str).collect());
    let (l, f) = run(&p, &li);
    check(&l, p.len(), &f);
    assert_eq!(f.len(), 5);
    assert_eq!(l.score.crossings, 0, "a fan of five couples can be drawn without crossings");
    // adoptive + birth families: child C has birth parents B1+B2 and adoptive parents A1+A2 (A1+A2 are one generation higher)
    let (l, f) = run(&["G", "A1", "A2", "B1", "B2", "C"], &["G>A1", "A1+A2:m", "B1+B2:u", "B1>C", "B2>C", "A1>C:adopted", "A2>C:adopted"]);
    check(&l, 6, &f);
    assert_eq!(f.len(), 3, "G>A1 column, A1+A2 column, B1+B2 column");
    assert!(l.person_gen[5] > l.person_gen[1], "child below the adoptive parents");
    assert!(l.person_gen[5] > l.person_gen[3], "child below the birth parents");
    // several unconnected trees (and one person alone)
    let (l, f) =
        run(&["A", "B", "K", "C", "D", "L", "M", "E", "Solo"], &["A+B:m", "A>K", "B>K", "C+D:m", "C>L", "D>L", "C>M", "D>M", "E>E2"].iter().filter(|s| !s.contains("E2")).copied().collect::<Vec<_>>());
    check(&l, 9, &f);
    let trees: std::collections::BTreeSet<u32> = l.person_tree.iter().copied().collect();
    assert_eq!(trees.len(), 4, "AB+K, CD+L+M, E, Solo");
    assert_eq!(l.score.crossings, 0);
    assert_eq!(l.person_tree[8], 3, "people alone go last");
    assert_eq!(l.person_tree[3], l.person_tree[6], "C and M are in one tree");
    assert_eq!(l.person_tree[3], 0, "the larger tree comes first");
}

#[test]
fn partners_who_would_form_a_cycle_do_not_break_the_layout() {
    // A+B are partners, and B's parent is A's child: impossible in life, possible in bad data.
    let (l, f) = run(&["A", "B", "C", "D"], &["A+B:m", "A>C", "C>D", "D>B"]);
    assert_eq!(f.len(), 4);
    check(&l, 4, &f); // permutations and score are still right; layer order is not promised when forced
}

fn generated(n: usize, seed: u64) -> (usize, Vec<i64>, Vec<FamilyIn>) {
    let text = family_gedcom::gen::generate_gedcom(&family_gedcom::gen::GenOptions { people: n, seed, ..Default::default() });
    let plan = family_gedcom::import_gedcom(text.as_bytes(), &family_gedcom::Limits::default()).unwrap();
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
    let tb = g.people.iter().map(|p| p.birth.map(|b| b.earliest_day() as i64).unwrap_or(i64::MAX)).collect();
    (g.people.len(), tb, input_from_derived(&d))
}

#[test]
fn generated_trees_hold_the_invariants_and_are_deterministic() {
    for seed in 1..4 {
        let (n, tb, fams) = generated(150, seed);
        let a = layout(n, &tb, &fams);
        check(&a, n, &fams);
        let b = layout(n, &tb, &fams);
        assert_eq!((a.rows.clone(), a.cols.clone(), a.person_gen.clone()), (b.rows, b.cols, b.person_gen), "same input, same output");
    }
}

/// Fraction of pairs (among items in both orders) whose relative order changed.
fn discordant(a: &[u32], b: &[u32], keep: impl Fn(u32) -> bool) -> f64 {
    let pos_b: std::collections::HashMap<u32, usize> = b.iter().enumerate().map(|(i, &x)| (x, i)).collect();
    let seq: Vec<usize> = a.iter().filter(|&&x| keep(x)).filter_map(|x| pos_b.get(x).copied()).collect();
    let n = seq.len();
    let mut inv = 0u64;
    for i in 0..n {
        for j in i + 1..n {
            if seq[i] > seq[j] {
                inv += 1;
            }
        }
    }
    inv as f64 / (n as f64 * (n as f64 - 1.0) / 2.0).max(1.0)
}

#[test]
fn one_edit_moves_little() {
    let (np, tb, fams) = generated(200, 8);
    let base = layout(np, &tb, &fams);
    let (rp, cp) = (base.row_pos(), base.col_pos());
    let mut s = 77u64;
    for t in 0..6 {
        let mut f2 = fams.clone();
        let mut tb2 = tb.clone();
        let mut people = np;
        match t % 3 {
            0 => {
                let fi = lcg(&mut s) as usize % f2.len();
                f2[fi].children.push(np as u32);
                tb2.push(i64::MAX);
                people += 1;
            }
            1 => {
                let p = lcg(&mut s) as u32 % np as u32;
                f2.push(FamilyIn { parents: vec![p, np as u32], children: vec![np as u32 + 1] });
                tb2.extend([i64::MAX, i64::MAX]);
                people += 2;
            }
            _ => {
                let p = lcg(&mut s) as usize % np;
                tb2[p] = (lcg(&mut s) % 20000) as i64;
            }
        }
        let hint = Hint { person_pos: (0..people).map(|i| (i < np).then(|| rp[i])).collect(), family_pos: (0..f2.len()).map(|j| (j < fams.len()).then(|| cp[j])).collect() };
        let warm = layout_with_hint(people, &tb2, &f2, &hint);
        check(&warm, people, &f2);
        assert!(discordant(&base.rows, &warm.rows, |x| (x as usize) < np) < 0.02, "with the previous layout as hint");
        assert!(discordant(&base.cols, &warm.cols, |x| (x as usize) < fams.len()) < 0.02);
        let cold = layout(people, &tb2, &f2);
        check(&cold, people, &f2);
        // without a hint the order is recomputed from scratch; it must still be mostly the same
        assert!(discordant(&base.rows, &cold.rows, |x| (x as usize) < np) < 0.10, "without hint");
    }
}

fn three_trees() -> (usize, Vec<i64>, Vec<FamilyIn>) {
    // Tree 0 (A..): 3 generations. Tree 1 (B..): 2 generations. Tree 2 (C..): 2 generations. P8 is alone.
    // people: 0 A1 1 A2 2 A3 3 A4 4 A5 | 5 B1 6 B2 7 B3 | 8 C1 9 C2 10 C3 | 11 Solo
    let f = |p: &[u32], c: &[u32]| FamilyIn { parents: p.to_vec(), children: c.to_vec() };
    let fams = vec![f(&[0, 1], &[2]), f(&[2, 3], &[4]), f(&[5, 6], &[7]), f(&[8, 9], &[10])];
    (12, vec![i64::MAX; 12], fams)
}

#[test]
fn layer_by_layer_puts_a_generation_in_one_band() {
    let (n, tb, fams) = three_trees();
    let tree = layout(n, &tb, &fams);
    let lay = layout_with_options(n, &tb, &fams, &Options { mode: Mode::LayerByLayer(Align::Top) }, None);
    // permutation, same trees
    assert_eq!(lay.rows.len(), n);
    assert_eq!(lay.cols.len(), fams.len());
    assert_eq!(lay.person_tree, tree.person_tree, "same trees in both modes");
    // rows sorted by (generation, tree); the person in no family is last
    let keys: Vec<(u32, u32)> = lay.rows.iter().take(n - 1).map(|&p| (lay.person_gen[p as usize], lay.person_tree[p as usize])).collect();
    assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
    assert_eq!(*lay.rows.last().unwrap(), 11);
    // a band: all generation-0 people come before any generation-1 person
    let first_gen1 = lay.rows.iter().position(|&p| lay.person_gen[p as usize] > 0).unwrap();
    assert!(lay.rows[..first_gen1].iter().all(|&p| lay.person_gen[p as usize] == 0));
    assert_eq!(first_gen1, 2 + 2 + 2, "A1 A2 B1 B2 C1 C2");
    // inside each tree the relative order is the same as in tree-by-tree mode
    for t in 0..3u32 {
        let pick = |l: &Layout| l.rows.iter().copied().filter(|&p| l.person_tree[p as usize] == t && p != 11).collect::<Vec<_>>();
        assert_eq!(pick(&lay), pick(&tree));
        let pickc = |l: &Layout| l.cols.iter().copied().filter(|&f| l.family_tree[f as usize] == t).collect::<Vec<_>>();
        assert_eq!(pickc(&lay), pickc(&tree));
    }
    // the layer-by-layer picture of unrelated trees has crossings; tree-by-tree has none
    assert_eq!(tree.score.crossings, 0);
    assert!(lay.score.crossings > 0);
    // and it is deterministic
    let again = layout_with_options(n, &tb, &fams, &Options { mode: Mode::LayerByLayer(Align::Top) }, None);
    assert_eq!((again.rows, again.cols), (lay.rows, lay.cols));
}

#[test]
fn birth_dates_line_up_generations_across_trees() {
    // Tree B is one century younger than tree A: its top generation should sit next to A's 4th generation.
    let day = |year: i64| ((year - 1970) as f64 * 365.2425) as i64;
    let f = |p: &[u32], c: &[u32]| FamilyIn { parents: p.to_vec(), children: c.to_vec() };
    // A: A0 -> A1 -> A2 -> A3 -> A4 -> A5 (a single line of couples, each generation 25 years later)
    let mut fams = Vec::new();
    let mut tb = Vec::new();
    // people 0..=5 = A0..A5 (one parent each), 6..=7 = B0 B1 (B0 parent of B1)
    for g in 0..6 {
        tb.push(day(1800 + 25 * g));
    }
    for g in 0..5 {
        fams.push(f(&[g], &[g + 1]));
    }
    tb.push(day(1900));
    tb.push(day(1925));
    fams.push(f(&[6], &[7]));
    let n = 8;
    let top = layout_with_options(n, &tb, &fams, &Options { mode: Mode::LayerByLayer(Align::Top) }, None);
    assert_eq!(top.person_gen[6], 0);
    let dated = layout_with_options(n, &tb, &fams, &Options { mode: Mode::LayerByLayer(Align::BirthDate { generation_years: 25 }) }, None);
    // B0 (born 1900) is four generations after A0 (born 1800): same band as A4 (born 1900)
    assert_eq!(dated.person_gen[6], dated.person_gen[4], "B0 sits in A4's band");
    assert_eq!(dated.person_gen[7], dated.person_gen[5]);
    let pos = dated.row_pos();
    assert!(pos[4].abs_diff(pos[6]) <= 1, "and the rows are next to each other");
    check_perm(&dated, n, &fams);
}

fn check_perm(l: &Layout, people: usize, fams: &[FamilyIn]) {
    let mut r = l.rows.clone();
    r.sort_unstable();
    assert_eq!(r, (0..people as u32).collect::<Vec<_>>());
    let mut c = l.cols.clone();
    c.sort_unstable();
    assert_eq!(c, (0..fams.len() as u32).collect::<Vec<_>>());
    assert_eq!(l.score, quilt_score(&l.row_pos(), &l.col_pos(), &cells_of(fams)));
}
