mod common;
use common::*;
use family_store::*;

#[test]
fn create_and_read_person_with_fuzzy_dates_and_places() {
    let mut s = store();
    let p = s
        .create_person(
            &CTX,
            NewPerson {
                sex: Sex::Female,
                birth: Some(FuzzyDate::parse("1887-04", Qualifier::About).unwrap()),
                birth_place: Some("Tehran".into()),
                death: Some(FuzzyDate::exact("1960").unwrap()),
                names: vec![NewName::new("Sara", "Adams"), NewName { name_type: Some(NameType::Alias), full_name: Some("سارا آدمز".into()), script: Some("Arab".into()), ..Default::default() }],
                ..Default::default()
            },
            &Origin::human(),
        )
        .unwrap();
    assert_eq!(p.display_name, "Sara Adams");
    assert!(!p.is_living, "a death date implies not living");
    assert!(p.is_deceased());
    assert_eq!(p.birth.unwrap().to_string(), "about 1887-04");
    assert_eq!(p.birth_place.as_ref().unwrap().name, "Tehran");
    assert_eq!(p.names.len(), 2);
    assert!(p.names[0].is_primary && !p.names[1].is_primary);
    // same place text is reused, not duplicated
    let q = s.create_person(&CTX, NewPerson { birth_place: Some("  tehran ".into()), names: vec![NewName::full("X")], ..Default::default() }, &Origin::human()).unwrap();
    assert_eq!(q.birth_place.unwrap().id, p.birth_place.unwrap().id);
}

#[test]
fn invalid_people_are_rejected() {
    let mut s = store();
    let no_name = s.create_person(&CTX, NewPerson::default(), &Origin::human());
    assert!(matches!(no_name, Err(StoreError::Invalid { .. })));
    let blank = s.create_person(&CTX, NewPerson { names: vec![NewName::full("   ")], ..Default::default() }, &Origin::human());
    assert!(matches!(blank, Err(StoreError::Invalid { .. })));
    let contradiction =
        s.create_person(&CTX, NewPerson { is_living: Some(true), death: Some(FuzzyDate::exact("2000").unwrap()), names: vec![NewName::full("A")], ..Default::default() }, &Origin::human());
    assert!(matches!(contradiction, Err(StoreError::Conflict(_))));
    let pending_without_proposal = s.create_person(&CTX, NewPerson { names: vec![NewName::full("A")], ..Default::default() }, &Origin { status: Status::Pending, ..Origin::human() });
    assert!(matches!(pending_without_proposal, Err(StoreError::Invalid { .. })));
    assert_eq!(s.search_people(&CTX, &SearchQuery::default()).unwrap().len(), 0, "failed creates leave nothing behind");
}

#[test]
fn update_person_patches_and_records_history() {
    let mut s = store();
    let id = person(&mut s, "Sara", "Adams", Sex::Female);
    let p = s.update_person(&CTX, &id, PersonPatch { birth: Field::Set(FuzzyDate::exact("1958").unwrap()), birth_place: Field::Set("Tehran".into()), ..Default::default() }, "human").unwrap();
    assert_eq!(p.birth.unwrap().to_string(), "1958");
    let p = s.update_person(&CTX, &id, PersonPatch { death: Field::Set(FuzzyDate::exact("2020").unwrap()), ..Default::default() }, "human").unwrap();
    assert!(!p.is_living);
    // clearing the death date does not resurrect the person by itself, but an explicit flag may
    let p = s.update_person(&CTX, &id, PersonPatch { death: Field::Clear, is_living: Some(true), ..Default::default() }, "human").unwrap();
    assert!(p.is_living && p.death.is_none());
    let bad = s.update_person(&CTX, &id, PersonPatch { death: Field::Set(FuzzyDate::exact("2020").unwrap()), is_living: Some(true), ..Default::default() }, "human");
    assert!(matches!(bad, Err(StoreError::Conflict(_))));

    // no-op edit: no new revision, updated_at untouched
    let before = s.get_person(&CTX, &id).unwrap();
    let n = s.revisions_of(&CTX, "person", &id).unwrap().len();
    let after = s.update_person(&CTX, &id, PersonPatch::default(), "human").unwrap();
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(s.revisions_of(&CTX, "person", &id).unwrap().len(), n);

    let revs = s.revisions_of(&CTX, "person", &id).unwrap();
    assert_eq!(revs.len(), 4, "create + 3 real edits");
    assert!(revs[0].before.is_none());
    assert_eq!(revs[1].before.as_ref().unwrap()["birth_date"], serde_json::Value::Null);
    assert_eq!(revs[1].after.as_ref().unwrap()["birth_date"], "1958");
}

