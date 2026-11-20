use family_review::apply::{approve, reject, ApproveRequest, ChangeDecision};
use family_review::query;
use family_review::review::review;
use family_review::submit::submit;
use family_review::ReviewError;
use family_store::*;
use serde_json::{json, Value};

const CTX: AccessContext = AccessContext::Everything;

fn person(s: &mut Store, name: &str, sex: Sex, birth: Option<&str>) -> String {
    let mut n = NewPerson { sex, names: vec![NewName::full(name)], ..Default::default() };
    n.birth = birth.map(|b| FuzzyDate::exact(b).unwrap());
    s.create_person(&CTX, n, &Origin::human()).unwrap().id
}

fn link(s: &mut Store, parent: &str, child: &str) -> String {
    s.add_relationship(&CTX, NewRelationship::parent_child(parent, child, LinkKind::Birth), &Origin::human()).unwrap().id
}

fn errors(r: std::result::Result<family_review::submit::SubmitResult, ReviewError>) -> Vec<(String, &'static str)> {
    match r {
        Err(ReviewError::Invalid(v)) => v.into_iter().map(|e| (e.path, e.code)).collect(),
        Ok(_) => panic!("expected validation errors"),
        Err(e) => panic!("unexpected {e}"),
    }
}

struct Fam {
    s: Store,
    grandpa: String,
    dad: String,
    kid: String,
    link_gd: String,
    link_dk: String,
}

fn fam() -> Fam {
    let mut s = Store::open_in_memory().unwrap();
    let grandpa = person(&mut s, "Gus Tester", Sex::Male, Some("1900"));
    let dad = person(&mut s, "Dan Tester", Sex::Male, Some("1930"));
    let kid = person(&mut s, "Kim Tester", Sex::Female, Some("1960"));
    let link_gd = link(&mut s, &grandpa, &dad);
    let link_dk = link(&mut s, &dad, &kid);
    Fam { s, grandpa, dad, kid, link_gd, link_dk }
}

fn change_person(id: &str, field: &str, from: Value, to: Value) -> Value {
    json!({"summary": "fix", "changes": [{"target_type": "person", "target_id": id, "op": "update", "fields": {field: {"from": from, "to": to}}}]})
}

#[test]
fn new_records_are_pending_and_invisible_until_approved() {
    let mut f = fam();
    let body = json!({
        "summary": "Add Zoe", "source_text": "Zoe is Kim's daughter",
        "people": [{"ref": "z", "given_name": "Zoe", "family_name": "Tester", "sex": "F", "birth_date": "1990"}],
        "relationships": [{"type": "parent_child", "a": f.kid, "b": "z", "kind": "birth"}]
    });
    let r = submit(&mut f.s, "test-agent", &body).unwrap();
    assert_eq!(r.status, "pending");
    assert_eq!(r.people.len(), 1);
    let zid = r.people[0].id.clone();
    // Verified-only search does not see her; the graph neither.
    assert!(query::search(&f.s, "Zoe", 10, false).unwrap().is_empty());
    assert_eq!(query::search(&f.s, "Zoe", 10, true).unwrap().len(), 1);
    assert!(query::load_graph(&f.s, false).unwrap().index_of(&zid).is_none());
    let v = review(&f.s, &r.proposal_id).unwrap();
    assert_eq!(v.new_people.len(), 1);
    assert_eq!(v.new_relationships.len(), 1);
    assert_eq!(v.new_people[0].created_by, "agent:test-agent");

    let rep = approve(&mut f.s, &r.proposal_id, &ApproveRequest::default(), "human").unwrap();
    assert_eq!((rep.people_verified, rep.relationships_verified), (1, 1));
    assert_eq!(query::search(&f.s, "Zoe", 10, false).unwrap().len(), 1);
    assert_eq!(f.s.get_proposal(&CTX, &r.proposal_id).unwrap().status, "approved");
    // Revisions of the approval carry the proposal id.
    let revs = f.s.revisions_of(&CTX, "person", &zid).unwrap();
    assert!(revs.iter().all(|r| r.proposal_id.is_some()));
}

#[test]
fn reject_removes_pending_and_changes_nothing() {
    let mut f = fam();
    let body = json!({"summary": "x", "people": [{"ref": "n", "full_name": "New Person"}],
        "changes": [{"target_type": "person", "target_id": f.kid, "op": "update", "fields": {"note": {"from": null, "to": "hello"}}}]});
    let r = submit(&mut f.s, "a", &body).unwrap();
    reject(&mut f.s, &r.proposal_id, "human").unwrap();
    assert!(query::search(&f.s, "New Person", 10, true).unwrap().is_empty());
    assert_eq!(f.s.get_person(&CTX, &f.kid).unwrap().note, None);
    assert_eq!(f.s.get_proposal(&CTX, &r.proposal_id).unwrap().status, "rejected");
    assert!(approve(&mut f.s, &r.proposal_id, &ApproveRequest::default(), "human").is_err(), "a decided proposal cannot be approved");
}

