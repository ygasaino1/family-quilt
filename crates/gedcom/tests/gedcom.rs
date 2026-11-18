use family_families::derive_families;
use family_families::testing::render;
use family_gedcom::gen::{generate_gedcom, GenOptions};
use family_gedcom::*;
use family_store::*;
use std::io::Cursor;

const CTX: AccessContext = AccessContext::Everything;

fn plan(text: &str) -> ImportPlan {
    import_gedcom(Cursor::new(text.as_bytes()), &Limits::default()).unwrap_or_else(|e| panic!("{e}"))
}
fn err(text: &str) -> GedError {
    import_gedcom(Cursor::new(text.as_bytes().to_vec()), &Limits::default()).unwrap_err()
}
fn codes(p: &ImportPlan) -> Vec<&'static str> {
    p.warnings.iter().map(|w| w.code).collect()
}
fn kind_of(p: &ImportPlan, parent: &str, child: &str) -> LinkKind {
    let idx = |x: &str| p.people.iter().position(|q| q.xref == x).unwrap();
    p.links.iter().find(|l| l.rel_type == RelType::ParentChild && l.a == idx(parent) && l.b == idx(child)).unwrap_or_else(|| panic!("no link {parent}>{child}")).kind.unwrap()
}

const SAMPLE: &str = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
1 CHAR UTF-8
0 @I1@ INDI
1 NAME Ali /Karimi/
1 SEX M
1 BIRT
2 DATE 12 APR 1950
2 PLAC Tehran
1 FAMS @F1@
0 @I2@ INDI
1 NAME Mina /Habibi/
2 GIVN Mina
2 SURN Habibi
1 SEX F
1 FAMS @F1@
0 @I3@ INDI
1 NAME Sara /Karimi/
1 SEX F
1 FAMC @F1@
1 FAMC @F2@
2 PEDI adopted
0 @I4@ INDI
1 NAME Dara /Moore/
1 FAMS @F2@
1 DEAT Y
0 @I5@ INDI
1 NAME Eve /Moore/
1 FAMS @F2@
0 @I6@ INDI
1 NAME Foster /Kid/
1 FAMC @F2@
2 PEDI foster
0 @I7@ INDI
1 NAME Seal /Kid/
1 FAMC @F2@
2 PEDI sealing
0 @I8@ INDI
1 NAME Weird /Kid/
1 FAMC @F2@
2 PEDI mystery
0 @I9@ INDI
1 NAME Other /Kid/
1 FAMC @F2@
2 PEDI other
3 PHRASE step
0 @F1@ FAM
1 HUSB @I1@
1 WIFE @I2@
1 CHIL @I3@
1 MARR
2 DATE 1975
0 @F2@ FAM
1 HUSB @I4@
1 WIFE @I5@
1 CHIL @I3@
1 DIV Y
0 TRLR
";

#[test]
fn pedi_values_become_kinds_and_multiple_famc_become_multiple_parent_sets() {
    let p = plan(SAMPLE);
    assert_eq!(p.people.len(), 9);
    assert_eq!(p.gedcom_version.as_deref(), Some("5.5.1"));
    // Sara is a child in two families: birth in F1, adopted in F2 (the original kept only the last one)
    assert_eq!(kind_of(&p, "I1", "I3"), LinkKind::Birth);
    assert_eq!(kind_of(&p, "I2", "I3"), LinkKind::Birth);
    assert_eq!(kind_of(&p, "I4", "I3"), LinkKind::Adopted);
    assert_eq!(kind_of(&p, "I5", "I3"), LinkKind::Adopted);
    assert_eq!(kind_of(&p, "I4", "I6"), LinkKind::Foster);
    assert_eq!(kind_of(&p, "I4", "I7"), LinkKind::Sealing);
    assert_eq!(kind_of(&p, "I4", "I8"), LinkKind::Unknown, "unknown PEDI value");
    assert_eq!(kind_of(&p, "I4", "I9"), LinkKind::Step, "PEDI OTHER + PHRASE step (7.0 style)");
    assert!(codes(&p).contains(&"unknown_pedi"));
    // partnership states: F1 has MARR -> married; F2 has DIV -> divorced
    let states: Vec<_> = p.links.iter().filter(|l| l.rel_type == RelType::Partnership).map(|l| (l.subtype.unwrap(), l.start_date.map(|d| d.to_string()))).collect();
    assert_eq!(states, vec![(PartnershipState::Married, Some("1975".into())), (PartnershipState::Divorced, None)]);
    // person details
    let ali = &p.people[0].person;
    assert_eq!(ali.sex, Sex::Male);
    assert_eq!(ali.birth.unwrap().to_string(), "1950-04-12");
    assert_eq!(ali.birth_place.as_deref(), Some("Tehran"));
    assert_eq!(ali.names[0].given.as_deref(), Some("Ali"));
    assert_eq!(ali.names[0].family.as_deref(), Some("Karimi"));
    assert_eq!(p.people[3].person.is_living, Some(false), "DEAT Y means not living");
}