#[test]
fn names_primary_rules_and_search_in_any_script() {
    let mut s = store();
    let id = person(&mut s, "Ali", "Karimi", Sex::Male);
    let fa = s.add_name(&CTX, &id, NewName::full("علی کریمی"), false, "human").unwrap();
    assert!(!fa.is_primary);
    let found = |s: &Store, q: &str| s.search_people(&CTX, &SearchQuery { text: q.into(), ..Default::default() }).unwrap().len();
    assert_eq!(found(&s, "kari"), 1);
    assert_eq!(found(&s, "ALI KA"), 1);
    assert_eq!(found(&s, "علي کريمي"), 1, "Arabic yeh/kaf match Persian");
    assert_eq!(found(&s, "zzz"), 0);
    assert_eq!(found(&s, ""), 1, "empty query lists everyone");

    s.set_primary_name(&CTX, &fa.id, "human").unwrap();
    let p = s.get_person(&CTX, &id).unwrap();
    assert_eq!(p.display_name, "علی کریمی");
    assert_eq!(p.names.iter().filter(|n| n.is_primary).count(), 1);

    let upd = s.update_name(&CTX, &fa.id, NamePatch { given: Field::Set("عليرضا".into()), family: Field::Set("کریمی".into()), ..Default::default() }, "human").unwrap();
    assert_eq!(upd.full_name, "عليرضا کریمی");
    assert_eq!(found(&s, "علیرضا"), 1, "search key follows the edit");

    // removing the primary promotes the other; the last name cannot go
    s.remove_name(&CTX, &fa.id, "human").unwrap();
    let p = s.get_person(&CTX, &id).unwrap();
    assert_eq!(p.names.len(), 1);
    assert!(p.names[0].is_primary);
    assert!(matches!(s.remove_name(&CTX, &p.names[0].id, "human"), Err(StoreError::Conflict(_))));
}

#[test]
fn search_status_filter_limit_and_ranking() {
    let mut s = store();
    let _a = person(&mut s, "Anna", "Zed", Sex::Female);
    let _b = person(&mut s, "Joanna", "Zed", Sex::Female);
    let r = s.search_people(&CTX, &SearchQuery { text: "anna".into(), ..Default::default() }).unwrap();
    assert_eq!(r[0].display_name, "Anna Zed", "prefix match ranks first");
    assert_eq!(r.len(), 2);
    let r = s.search_people(&CTX, &SearchQuery { text: "zed".into(), limit: 1, ..Default::default() }).unwrap();
    assert_eq!(r.len(), 1);
    let r = s.search_people(&CTX, &SearchQuery { text: "zed".into(), status: Some(Status::Pending), ..Default::default() }).unwrap();
    assert!(r.is_empty());
    // wildcard characters are plain text
    assert!(s.search_people(&CTX, &SearchQuery { text: "%".into(), ..Default::default() }).unwrap().is_empty());
}

#[test]
fn links_all_kinds_and_defaults() {
    let mut s = store();
    let (m, f, c) = (person(&mut s, "M", "A", Sex::Female), person(&mut s, "F", "A", Sex::Male), person(&mut s, "C", "A", Sex::Female));
    for kind in LinkKind::ALL {
        let child = person(&mut s, "K", "B", Sex::Unknown);
        let r = parent(&mut s, &m, &child, *kind).unwrap();
        assert_eq!(r.kind, Some(*kind));
        assert_eq!(r.subtype, None);
    }
    let default_kind = s.add_relationship(&CTX, NewRelationship { kind: None, ..NewRelationship::parent_child(&f, &c, LinkKind::Birth) }, &Origin::human()).unwrap();
    assert_eq!(default_kind.kind, Some(LinkKind::Birth));
    let p = partner(&mut s, &m, &f, PartnershipState::Married).unwrap();
    assert_eq!(p.kind, None);
    assert_eq!(p.subtype, Some(PartnershipState::Married));
    assert!(LinkKind::Birth.is_blood());
    assert!(LinkKind::ALL.iter().filter(|k| k.is_blood()).count() == 1, "only birth is blood (unknown is not)");
}