#[test]
fn proposed_change_leaves_record_alone_until_approval_and_applies_only_accepted_fields() {
    let mut f = fam();
    let body = json!({"summary": "fix Kim", "changes": [{"target_type": "person", "target_id": f.kid, "op": "update", "agent_note": "user said so",
        "fields": {"birth_date": {"from": "1960", "to": "1961"}, "note": {"from": null, "to": "born in spring"}, "birth_place": {"from": null, "to": "Shiraz"}}}]});
    let r = submit(&mut f.s, "a", &body).unwrap();
    let before = f.s.get_person(&CTX, &f.kid).unwrap();
    assert_eq!(before.birth.unwrap().date.to_string(), "1960");
    assert_eq!(before.note, None);
    // The sidebar sees it as a suggested change.
    assert_eq!(query::person_detail(&f.s, &f.kid).unwrap().suggested_changes.len(), 1);
    let v = review(&f.s, &r.proposal_id).unwrap();
    assert_eq!(v.changes[0].fields.len(), 3);

    let cid = r.changes[0].id.clone();
    let req = ApproveRequest { decisions: vec![ChangeDecision { change_id: cid.clone(), accept: true, fields: Some(vec!["birth_date".into(), "note".into()]), overwrite: vec![] }] };
    let rep = approve(&mut f.s, &r.proposal_id, &req, "human").unwrap();
    assert_eq!(rep.changes_applied, 1);
    let after = f.s.get_person(&CTX, &f.kid).unwrap();
    assert_eq!(after.birth.unwrap().date.to_string(), "1961");
    assert_eq!(after.note.as_deref(), Some("born in spring"));
    assert!(after.birth_place.is_none(), "the unaccepted field must not be applied");
    assert_eq!(f.s.changes_of(&CTX, &r.proposal_id).unwrap()[0].status, "applied");
}

#[test]
fn stale_from_is_rejected_at_submit_and_flagged_after() {
    let mut f = fam();
    // Wrong `from` at submit time.
    let e = errors(submit(&mut f.s, "a", &change_person(&f.kid, "birth_date", json!("1950"), json!("1961"))));
    assert_eq!(e, vec![("$.changes[0].fields.birth_date.from".to_string(), "stale_from")]);

    // Right `from`, then the human edits the same field before reviewing.
    let r = submit(&mut f.s, "a", &change_person(&f.kid, "birth_date", json!("1960"), json!("1961"))).unwrap();
    f.s.update_person(&CTX, &f.kid, PersonPatch { birth: Field::Set(FuzzyDate::exact("1959").unwrap()), ..Default::default() }, "human").unwrap();
    let v = review(&f.s, &r.proposal_id).unwrap();
    assert!(v.changes[0].stale);
    assert!(v.changes[0].fields[0].conflict);
    assert_eq!(v.changes[0].fields[0].current, json!("1959"));
    let rep = approve(&mut f.s, &r.proposal_id, &ApproveRequest::default(), "human").unwrap();
    assert_eq!(rep.changes_applied, 0);
    assert_eq!(rep.conflicts_skipped.len(), 1);
    assert_eq!(f.s.get_person(&CTX, &f.kid).unwrap().birth.unwrap().date.to_string(), "1959", "never applied silently");

    // A second proposal, overwritten on purpose.
    let r2 = submit(&mut f.s, "a", &change_person(&f.kid, "birth_date", json!("1959"), json!("1962"))).unwrap();
    f.s.update_person(&CTX, &f.kid, PersonPatch { birth: Field::Set(FuzzyDate::exact("1958").unwrap()), ..Default::default() }, "human").unwrap();
    let req = ApproveRequest { decisions: vec![ChangeDecision { change_id: r2.changes[0].id.clone(), accept: true, fields: None, overwrite: vec!["birth_date".into()] }] };
    approve(&mut f.s, &r2.proposal_id, &req, "human").unwrap();
    assert_eq!(f.s.get_person(&CTX, &f.kid).unwrap().birth.unwrap().date.to_string(), "1962");
}

