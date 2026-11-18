use family_families::testing::build_graph;
use family_relate::*;
use family_store::{Graph, Sex};

/// People are "Name" (unknown sex), "Name:M" or "Name:F".
fn graph(people: &[&str], links: &[&str]) -> Graph {
    let names: Vec<&str> = people.iter().map(|p| p.split(':').next().unwrap()).collect();
    let mut g = build_graph(&names, links);
    for (i, p) in people.iter().enumerate() {
        g.people[i].sex = match p.split(':').nth(1) {
            Some("M") => Sex::Male,
            Some("F") => Sex::Female,
            _ => Sex::Unknown,
        };
    }
    g
}

fn rel(g: &Graph, a: &str, b: &str) -> Relation {
    let rg = RelGraph::new(g, false);
    let ia = g.people.iter().position(|p| p.display_name == a).unwrap() as u32;
    let ib = g.people.iter().position(|p| p.display_name == b).unwrap() as u32;
    rg.find(ia, ib, &FindOptions::default())
}

fn text(r: &Relation) -> String {
    r.label.as_ref().map(|l| l.text.clone()).unwrap_or_else(|| "<none>".into())
}

#[test]
fn blood_names_table() {
    let people = ["Gp1:M", "Gp2:F", "Uncle:M", "Dad:M", "Mum:F", "Me:M", "Sis:F", "Cuz:M", "CuzKid:F", "Aunt2:F"];
    let links = ["Gp1+Gp2:m", "Gp1>Dad", "Gp2>Dad", "Gp1>Uncle", "Gp2>Uncle", "Dad+Mum:m", "Dad>Me", "Mum>Me", "Dad>Sis", "Mum>Sis", "Uncle>Cuz", "Cuz>CuzKid", "Uncle>Aunt2"];
    let g = graph(&people, &links);
    let table: &[(&str, &str, &str, &str)] = &[
        ("Me", "Sis", "brother", "sister"),
        ("Me", "Dad", "son", "father"),
        ("Dad", "Me", "father", "son"),
        ("Me", "Gp1", "grandson", "grandfather"),
        ("Me", "Uncle", "nephew", "uncle"),
        ("Uncle", "Sis", "uncle", "niece"),
        ("Me", "Cuz", "1st cousin", "1st cousin"),
        ("Me", "CuzKid", "1st cousin once removed", "1st cousin once removed"),
        ("CuzKid", "Me", "1st cousin once removed", "1st cousin once removed"),
        ("Cuz", "Aunt2", "half-brother", "half-sister"),
    ];
    for (a, b, la, lb) in table {
        let r = rel(&g, a, b);
        assert_eq!(r.kind, RelationKind::Blood, "{a}->{b}");
        assert!(r.by_blood);
        assert_eq!(text(&r), *la, "{a}->{b}");
        assert_eq!(r.reverse_label.as_ref().unwrap().text, *lb, "{a}->{b} reverse");
    }
    let r = rel(&g, "Me", "Cuz");
    assert_eq!(r.generations, Some((2, 2)));
    assert_eq!(r.common_ancestors.len(), 2, "full cousins share both grandparents");
    assert_eq!(r.path.as_ref().unwrap().steps.len(), 4);
    assert_eq!(r.path.as_ref().unwrap().nodes.len(), 5);
}

#[test]
fn half_and_great_and_second_cousins() {
    // Half-siblings: share Dad only. Second cousins: great-grandparents Gg1+Gg2.
    let people = ["Dad:M", "Mum:F", "Mum2:F", "A:M", "B:F", "Gg1:M", "Gg2:F", "G1:M", "G2:F", "P1:M", "P2:F", "C1:M", "C2:F"];
    let links = ["Dad>A", "Mum>A", "Dad>B", "Mum2>B", "Gg1+Gg2:m", "Gg1>G1", "Gg2>G1", "Gg1>G2", "Gg2>G2", "G1>P1", "G2>P2", "P1>C1", "P2>C2"];
    let g = graph(&people, &links);
    let r = rel(&g, "A", "B");
    assert_eq!(text(&r), "half-brother");
    assert!(r.label.as_ref().unwrap().half);
    assert_eq!(r.common_ancestors.len(), 1);
    assert_eq!(text(&rel(&g, "C1", "C2")), "2nd cousin");
    assert_eq!(text(&rel(&g, "C1", "Gg1")), "great-grandson");
    assert_eq!(text(&rel(&g, "Gg2", "C2")), "great-grandmother");
    assert_eq!(text(&rel(&g, "C1", "G2")), "great-nephew");
    assert_eq!(text(&rel(&g, "C1", "C1")), "same person");
    assert_eq!(rel(&g, "C1", "C1").kind, RelationKind::Same);
}