#[test]
fn partnership_is_stored_once_in_either_order() {
    let mut s = store();
    let (a, b) = (person(&mut s, "A", "A", Sex::Male), person(&mut s, "B", "B", Sex::Female));
    let r1 = partner(&mut s, &a, &b, PartnershipState::Married).unwrap();
    assert!(r1.person_a < r1.person_b);
    assert!(matches!(partner(&mut s, &b, &a, PartnershipState::Unmarried), Err(StoreError::Duplicate { .. })));
}

#[test]
fn duplicates_self_links_and_wrong_fields_are_rejected() {
    let mut s = store();
    let (a, b) = (person(&mut s, "A", "A", Sex::Male), person(&mut s, "B", "B", Sex::Male));
    parent(&mut s, &a, &b, LinkKind::Birth).unwrap();
    assert!(matches!(parent(&mut s, &a, &b, LinkKind::Adopted), Err(StoreError::Duplicate { .. })), "one link per pair, even with another kind");
    assert!(matches!(parent(&mut s, &a, &a, LinkKind::Birth), Err(StoreError::Invalid { .. })));
    let mut bad = NewRelationship::parent_child(&a, &b, LinkKind::Birth);
    bad.subtype = Some(PartnershipState::Married);
    assert!(s.add_relationship(&CTX, bad, &Origin::human()).is_err());
    let mut bad = NewRelationship::partnership(&a, &b, PartnershipState::Married);
    bad.kind = Some(LinkKind::Adopted);
    assert!(s.add_relationship(&CTX, bad, &Origin::human()).is_err());
}

#[test]
fn cycles_are_rejected() {
    let mut s = store();
    let (g, p, c) = (person(&mut s, "G", "X", Sex::Male), person(&mut s, "P", "X", Sex::Male), person(&mut s, "C", "X", Sex::Male));
    parent(&mut s, &g, &p, LinkKind::Birth).unwrap();
    parent(&mut s, &p, &c, LinkKind::Adopted).unwrap();
    assert!(matches!(parent(&mut s, &c, &g, LinkKind::Birth), Err(StoreError::Cycle)), "great-grandchild as parent of grandparent");
    assert!(matches!(parent(&mut s, &p, &g, LinkKind::Step), Err(StoreError::Cycle)), "direct reversal, any kind");
    // a non-cycle diamond is fine
    let q = person(&mut s, "Q", "X", Sex::Female);
    parent(&mut s, &g, &q, LinkKind::Birth).unwrap();
    parent(&mut s, &q, &c, LinkKind::Birth).unwrap();
    // nothing was written by the rejected attempts
    assert_eq!(s.relationships_of(&CTX, &c).unwrap().len(), 2);
}

#[test]
fn links_need_live_people() {
    let mut s = store();
    let (a, b) = (person(&mut s, "A", "A", Sex::Male), person(&mut s, "B", "B", Sex::Male));
    assert!(matches!(parent(&mut s, &a, "00000000-0000-7000-8000-000000000000", LinkKind::Birth), Err(StoreError::NotFound { .. })));
    s.soft_delete_person(&CTX, &b, "human").unwrap();
    assert!(matches!(parent(&mut s, &a, &b, LinkKind::Birth), Err(StoreError::NotFound { .. })));
    assert!(matches!(s.get_person(&CTX, &b), Err(StoreError::NotFound { .. })));
}

#[test]
fn soft_delete_cascades_and_links_can_be_re_entered() {
    let mut s = store();
    let (a, b, c) = (person(&mut s, "A", "A", Sex::Male), person(&mut s, "B", "B", Sex::Male), person(&mut s, "C", "C", Sex::Male));
    let l1 = parent(&mut s, &a, &b, LinkKind::Birth).unwrap();
    parent(&mut s, &b, &c, LinkKind::Birth).unwrap();
    assert_eq!(s.soft_delete_person(&CTX, &b, "human").unwrap(), 2);
    assert!(s.relationships_of(&CTX, &a).unwrap().is_empty());
    assert!(matches!(s.get_relationship(&CTX, &l1.id), Err(StoreError::NotFound { .. })));
    // the rows are still there (never hard-deleted)
    let rows: i64 = s.revisions_of(&CTX, "person", &b).unwrap().len() as i64;
    assert_eq!(rows, 2);
    // removing and re-adding a link works
    let l = parent(&mut s, &a, &c, LinkKind::Foster).unwrap();
    s.remove_relationship(&CTX, &l.id, "human").unwrap();
    assert!(parent(&mut s, &a, &c, LinkKind::Birth).is_ok());
    assert!(s.search_people(&CTX, &SearchQuery { text: "B".into(), ..Default::default() }).unwrap().iter().all(|p| p.id != b));
}