#[test]
fn impact_note_for_birth_to_adopted() {
    let mut f = fam();
    let body = json!({"summary": "Dan was adopted", "changes": [{"target_type": "relationship", "target_id": f.link_gd, "op": "update",
        "agent_note": "user said adopted", "fields": {"kind": {"from": "birth", "to": "adopted"}}}]});
    let r = submit(&mut f.s, "a", &body).unwrap();
    let w = r.warnings.iter().find(|w| w.code == "link_kind_impact").expect("impact warning");
    assert!(w.message.contains("removes 1 person from Dan Tester's blood lineage"), "{}", w.message);
    assert!(w.message.contains("2 stored relationship answers"), "{}", w.message); // Dan + Kim x Gus
                                                                                   // Pending=1 applies the change on top; verified-only does not.
    let blood = |pending| query::relationship(&f.s, &f.grandpa, &f.kid, pending, false).unwrap().by_blood;
    assert!(blood(false));
    assert!(!blood(true), "pending changes apply on top only with pending=1");
}

#[test]
fn undo_restores_previous_values_and_is_recorded() {
    let mut f = fam();
    f.s.update_person(&CTX, &f.kid, PersonPatch { note: Field::Set("first".into()), ..Default::default() }, "human").unwrap();
    f.s.update_person(&CTX, &f.kid, PersonPatch { note: Field::Set("second".into()), ..Default::default() }, "human").unwrap();
    let revs = f.s.recent_revisions(&CTX, Some(("person", &f.kid)), 10).unwrap();
    let (latest, older) = (&revs[0], &revs[1]);
    // The older edit cannot be undone while a newer one sits on top.
    assert!(f.s.undo_revision(&CTX, &older.id, "human").is_err());
    f.s.undo_revision(&CTX, &latest.id, "human").unwrap();
    assert_eq!(f.s.get_person(&CTX, &f.kid).unwrap().note.as_deref(), Some("first"));
    let n_after = f.s.recent_revisions(&CTX, Some(("person", &f.kid)), 10).unwrap().len();
    assert_eq!(n_after, revs.len() + 1, "undo writes a new revision, history is kept");
    // Undo of a link removal brings the link back.
    f.s.remove_relationship(&CTX, &f.link_dk, "human").unwrap();
    assert!(f.s.relationships_of(&CTX, &f.kid).unwrap().is_empty());
    let rev = f.s.recent_revisions(&CTX, Some(("relationship", &f.link_dk)), 1).unwrap().remove(0);
    f.s.undo_revision(&CTX, &rev.id, "human").unwrap();
    assert_eq!(f.s.relationships_of(&CTX, &f.kid).unwrap().len(), 1);
    let _ = &f.dad;
}

#[test]
fn disallowed_things_are_rejected_with_field_errors() {
    let mut f = fam();
    for (field, code) in [
        ("id", "field_not_editable"),
        ("status", "field_not_editable"),
        ("created_by", "field_not_editable"),
        ("vault_id", "field_not_editable"),
        ("merged_into", "field_not_editable"),
        ("updated_at", "field_not_editable"),
        ("proposal_id", "field_not_editable"),
    ] {
        let e = errors(submit(&mut f.s, "a", &change_person(&f.kid, field, json!("x"), json!("y"))));
        assert_eq!(e, vec![(format!("$.changes[0].fields.{field}"), code)], "{field}");
    }
    for op in ["delete", "merge"] {
        let bad = json!({"summary": "x", "changes": [{"target_type": "person", "target_id": f.kid, "op": op}]});
        assert_eq!(errors(submit(&mut f.s, "a", &bad)), vec![("$.changes[0].op".to_string(), "not_allowed")], "{op}");
    }
    let names = json!({"summary": "x", "changes": [{"target_type": "person_name", "target_id": f.kid, "op": "remove"}]});
    assert_eq!(errors(submit(&mut f.s, "a", &names))[0].1, "not_allowed");
    // Unknown top-level and person fields, bad enum, missing ref, unknown id.
    let bad = json!({"summary": "x", "foo": 1, "people": [{"given_name": "A", "sex": "Q", "colour": "red"}], "relationships": [{"type": "parent_child", "a": "nope", "b": f.kid}]});
    let e = errors(submit(&mut f.s, "a", &bad));
    for want in [("$.foo", "unknown_field"), ("$.people[0].colour", "unknown_field"), ("$.people[0].sex", "invalid_value"), ("$.people[0].ref", "required"), ("$.relationships[0].a", "unknown_id")] {
        assert!(e.iter().any(|(p, c)| p == want.0 && *c == want.1), "missing {want:?} in {e:?}");
    }
    // Nothing was stored by any failed proposal.
    assert!(f.s.list_proposals(&CTX, None, None, 10).unwrap().is_empty());
}