#[test]
fn adoption_is_not_blood() {
    // Dad+Mum have a birth child Bio and an adopted child Ado.
    let people = ["Dad:M", "Mum:F", "Bio:M", "Ado:F", "AdoKid:M"];
    let links = ["Dad+Mum:m", "Dad>Bio", "Mum>Bio", "Dad>Ado:adopted", "Mum>Ado:adopted", "Ado>AdoKid"];
    let g = graph(&people, &links);
    let r = rel(&g, "Ado", "Dad");
    assert_eq!((r.kind, r.by_blood), (RelationKind::Connection, false));
    assert_eq!(text(&r), "adopted daughter");
    assert_eq!(r.reverse_label.as_ref().unwrap().text, "adoptive father");
    assert!(r.has_non_birth && r.path.as_ref().unwrap().steps[0].non_birth());
    let r = rel(&g, "Ado", "Bio");
    assert_eq!(r.kind, RelationKind::Connection);
    assert_eq!(text(&r), "adoptive sister");
    assert_eq!(r.reverse_label.as_ref().unwrap().text, "adoptive brother");
    // Ado's own (birth) child is blood to Ado, but only a connection to Bio.
    assert_eq!(rel(&g, "Ado", "AdoKid").kind, RelationKind::Blood);
    let r = rel(&g, "AdoKid", "Bio");
    assert_eq!(r.kind, RelationKind::Connection, "adoptive nephew is not a blood nephew");
    assert!(r.chain.as_ref().unwrap().contains("adopted child of"));
}

#[test]
fn foster_guardian_unknown_sealing_and_step() {
    let people = ["P:M", "Q:F", "FosterKid:F", "Ward:M", "Mystery:M", "Sealed:F", "Own:M", "StepKid:F"];
    let links = ["P>FosterKid:foster", "P>Ward:guardian", "P>Mystery:unknown", "P>Sealed:sealing", "P>Own", "Q>StepKid", "P+Q:m"];
    let g = graph(&people, &links);
    let t = |a: &str, b: &str| {
        let r = rel(&g, a, b);
        assert_eq!(r.kind, RelationKind::Connection, "{a}->{b}");
        (text(&r), r.reverse_label.unwrap().text)
    };
    assert_eq!(t("FosterKid", "P"), ("foster daughter".into(), "foster father".into()));
    assert_eq!(t("Ward", "P"), ("ward".into(), "guardian".into()));
    assert_eq!(t("Mystery", "P").1, "parent (nature not recorded)");
    assert_eq!(t("Sealed", "P").1, "sealed father");
    // Unknown and sealing are not blood: Own and Mystery are not siblings by blood.
    let r = rel(&g, "Own", "Mystery");
    assert_eq!(r.kind, RelationKind::Connection);
    assert!(r.label.is_none(), "an unknown-kind sibling link has no name, only a chain");
    assert!(r.chain.as_ref().unwrap().contains("nature not recorded"));
    // Step relatives derived from the partnership.
    assert_eq!(t("StepKid", "P"), ("stepdaughter".into(), "stepfather".into()));
    assert_eq!(rel(&g, "Own", "Q").label.as_ref().unwrap().text, "stepson");
    assert_eq!(rel(&g, "Own", "StepKid").label.as_ref().unwrap().text, "stepbrother");
}

#[test]
fn in_laws_partners_and_ended() {
    let people = ["Hus:M", "Wife:F", "HisDad:M", "HerDad:M", "HerSis:F", "HerSisHus:M", "Kid:M", "KidWife:F"];
    let links = ["Hus+Wife:m", "HisDad>Hus", "HerDad>Wife", "HerDad>HerSis", "HerSis+HerSisHus:m", "Hus>Kid", "Wife>Kid", "Kid+KidWife:m"];
    let g = graph(&people, &links);
    assert_eq!(text(&rel(&g, "Hus", "Wife")), "spouse/partner");
    assert_eq!(text(&rel(&g, "Hus", "HerDad")), "son-in-law");
    assert_eq!(text(&rel(&g, "HerDad", "Hus")), "father-in-law");
    assert_eq!(text(&rel(&g, "Wife", "HisDad")), "daughter-in-law");
    assert_eq!(text(&rel(&g, "Hus", "HerSis")), "brother-in-law");
    assert_eq!(text(&rel(&g, "Hus", "KidWife")), "father-in-law");
    assert_eq!(text(&rel(&g, "KidWife", "Hus")), "daughter-in-law");
    let r = rel(&g, "HisDad", "HerDad");
    assert_eq!(r.kind, RelationKind::Connection);
    assert!(r.label.is_none());
    assert_eq!(r.chain.unwrap(), "HisDad → parent of → Hus → partner of → Wife → child of → HerDad");

    // An ended partnership costs 0.5 more.
    let g2 = graph(&["A", "B"], &["A+B:d"]);
    let r = rel(&g2, "A", "B");
    assert_eq!(r.cost, Some(2.0));
    assert_eq!(r.chain.unwrap(), "A → former partner of → B");
    let g3 = graph(&["A", "B"], &["A+B:m"]);
    assert_eq!(rel(&g3, "A", "B").cost, Some(1.5));
}