#[test]
fn update_relationship_changes_kind_with_history() {
    let mut s = store();
    let (a, b) = (person(&mut s, "A", "A", Sex::Male), person(&mut s, "B", "B", Sex::Male));
    let l = parent(&mut s, &a, &b, LinkKind::Birth).unwrap();
    let u = s
        .update_relationship(
            &CTX,
            &l.id,
            RelationshipPatch { kind: Some(LinkKind::Adopted), note: Field::Set("adopted 1990".into()), start_date: Field::Set(PartialDate::parse("1990").unwrap()), ..Default::default() },
            "human",
        )
        .unwrap();
    assert_eq!(u.kind, Some(LinkKind::Adopted));
    assert_eq!(u.start_date.as_deref(), Some("1990"));
    assert!(s.update_relationship(&CTX, &l.id, RelationshipPatch { subtype: Some(PartnershipState::Married), ..Default::default() }, "human").is_err());
    let revs = s.revisions_of(&CTX, "relationship", &l.id).unwrap();
    assert_eq!(revs.len(), 2);
    assert_eq!(revs[1].before.as_ref().unwrap()["kind"], "birth");
    assert_eq!(revs[1].after.as_ref().unwrap()["kind"], "adopted");
}

#[test]
fn graph_snapshot_hides_pending_unless_asked() {
    let mut s = store();
    let a = person(&mut s, "A", "A", Sex::Male);
    let b = person(&mut s, "B", "B", Sex::Female);
    parent(&mut s, &a, &b, LinkKind::Birth).unwrap();
    // a pending proposal with a pending person linked to a verified one
    s.conn_for_tests().execute("INSERT INTO proposal(id, created_by, created_at) VALUES ('p1','agent:test','2026-01-01T00:00:00.000Z')", []).unwrap();
    let origin = Origin::agent("test", "p1", Some("from a chat"));
    let c = s.create_person(&CTX, NewPerson { names: vec![NewName::new("C", "C")], ..Default::default() }, &origin).unwrap();
    assert_eq!(c.status, Status::Pending);
    assert_eq!(c.created_by, "agent:test");
    s.add_relationship(&CTX, NewRelationship::parent_child(&b, &c.id, LinkKind::Adopted), &origin).unwrap();

    let g = s.load_graph(&CTX, GraphFilter::default()).unwrap();
    assert_eq!((g.people.len(), g.links.len()), (2, 1));
    assert!(!g.incomplete && !g.links[0].pending);
    assert_eq!(g.person(g.index_of(&a).unwrap()).display_name, "A A");

    let g = s.load_graph(&CTX, GraphFilter { include_pending: true }).unwrap();
    assert_eq!((g.people.len(), g.links.len()), (3, 2));
    assert_eq!(g.links.iter().filter(|l| l.pending).count(), 1, "the link to the pending person is pending, the verified one is not");
    let l = g.links.iter().find(|l| l.pending).unwrap();
    assert_eq!(l.kind, Some(LinkKind::Adopted));
}

#[test]
fn meta_settings_round_trip() {
    let mut s = store();
    assert_eq!(s.meta_get(&CTX, "title").unwrap(), None);
    s.meta_set(&CTX, "title", "My Family Tree").unwrap();
    s.meta_set(&CTX, "title", "Ours").unwrap();
    assert_eq!(s.meta_get(&CTX, "title").unwrap().as_deref(), Some("Ours"));
}

#[test]
fn every_table_has_vault_id_and_it_is_null() {
    let mut s = store();
    let (a, b) = (person(&mut s, "A", "A", Sex::Male), person(&mut s, "B", "B", Sex::Male));
    parent(&mut s, &a, &b, LinkKind::Birth).unwrap();
    let conn = s.conn_for_tests();
    let tables: Vec<String> =
        conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    assert!(tables.len() == 11, "{tables:?}");
    for t in tables.iter().filter(|t| *t != "meta") {
        let has: i64 = conn.query_row(&format!("SELECT count(*) FROM pragma_table_info('{t}') WHERE name = 'vault_id'"), [], |r| r.get(0)).unwrap();
        assert_eq!(has, 1, "{t} must have vault_id");
        let non_null: i64 = conn.query_row(&format!("SELECT count(*) FROM {t} WHERE vault_id IS NOT NULL"), [], |r| r.get(0)).unwrap();
        assert_eq!(non_null, 0, "{t} must only hold public (NULL) rows");
    }
    let fk: i64 = conn.prepare("PRAGMA foreign_key_check").unwrap().query_map([], |_| Ok(())).unwrap().count() as i64;
    assert_eq!(fk, 0);
}