#[test]
fn removal_with_stale_base_is_refused() {
    let mut f = fam();
    let e = errors(submit(&mut f.s, "b", &json!({"summary": "x", "changes": [{"target_type": "relationship", "target_id": f.link_dk, "op": "remove", "base_updated_at": "2000-01-01T00:00:00Z"}]})));
    assert_eq!(e[0].1, "stale");
}

#[test]
fn links_and_edits_may_target_pending_people_but_approval_waits_for_their_proposal() {
    let mut f = fam();
    // Proposal A adds Pat (pending).
    let a = submit(&mut f.s, "a", &json!({"summary": "add Pat", "people": [{"ref": "n", "full_name": "Pending Pat"}]})).unwrap();
    let pat = a.people[0].id.clone();
    // Proposal B (another agent) links Kim -> Pat and edits Pat's note.
    let b = submit(
        &mut f.s,
        "b",
        &json!({"summary": "Pat is Kim's child", "relationships": [{"type": "parent_child", "a": f.kid, "b": pat}],
                "changes": [{"target_type": "person", "target_id": pat, "op": "update", "fields": {"note": {"from": null, "to": "hello"}}}]}),
    )
    .unwrap();
    let v = review(&f.s, &b.proposal_id).unwrap();
    assert_eq!(v.depends_on.len(), 1);
    assert_eq!(v.depends_on[0].proposal_id, a.proposal_id);
    assert!(review(&f.s, &a.proposal_id).unwrap().depends_on.is_empty());
    // Approve is blocked until A is approved.
    let err = approve(&mut f.s, &b.proposal_id, &ApproveRequest::default(), "human").unwrap_err();
    assert!(err.to_string().contains("approve the proposal it depends on first"), "{err}");
    assert_eq!(f.s.get_proposal(&CTX, &b.proposal_id).unwrap().status, "pending");
    approve(&mut f.s, &a.proposal_id, &ApproveRequest::default(), "human").unwrap();
    let rep = approve(&mut f.s, &b.proposal_id, &ApproveRequest::default(), "human").unwrap();
    assert_eq!((rep.relationships_verified, rep.changes_applied), (1, 1));
    assert_eq!(f.s.get_person(&CTX, &pat).unwrap().note.as_deref(), Some("hello"));
    assert!(query::relationship(&f.s, &f.kid, &pat, false, false).unwrap().a_is.is_some());
}

#[test]
fn rejecting_the_proposal_a_link_depends_on_removes_the_dangling_link_and_reports_it() {
    let mut f = fam();
    let a = submit(&mut f.s, "a", &json!({"summary": "add Pat", "people": [{"ref": "n", "full_name": "Pending Pat"}]})).unwrap();
    let pat = a.people[0].id.clone();
    let b = submit(
        &mut f.s,
        "b",
        &json!({"summary": "link + edit", "people": [{"ref": "z", "full_name": "Zed Z"}],
        "relationships": [{"type": "parent_child", "a": f.kid, "b": pat}, {"type": "parent_child", "a": f.kid, "b": "z"}],
        "changes": [{"target_type": "person", "target_id": pat, "op": "update", "fields": {"note": {"from": null, "to": "x"}}}]}),
    )
    .unwrap();
    let rep = reject(&mut f.s, &a.proposal_id, "human").unwrap();
    assert_eq!(rep.affected_proposals, vec![b.proposal_id.clone()]);
    // B is now free of the dependency; its own new person and link survive, the dangling link and edit do not.
    let v = review(&f.s, &b.proposal_id).unwrap();
    assert!(v.depends_on.is_empty());
    assert_eq!(v.new_relationships.len(), 1);
    assert_eq!(v.changes[0].change.status, "conflicting");
    let rep = approve(&mut f.s, &b.proposal_id, &ApproveRequest::default(), "human").unwrap();
    assert_eq!((rep.people_verified, rep.relationships_verified, rep.changes_applied), (1, 1, 0));
    assert!(query::search(&f.s, "Pending Pat", 5, true).unwrap().is_empty());
}