#[test]
fn birth_path_wins_ties_and_blood_beats_adoption() {
    // C is a birth child of P and also adopted by P (duplicate pair is impossible in the store,
    // so use a second parent): C born to P1, adopted by P2; P1 and P2 are siblings.
    let people = ["G", "G2", "P1", "P2", "C"];
    let links = ["G>P1", "G2>P1", "G>P2", "G2>P2", "P1>C", "P2>C:adopted"];
    let g = graph(&people, &links);
    let r = rel(&g, "C", "P2");
    assert_eq!(r.kind, RelationKind::Blood, "C is P2's niece/nephew by blood via P1");
    assert_eq!(text(&r), "niece/nephew");
    let rg = RelGraph::new(&g, false);
    let (ic, ip2) = (4, 3);
    let all = rg.find(ic, ip2, &FindOptions { all: true, ..Default::default() });
    let via = all.via_any.expect("via_any with all=true");
    assert_eq!(via.cost, 1.0);
    assert!(via.has_non_birth, "the one-step route is the adoptive link");
    assert_eq!(via.label.unwrap().text, "adopted child");
}

#[test]
fn unrelated_and_cost_limit() {
    let g = graph(&["A", "B", "C", "D"], &["A>B", "C>D"]);
    let r = rel(&g, "A", "D");
    assert_eq!(r.kind, RelationKind::None);
    assert!(r.path.is_none() && !r.waiting_for_review);
    // A long chain beyond max_cost is not found.
    let names: Vec<String> = (0..30).map(|i| format!("N{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let links: Vec<String> = (0..29).map(|i| format!("N{i}>N{}", i + 1)).collect();
    let lrefs: Vec<&str> = links.iter().map(String::as_str).collect();
    let g = graph(&refs, &lrefs);
    assert_eq!(rel(&g, "N0", "N29").kind, RelationKind::Blood);
    let rg = RelGraph::new(&g, false);
    let r = rg.find(0, 29, &FindOptions { all: true, max_cost: 10.0, ..Default::default() });
    assert!(r.via_any.is_none());
    assert_eq!(text(&r), "27th great-grandparent");
}

#[test]
fn pending_data_is_ignored_unless_requested() {
    // Link B>C is pending: A..C is unrelated in the verified view.
    let g = graph(&["A", "B", "C"], &["A>B", "B>C!"]);
    let rg = RelGraph::new(&g, false);
    assert_eq!(rg.find(0, 2, &FindOptions::default()).kind, RelationKind::None);
    assert_eq!(rg.find(0, 1, &FindOptions::default()).kind, RelationKind::Blood);
    let rgp = RelGraph::new(&g, true);
    let r = rgp.find(0, 2, &FindOptions { include_pending: true, ..Default::default() });
    assert_eq!(r.kind, RelationKind::Blood);
    assert!(r.includes_pending && r.waiting_for_review);
    let ok = rgp.find(0, 1, &FindOptions { include_pending: true, ..Default::default() });
    assert!(!ok.waiting_for_review, "a verified path stays unflagged");

    // A pending person: the verified view says so instead of "unrelated".
    let mut g = graph(&["A", "B"], &["A>B"]);
    g.people[1].status = family_store::Status::Pending;
    let rg = RelGraph::new(&g, false);
    let r = rg.find(0, 1, &FindOptions::default());
    assert!(r.waiting_for_review && r.kind == RelationKind::None);
}

#[test]
fn pedigree_collapse_cousin_marriage() {
    // X and Y are first cousins who married; their child Z is related to Gp both ways.
    let people = ["Gp1:M", "Gp2:F", "S1", "S2", "X:M", "Y:F", "Z"];
    let links = ["Gp1>S1", "Gp2>S1", "Gp1>S2", "Gp2>S2", "S1>X", "S2>Y", "X+Y:m", "X>Z", "Y>Z"];
    let g = graph(&people, &links);
    assert_eq!(text(&rel(&g, "X", "Y")), "1st cousin");
    assert_eq!(rel(&g, "X", "Y").kind, RelationKind::Blood, "cousins who married are still blood relatives");
    assert_eq!(text(&rel(&g, "Z", "Gp1")), "great-grandchild");
}

#[test]
fn route_is_deterministic_and_symmetric() {
    let g = graph(&["A", "B", "C", "D", "E", "F"], &["A+B:m", "B>C", "C+D:m", "D>E", "E+F:u", "A>F:step"]);
    let rg = RelGraph::new(&g, false);
    for a in 0..6 {
        for b in 0..6 {
            let x = rg.find(a, b, &FindOptions::default());
            let y = rg.find(b, a, &FindOptions::default());
            assert_eq!(x.kind, y.kind);
            assert_eq!(x.cost, y.cost, "{a}->{b}");
            assert_eq!(x.path.map(|p| p.nodes.len()), y.path.map(|p| p.nodes.len()));
        }
    }
}