#[test]
fn database_constraints_hold_even_if_the_store_layer_is_bypassed() {
    let s = store();
    let c = s.conn_for_tests();
    let now = "2026-01-01T00:00:00.000Z";
    assert!(c.execute("INSERT INTO person(id, sex, created_at, updated_at) VALUES ('x','Q',?1,?1)", [now]).is_err(), "bad sex");
    assert!(c.execute("INSERT INTO person(id, birth_date, created_at, updated_at) VALUES ('x','1887',?1,?1)", [now]).is_err(), "date without qualifier");
    assert!(c.execute("INSERT INTO person(id, birth_date, birth_qualifier, created_at, updated_at) VALUES ('x','18a7','exact',?1,?1)", [now]).is_err(), "malformed date");
    assert!(c.execute("INSERT INTO person(id, is_living, death_date, death_qualifier, created_at, updated_at) VALUES ('x',1,'2000','exact',?1,?1)", [now]).is_err(), "living with death date");
    c.execute("INSERT INTO person(id, created_at, updated_at) VALUES ('a',?1,?1)", [now]).unwrap();
    c.execute("INSERT INTO person(id, created_at, updated_at) VALUES ('b',?1,?1)", [now]).unwrap();
    assert!(c.execute("INSERT INTO relationship(id,type,person_a,person_b,kind,created_at,updated_at) VALUES ('r','parent_child','a','b','bogus',?1,?1)", [now]).is_err(), "bad kind");
    assert!(
        c.execute("INSERT INTO relationship(id,type,person_a,person_b,kind,subtype,created_at,updated_at) VALUES ('r','parent_child','a','b','birth','married',?1,?1)", [now]).is_err(),
        "subtype on parent_child"
    );
    assert!(
        c.execute("INSERT INTO relationship(id,type,person_a,person_b,subtype,created_at,updated_at) VALUES ('r','partnership','b','a','married',?1,?1)", [now]).is_err(),
        "partnership not in canonical order"
    );
    assert!(c.execute("INSERT INTO relationship(id,type,person_a,person_b,kind,created_at,updated_at) VALUES ('r','parent_child','a','a','birth',?1,?1)", [now]).is_err(), "self link");
    assert!(
        c.execute("INSERT INTO relationship(id,type,person_a,person_b,kind,created_at,updated_at) VALUES ('r','parent_child','a','zzz','birth',?1,?1)", [now]).is_err(),
        "missing person (foreign key)"
    );
}

#[test]
fn errors_have_stable_codes() {
    let e = StoreError::Cycle;
    assert_eq!(e.code(), "link_cycle");
    let e = StoreError::NotFound { kind: "person", id: "x".into() };
    assert_eq!(e.code(), "not_found");
    assert_eq!(e.params()["kind"], "person");
    assert_eq!(StoreError::PasswordRequired.code(), "locked");
}

#[test]
fn atomic_batches_commit_or_roll_back_together() {
    let mut s = store();
    let ok = s.atomic(|s| {
        let a = person(s, "A", "A", Sex::Male);
        let b = person(s, "B", "B", Sex::Male);
        parent(s, &a, &b, LinkKind::Birth)?;
        Ok(a)
    });
    assert!(ok.is_ok());
    assert_eq!(s.search_people(&CTX, &SearchQuery::default()).unwrap().len(), 2);

    let failed: Result<()> = s.atomic(|s| {
        let c = person(s, "C", "C", Sex::Male);
        let d = person(s, "D", "D", Sex::Male);
        parent(s, &c, &d, LinkKind::Birth)?;
        parent(s, &d, &c, LinkKind::Birth)?; // cycle: the whole batch must vanish
        Ok(())
    });
    assert!(matches!(failed, Err(StoreError::Cycle)));
    assert_eq!(s.search_people(&CTX, &SearchQuery::default()).unwrap().len(), 2, "nothing of the failed batch remains");
    // the store still works normally afterwards
    person(&mut s, "E", "E", Sex::Male);
    assert_eq!(s.search_people(&CTX, &SearchQuery::default()).unwrap().len(), 3);
}