#[test]
fn warnings_are_computed_on_the_resulting_state() {
    let mut f = fam();
    // Kim (b.1960) becomes the mother of someone born 1965 (age 5), and the new person dies before birth.
    let body = json!({"summary": "x", "people": [{"ref": "c", "full_name": "Tiny Tester", "birth_date": "1965", "death_date": "1964"}],
        "relationships": [{"type": "parent_child", "a": f.kid, "b": "c"}]});
    let r = submit(&mut f.s, "a", &body).unwrap();
    let codes: Vec<_> = r.warnings.iter().map(|w| w.code).collect();
    assert!(codes.contains(&"parent_too_young"), "{codes:?}");
    assert!(codes.contains(&"death_before_birth"), "{codes:?}");
    // A change that moves a birth date creates a warning too.
    let r2 = submit(&mut f.s, "a", &change_person(&f.dad, "death_date", Value::Null, json!("1955"))).unwrap();
    assert!(r2.warnings.iter().any(|w| w.code == "parent_died_before_birth"), "{:?}", r2.warnings);
    // Duplicate names are flagged.
    let r3 = submit(&mut f.s, "a", &json!({"summary": "x", "people": [{"ref": "d", "full_name": "Kim Tester"}]})).unwrap();
    assert!(r3.warnings.iter().any(|w| w.code == "possible_duplicate"));
}

#[test]
fn limits_on_open_proposals_per_agent() {
    let mut f = fam();
    for i in 0..50 {
        submit(&mut f.s, "spammer", &json!({"summary": format!("p{i}"), "people": [{"ref": "x", "full_name": format!("P {i}")}]})).unwrap();
    }
    let e = errors(submit(&mut f.s, "spammer", &json!({"summary": "one more", "people": [{"ref": "x", "full_name": "Zed"}]})));
    assert_eq!(e[0].1, "too_many_open_proposals");
    // Another agent is unaffected.
    submit(&mut f.s, "other", &json!({"summary": "ok", "people": [{"ref": "x", "full_name": "Zed"}]})).unwrap();
}

#[test]
fn agents_md_matches_the_guide() {
    let md = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../AGENTS.md")).unwrap();
    assert_eq!(md, family_review::guide::GUIDE);
}

#[test]
fn person_removal_is_a_proposal_with_impact_note_and_needs_an_explicit_tick() {
    let mut f = fam();
    let body = json!({"summary": "Dan was entered twice", "changes": [{"target_type": "person", "target_id": f.dad, "op": "remove", "agent_note": "user asked to delete Dan"}]});
    let r = submit(&mut f.s, "a", &body).unwrap();
    // Nothing happens at submit; the impact note names the links and the orphaned child.
    assert!(f.s.get_person(&CTX, &f.dad).is_ok());
    let w = r.warnings.iter().find(|w| w.code == "person_removal_impact").expect("impact");
    assert!(w.message.contains("2 links") && w.message.contains("Kim Tester"), "{}", w.message);
    let v = review(&f.s, &r.proposal_id).unwrap();
    assert!(v.changes[0].needs_explicit_approval);
    // Approving without ticking it does not delete anyone.
    let rep = approve(&mut f.s, &r.proposal_id, &ApproveRequest::default(), "human").unwrap();
    assert_eq!((rep.people_removed, rep.removals_not_confirmed.len()), (0, 1));
    assert!(f.s.get_person(&CTX, &f.dad).is_ok());
    // Second proposal, ticked.
    let r2 = submit(&mut f.s, "a", &body).unwrap();
    let req = ApproveRequest { decisions: vec![ChangeDecision { change_id: r2.changes[0].id.clone(), accept: true, fields: None, overwrite: vec![] }] };
    let rep = approve(&mut f.s, &r2.proposal_id, &req, "human").unwrap();
    assert_eq!(rep.people_removed, 1);
    assert!(f.s.get_person(&CTX, &f.dad).is_err());
    assert!(f.s.relationships_of(&CTX, &f.kid).unwrap().is_empty());
    // Undo brings him back (the person revision), tagged with the proposal.
    let rev = f.s.recent_revisions(&CTX, Some(("person", &f.dad)), 1).unwrap().remove(0);
    assert_eq!(rev.proposal_id.as_deref(), Some(r2.proposal_id.as_str()));
    f.s.undo_revision(&CTX, &rev.id, "human").unwrap();
    assert!(f.s.get_person(&CTX, &f.dad).is_ok());
    // A pending person cannot be removed by proposal.
    let a = submit(&mut f.s, "a", &json!({"summary": "x", "people": [{"ref": "n", "full_name": "Pending Pat"}]})).unwrap();
    let e = errors(submit(&mut f.s, "b", &json!({"summary": "x", "changes": [{"target_type": "person", "target_id": a.people[0].id, "op": "remove"}]})));
    assert_eq!(e[0].1, "pending_target");
}