#[test]
fn applying_the_plan_gives_the_expected_columns() {
    let p = plan(SAMPLE);
    let mut s = Store::open_in_memory().unwrap();
    let report = p.apply(&mut s, &CTX, &Origin::human()).unwrap();
    assert_eq!((report.person_ids.len(), report.links_created), (9, p.links.len()));
    let g = s.load_graph(&CTX, GraphFilter::default()).unwrap();
    let lines = render(&derive_families(&g), &g);
    // Sara: filled in her birth column, outlined in the adoptive one
    assert!(lines.contains(&"{Ali Karimi,Mina Habibi} M | Sara Karimi".to_string()), "{lines:#?}");
    assert!(lines.iter().any(|l| l.starts_with("{Dara Moore,Eve Moore} D | Sara Karimi*adopted, Foster Kid*foster")), "{lines:#?}");
}

#[test]
fn syntax_errors_name_the_line() {
    let bad = |t: &str| match err(t) {
        GedError::Syntax { line, message } => (line, message),
        other => panic!("expected a syntax error, got {other:?}"),
    };
    assert_eq!(bad("0 HEAD\n1 GEDC\nhello world\n0 TRLR\n").0, 3);
    assert_eq!(bad("0 HEAD\n2 VERS 5\n0 TRLR\n").0, 2, "level jumps");
    assert_eq!(bad("0 HEAD\n1 CHAR UTF-8\n0 @I1 INDI\n").0, 3, "bad record id");
    assert_eq!(bad("0 HEAD\n1 \n").0, 2, "missing tag");
    let mut bytes = b"0 HEAD\n1 NOTE caf".to_vec();
    bytes.extend([0xE9, b'\n']); // Latin-1 e-acute: not UTF-8
    match import_gedcom(Cursor::new(bytes), &Limits::default()).unwrap_err() {
        GedError::Syntax { line, message } => {
            assert_eq!(line, 2);
            assert!(message.contains("UTF-8"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(import_gedcom(Cursor::new(vec![0xFF, 0xFE, b'0', 0]), &Limits::default()), Err(GedError::Syntax { line: 1, .. })));
    assert!(matches!(err("not a gedcom file\n"), GedError::NotGedcom | GedError::Syntax { .. }));
    assert!(matches!(err("0 @I1@ INDI\n"), GedError::NotGedcom));
    assert!(matches!(err(""), GedError::NotGedcom));
    let long = format!("0 HEAD\n1 NOTE {}\n", "x".repeat(70_000));
    assert!(matches!(err(&long), GedError::Syntax { line: 2, .. }));
    let tiny = Limits { max_bytes: 20, max_line_bytes: 1000 };
    assert!(matches!(import_gedcom(Cursor::new(SAMPLE.as_bytes()), &tiny), Err(GedError::TooBig { .. })));
    assert_eq!(err("0 HEAD\n1 \n").code(), "gedcom_syntax");
}

#[test]
fn line_endings_bom_conc_cont_and_escapes() {
    let text = "\u{feff}0 HEAD\r\n0 @I1@ INDI\r\n1 NAME A /B/\r\n1 NOTE first\r\n2 CONT second\r\n2 CONC  and more\r\n1 NOTE mail@@example\r\n0 TRLR\r\n";
    let p = plan(text);
    let note = p.people[0].person.note.clone().unwrap();
    assert!(note.contains("first\nsecond and more"), "{note:?}");
    assert!(note.contains("mail@example"));
    // old Mac files end lines with CR only
    let cr = "0 HEAD\r0 @I1@ INDI\r1 NAME A /B/\r0 TRLR\r";
    assert_eq!(plan(cr).people.len(), 1);
    // blank lines and unknown tags are fine
    assert_eq!(plan("0 HEAD\n\n0 @S1@ SOUR\n1 TITL x\n0 @I1@ INDI\n1 NAME A /B/\n1 _CUSTOM 1\n2 _SUB 2\n0 TRLR\n").people.len(), 1);
}

#[test]
fn soft_problems_become_warnings_not_failures() {
    let p = plan(
        "0 HEAD\n1 CHAR ANSEL\n\
         0 @I1@ INDI\n1 NAME A /A/\n1 BIRT\n2 DATE @#DJULIAN@ 1 JAN 1700\n1 SEX Q\n1 FAMC @F1@\n\
         0 @I2@ INDI\n1 NAME B /B/\n1 FAMC @F2@\n\
         0 @I3@ INDI\n1 FAMS @F1@\n1 FAMS @F2@\n\
         0 @I1@ INDI\n1 NAME dup /dup/\n\
         0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I9@\n1 CHIL @I1@\n1 CHIL @I2@\n\
         0 @F2@ FAM\n1 HUSB @I2@\n1 CHIL @I1@\n1 CHIL @I2@\n",
    );
    let c = codes(&p);
    for want in ["charset", "unsupported_date", "bad_sex", "no_name", "duplicate_id", "unknown_reference", "self_parent", "cycle_skipped", "missing_trlr"] {
        assert!(c.contains(&want), "missing warning {want}: {c:?}");
    }
    assert!(p.people[0].person.note.as_deref().unwrap().contains("@#DJULIAN@ 1 JAN 1700"), "original date text is kept");
    assert!(p.people[0].person.birth.is_none());
    assert_eq!(p.people[2].person.names[0].full_name.as_deref(), Some("(unnamed)"));
    // and the plan still applies cleanly: the bad links were dropped
    let mut s = Store::open_in_memory().unwrap();
    p.apply(&mut s, &CTX, &Origin::human()).unwrap();
    // every warning carries a line number except the end-of-file one
    assert!(p.warnings.iter().filter(|w| w.code != "missing_trlr").all(|w| w.line > 0));
}

#[test]
fn duplicate_links_and_partnerships_are_not_repeated() {
    let p = plan(
        "0 HEAD\n0 @I1@ INDI\n1 NAME A /A/\n0 @I2@ INDI\n1 NAME B /B/\n0 @I3@ INDI\n1 NAME C /C/\n1 FAMC @F1@\n1 FAMC @F2@\n\
         0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n0 @F2@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n0 TRLR\n",
    );
    assert_eq!(p.links.iter().filter(|l| l.rel_type == RelType::Partnership).count(), 1);
    assert_eq!(p.links.iter().filter(|l| l.rel_type == RelType::ParentChild).count(), 2);
    assert!(codes(&p).contains(&"duplicate_link") && codes(&p).contains(&"duplicate_partnership"));
}

#[test]
fn date_ranges_and_other_dates_keep_their_information() {
    let p = plan("0 HEAD\n0 @I1@ INDI\n1 NAME A /A/\n1 BIRT\n2 DATE BET 1880 AND 1890\n1 DEAT\n2 DATE AFT 1950\n0 TRLR\n");
    let a = &p.people[0].person;
    assert_eq!(a.birth.unwrap().to_string(), "about 1880");
    assert!(a.note.as_deref().unwrap().contains("BET 1880 AND 1890"));
    assert_eq!(a.death.unwrap().to_string(), "after 1950");
    assert_eq!(a.is_living, Some(false));
}

// ───────────────────────── round trip ─────────────────────────

fn load(text: &str) -> Store {
    let p = plan(text);
    let mut s = Store::open_in_memory().unwrap();
    p.apply(&mut s, &CTX, &Origin::human()).unwrap();
    s
}

/// A tree built directly in the store with everything GEDCOM cannot say natively.
fn rich_store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    let h = Origin::human();
    let mk = |s: &mut Store, name: NewName, sex: Sex, birth: Option<&str>| {
        s.create_person(
            &CTX,
            NewPerson {
                sex,
                birth: birth.map(|b| FuzzyDate::parse(b, Qualifier::About).unwrap()),
                birth_place: birth.map(|_| "Tehran".into()),
                names: vec![name],
                note: Some("line one\nline two with @ sign".into()),
                ..Default::default()
            },
            &h,
        )
        .unwrap()
        .id
    };
    let a = mk(&mut s, NewName::new("Ali", "Karimi"), Sex::Male, Some("1950-04"));
    let b = mk(&mut s, NewName::full("مینا حبیبی"), Sex::Female, Some("1955"));
    let c = mk(&mut s, NewName::new("Step", "Dad"), Sex::Male, None);
    let d = mk(&mut s, NewName::new("Kid", "One"), Sex::Unknown, Some("1980"));
    let e = mk(&mut s, NewName::new("Kid", "Two"), Sex::Other, None);
    let f = mk(&mut s, NewName::new("Third", "Parent"), Sex::Female, None);
    let g = mk(&mut s, NewName::new("Guard", "Ian"), Sex::Female, None);
    let x = mk(&mut s, NewName::new("Ex", "Wife"), Sex::Female, None);
    s.add_name(&CTX, &d, NewName { name_type: Some(NameType::Alias), full_name: Some("Kiddo".into()), ..Default::default() }, false, "human").unwrap();
    let mut m = NewRelationship::partnership(&a, &b, PartnershipState::Married);
    m.start_date = Some(PartialDate::parse("1975-06-01").unwrap());
    m.place = Some("Tehran".into());
    s.add_relationship(&CTX, m, &h).unwrap();
    let mut u = NewRelationship::partnership(&a, &x, PartnershipState::Ended);
    u.start_date = Some(PartialDate::parse("1990").unwrap());
    u.end_date = Some(PartialDate::parse("1995").unwrap());
    u.note = Some("short".into());
    s.add_relationship(&CTX, u, &h).unwrap();
    s.add_relationship(&CTX, NewRelationship::partnership(&b, &c, PartnershipState::Unmarried), &h).unwrap();
    s.add_relationship(&CTX, NewRelationship::partnership(&c, &g, PartnershipState::Divorced), &h).unwrap();
    let pc = |s: &mut Store, p: &str, ch: &str, k: LinkKind| {
        s.add_relationship(&CTX, NewRelationship::parent_child(p, ch, k), &h).unwrap();
    };
    pc(&mut s, &a, &d, LinkKind::Birth);
    pc(&mut s, &b, &d, LinkKind::Birth);
    pc(&mut s, &c, &d, LinkKind::Step); // mixed kinds in one column {B,C} would need partnership b+c: merge case
    pc(&mut s, &a, &e, LinkKind::Adopted);
    pc(&mut s, &b, &e, LinkKind::Foster);
    pc(&mut s, &f, &e, LinkKind::Guardian);
    pc(&mut s, &g, &e, LinkKind::Unknown);
    s.update_person(&CTX, &x, PersonPatch { death: Field::Set(FuzzyDate::exact("2001-02-03").unwrap()), death_place: Field::Set("Paris".into()), ..Default::default() }, "human").unwrap();
    s
}

fn semantic(s: &Store) -> Vec<String> {
    let g = s.load_graph(&CTX, GraphFilter::default()).unwrap();
    let mut v = render(&derive_families(&g), &g);
    v.sort();
    v
}

#[test]
fn export_import_export_is_stable_in_both_versions() {
    for version in [GedcomVersion::V551, GedcomVersion::V70] {
        let s1 = rich_store();
        let t1 = export_gedcom(&s1, &CTX, version, false).unwrap();
        let p = plan(&t1);
        assert!(p.warnings.is_empty(), "{version:?}: {:#?}", p.warnings);
        let s2 = load(&t1);
        let t2 = export_gedcom(&s2, &CTX, version, false).unwrap();
        assert_eq!(t1, t2, "{version:?}: second export differs from the first");
        assert_eq!(semantic(&s1), semantic(&s2), "{version:?}: families differ after the round trip");
        // third time too
        assert_eq!(t2, export_gedcom(&load(&t2), &CTX, version, false).unwrap());
    }
}

#[test]
fn export_writes_pedi_and_keeps_birth_and_adoptive_families_apart() {
    let mut s = Store::open_in_memory().unwrap();
    let h = Origin::human();
    let mut mk = |n: &str, sex| s.create_person(&CTX, NewPerson { sex, names: vec![NewName::full(n)], ..Default::default() }, &h).unwrap().id;
    let (a, b, c, d, x) = (mk("A", Sex::Male), mk("B", Sex::Female), mk("C", Sex::Male), mk("D", Sex::Female), mk("X", Sex::Female));
    for (p, k) in [(&a, LinkKind::Birth), (&b, LinkKind::Birth), (&c, LinkKind::Adopted), (&d, LinkKind::Adopted)] {
        s.add_relationship(&CTX, NewRelationship::parent_child(p, &x, k), &h).unwrap();
    }
    let t = export_gedcom(&s, &CTX, GedcomVersion::V551, false).unwrap();
    assert_eq!(t.matches(" FAM\n").count(), 2);
    assert_eq!(t.matches("1 FAMC").count(), 2, "the child has one FAMC per family:\n{t}");
    assert_eq!(t.matches("2 PEDI adopted").count(), 1);
    assert!(t.starts_with("0 HEAD\n") && t.ends_with("0 TRLR\n"));
    assert!(t.contains("2 VERS 5.5.1") && t.contains("1 CHAR UTF-8"));
    let t7 = export_gedcom(&s, &CTX, GedcomVersion::V70, false).unwrap();
    assert!(t7.contains("2 VERS 7.0") && !t7.contains("CHAR UTF-8"));
    // step / guardian / unknown: note in 5.5.1, PEDI OTHER + PHRASE in 7.0
    let y = s.create_person(&CTX, NewPerson { names: vec![NewName::full("Y")], ..Default::default() }, &h).unwrap().id;
    s.add_relationship(&CTX, NewRelationship::parent_child(&a, &y, LinkKind::Step), &h).unwrap();
    let t = export_gedcom(&s, &CTX, GedcomVersion::V551, false).unwrap();
    assert!(t.contains("2 NOTE family-quilt:kind=step"));
    let t7 = export_gedcom(&s, &CTX, GedcomVersion::V70, false).unwrap();
    assert!(t7.contains("2 PEDI OTHER\n3 PHRASE step"));
}

#[test]
fn pending_data_is_not_exported_unless_asked() {
    let mut s = Store::open_in_memory().unwrap();
    let pid = s.insert_proposal(&CTX, "x", "y", "agent:t").unwrap();
    let o = Origin::agent("t", &pid, None);
    s.create_person(&CTX, NewPerson { names: vec![NewName::full("Pending Person")], ..Default::default() }, &o).unwrap();
    s.create_person(&CTX, NewPerson { names: vec![NewName::full("Real Person")], ..Default::default() }, &Origin::human()).unwrap();
    let t = export_gedcom(&s, &CTX, GedcomVersion::V551, false).unwrap();
    assert!(t.contains("Real Person") && !t.contains("Pending Person"));
    assert!(export_gedcom(&s, &CTX, GedcomVersion::V551, true).unwrap().contains("Pending Person"));
}

#[test]
fn import_can_be_applied_as_a_pending_proposal() {
    let p = plan(SAMPLE);
    let mut s = Store::open_in_memory().unwrap();
    let pid = s.insert_proposal(&CTX, "GEDCOM import", "file: sample.ged", "human").unwrap();
    let o = Origin { created_by: "human".into(), status: Status::Pending, proposal_id: Some(pid), agent_note: None };
    p.apply(&mut s, &CTX, &o).unwrap();
    assert_eq!(s.load_graph(&CTX, GraphFilter::default()).unwrap().people.len(), 0, "nothing is verified yet");
    assert_eq!(s.load_graph(&CTX, GraphFilter { include_pending: true }).unwrap().people.len(), 9);
}

// ───────────────────────── generator ─────────────────────────

#[test]
fn generator_is_deterministic_and_imports_cleanly() {
    let o = GenOptions { people: 1500, seed: 7, ..Default::default() };
    let t = generate_gedcom(&o);
    assert_eq!(t, generate_gedcom(&o));
    assert_ne!(t, generate_gedcom(&GenOptions { seed: 8, ..o.clone() }));
    let p = plan(&t);
    assert!(p.people.len() >= 1500, "{}", p.people.len());
    let bad: Vec<_> = p.warnings.iter().filter(|w| !matches!(w.code, "duplicate_link")).collect();
    assert!(bad.is_empty(), "{bad:#?}");
    let kinds = |k: LinkKind| p.links.iter().filter(|l| l.kind == Some(k)).count();
    assert!(kinds(LinkKind::Adopted) > 5 && kinds(LinkKind::Foster) > 2 && kinds(LinkKind::Birth) > 500);
    assert!(p.links.iter().any(|l| l.subtype == Some(PartnershipState::Divorced)), "remarriages present");
    assert!(t.contains("علی") || t.contains("سارا") || t.contains("مینا") || t.contains("رضا"), "some Persian names present");
    // a person in two families (adoption with known birth family)
    let mut s = Store::open_in_memory().unwrap();
    p.apply(&mut s, &CTX, &Origin::human()).unwrap();
    let g = s.load_graph(&CTX, GraphFilter::default()).unwrap();
    let d = derive_families(&g);
    let mut in_cols = std::collections::HashMap::new();
    for f in &d.families {
        for c in &f.children {
            *in_cols.entry(c.child).or_insert(0) += 1;
        }
    }
    assert!(in_cols.values().any(|n| *n >= 2), "some child appears in two columns");
    assert!(d.families.iter().any(|f| f.children.iter().any(|c| c.outlined)));
    // and it survives a full round trip
    let t1 = export_gedcom(&s, &CTX, GedcomVersion::V551, false).unwrap();
    let s2 = load(&t1);
    assert_eq!(t1, export_gedcom(&s2, &CTX, GedcomVersion::V551, false).unwrap());
    assert_eq!(semantic(&s), semantic(&s2));
}

#[test]
fn ten_thousand_people_import_apply_and_derive_quickly_enough() {
    let t = generate_gedcom(&GenOptions { people: 10_000, seed: 3, ..Default::default() });
    let t0 = std::time::Instant::now();
    let p = plan(&t);
    let t_parse = t0.elapsed();
    let mut s = Store::open_in_memory().unwrap();
    let t1 = std::time::Instant::now();
    p.apply(&mut s, &CTX, &Origin::human()).unwrap();
    let t_apply = t1.elapsed();
    let t2 = std::time::Instant::now();
    let g = s.load_graph(&CTX, GraphFilter::default()).unwrap();
    let d = derive_families(&g);
    let t_derive = t2.elapsed();
    eprintln!("10k people: {} people, {} links, {} columns; parse {t_parse:?}, apply {t_apply:?}, load+derive {t_derive:?}", g.people.len(), g.links.len(), d.families.len());
    assert!(g.people.len() >= 10_000);
    assert!(t_parse.as_secs() < 20 && t_apply.as_secs() < 60 && t_derive.as_secs() < 20, "debug build limits");
}

#[test]
fn original_sample_files_if_present() {
    // The sample .ged files of the original GeneaQuilts are not shipped (unknown licence). Put copies
    // in crates/gedcom/tests/data/ (git-ignored) to run this check locally.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("no sample files in {} (skipped)", dir.display());
        return;
    };
    for e in rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("ged"))) {
        let f = std::fs::File::open(e.path()).unwrap();
        match import_gedcom(std::io::BufReader::new(f), &Limits::default()) {
            Ok(p) => {
                let mut s = Store::open_in_memory().unwrap();
                p.apply(&mut s, &CTX, &Origin::human()).unwrap();
                let g = s.load_graph(&CTX, GraphFilter::default()).unwrap();
                let d = derive_families(&g);
                eprintln!("{}: {} people, {} columns, {} warnings", e.path().display(), g.people.len(), d.families.len(), p.warnings.len());
            }
            Err(err) => eprintln!("{}: {err}", e.path().display()),
        }
    }
}
