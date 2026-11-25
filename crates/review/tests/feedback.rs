//! Reject reasons, the resend guard, corrections in review, and the history agents read.

use family_review::apply::{approve, reject_with_note, ApproveRequest};
use family_review::edit::{edit, EditRequest};
use family_review::history::person_history;
use family_review::review::review;
use family_review::submit::submit;
use family_review::ReviewError;
use family_store::*;
use serde_json::{json, Value};

const CTX: AccessContext = AccessContext::Everything;

fn base() -> (Store, String) {
    let mut s = Store::open_in_memory().unwrap();
    let kim = s
        .create_person(&CTX, NewPerson { sex: Sex::Female, names: vec![NewName::full("Kim Tester")], birth: Some(FuzzyDate::exact("1960").unwrap()), ..Default::default() }, &Origin::human())
        .unwrap()
        .id;
    (s, kim)
}

fn add_zoe(kim: &str) -> Value {
    json!({
        "summary": "Add Zoe", "source_text": "Zoe is Kim's daughter",
        "people": [{"ref": "z", "given_name": "Zoe", "family_name": "Tester", "sex": "F", "birth_date": "1990", "agent_note": "year from the user"}],
        "relationships": [{"type": "parent_child", "a": kim, "b": "z", "kind": "birth"}]
    })
}

fn invalid(r: std::result::Result<family_review::submit::SubmitResult, ReviewError>) -> Vec<(String, &'static str, String)> {
    match r {
        Err(ReviewError::Invalid(v)) => v.into_iter().map(|e| (e.path, e.code, e.message)).collect(),
        Ok(_) => panic!("expected an error"),
        Err(e) => panic!("unexpected {e}"),
    }
}

fn events(s: &Store, id: &str) -> Vec<Value> {
    person_history(s, id, None).unwrap()["history"].as_array().unwrap().clone()
}

#[test]
fn a_rejected_proposal_cannot_be_sent_again_unless_acknowledged() {
    let (mut s, kim) = base();
    let first = submit(&mut s, "scribe", &add_zoe(&kim)).unwrap();
    reject_with_note(&mut s, &first.proposal_id, "human", Some("wrong: Zoe is the niece")).unwrap();
    assert_eq!(s.get_proposal(&CTX, &first.proposal_id).unwrap().decision_note.as_deref(), Some("wrong: Zoe is the niece"));

    // Same request, different wording of the notes: refused, with the reason and the id to acknowledge.
    let mut again = add_zoe(&kim);
    again["summary"] = json!("Please add Zoe");
    again["people"][0]["agent_note"] = json!("different words");
    let e = invalid(submit(&mut s, "scribe", &again));
    assert_eq!(e[0].1, "previously_rejected");
    assert!(e[0].2.contains("Zoe is the niece") && e[0].2.contains(&first.proposal_id), "{}", e[0].2);
    // Another agent is told the same.
    assert_eq!(invalid(submit(&mut s, "other", &again))[0].1, "previously_rejected");

    // Changed in a way that matters (a different birth year): allowed.
    let mut changed = add_zoe(&kim);
    changed["people"][0]["birth_date"] = json!("1995");
    submit(&mut s, "scribe", &changed).unwrap();
    // Unchanged but acknowledged: allowed.
    again["acknowledge_rejected"] = json!([first.proposal_id]);
    submit(&mut s, "scribe", &again).unwrap();
}

#[test]
fn corrections_in_review_are_checked_applied_and_remembered() {
    let (mut s, kim) = base();
    let r = submit(&mut s, "scribe", &add_zoe(&kim)).unwrap();
    let view = review(&s, &r.proposal_id).unwrap();
    let zoe = view.new_people[0].id.clone();
    let link = view.new_relationships[0].rel.id.clone();

    // A bad value is refused with the field named, and nothing changes.
    let bad = EditRequest { kind: "person".into(), target_id: zoe.clone(), fields: json!({"birth_date": "19x0"}).as_object().unwrap().clone(), ..Default::default() };
    match edit(&mut s, &r.proposal_id, &bad, "human") {
        Err(ReviewError::Invalid(v)) => assert_eq!(v[0].path, "$.fields.birth_date"),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.get_person(&CTX, &zoe).unwrap().birth.unwrap().date.to_string(), "1990");

    // Fix the date, the qualifier and the link kind.
    let fix = EditRequest { kind: "person".into(), target_id: zoe.clone(), fields: json!({"birth_date": "1991-05", "birth_qualifier": "about"}).as_object().unwrap().clone(), ..Default::default() };
    edit(&mut s, &r.proposal_id, &fix, "human").unwrap();
    let kind = EditRequest { kind: "relationship".into(), target_id: link.clone(), fields: json!({"kind": "adopted"}).as_object().unwrap().clone(), ..Default::default() };
    edit(&mut s, &r.proposal_id, &kind, "human").unwrap();
    // An item from another proposal, or the ends of a link, cannot be edited here.
    let ends = EditRequest { kind: "relationship".into(), target_id: link.clone(), fields: json!({"person_a": "x"}).as_object().unwrap().clone(), ..Default::default() };
    assert!(matches!(edit(&mut s, &r.proposal_id, &ends, "human"), Err(ReviewError::Invalid(_))));
    let stranger = EditRequest { kind: "person".into(), target_id: kim.clone(), fields: json!({"note": "x"}).as_object().unwrap().clone(), ..Default::default() };
    assert!(matches!(edit(&mut s, &r.proposal_id, &stranger, "human"), Err(ReviewError::Invalid(_))));

    // The log keeps what the agent said; putting the value back removes the entry.
    let edits = review(&s, &r.proposal_id).unwrap().edits;
    let by = |f: &str| edits.iter().find(|e| e["field"] == f).cloned();
    assert_eq!(by("birth_date").unwrap()["original"], json!("1990"));
    assert_eq!(by("birth_date").unwrap()["current"], json!("1991-05"));
    let undo = EditRequest { kind: "relationship".into(), target_id: link.clone(), fields: json!({"kind": "birth"}).as_object().unwrap().clone(), ..Default::default() };
    edit(&mut s, &r.proposal_id, &undo, "human").unwrap();
    assert!(review(&s, &r.proposal_id).unwrap().edits.iter().all(|e| e["field"] != "kind"));

    // Approving applies the corrected values.
    approve(&mut s, &r.proposal_id, &ApproveRequest::default(), "human").unwrap();
    let p = s.get_person(&CTX, &zoe).unwrap();
    assert_eq!(p.birth.unwrap().date.to_string(), "1991-05");
    // Once decided, no more corrections.
    assert!(edit(&mut s, &r.proposal_id, &fix, "human").is_err());
}

#[test]
fn history_tells_the_story_to_an_agent() {
    let (mut s, kim) = base();
    let r = submit(&mut s, "scribe", &add_zoe(&kim)).unwrap();
    let zoe = review(&s, &r.proposal_id).unwrap().new_people[0].id.clone();
    let fix = EditRequest { kind: "person".into(), target_id: zoe.clone(), fields: json!({"birth_date": "1991"}).as_object().unwrap().clone(), note: Some("  her passport says\n 1991  ".into()) };
    edit(&mut s, &r.proposal_id, &fix, "human").unwrap();
    // A note that is too long is refused and nothing else changes.
    let long = EditRequest { kind: "person".into(), target_id: zoe.clone(), fields: json!({"birth_date": "1992"}).as_object().unwrap().clone(), note: Some("x".repeat(301)) };
    match edit(&mut s, &r.proposal_id, &long, "human") {
        Err(ReviewError::Invalid(v)) => assert_eq!(v[0].path, "$.note"),
        other => panic!("{other:?}"),
    }
    approve(&mut s, &r.proposal_id, &ApproveRequest::default(), "human").unwrap();

    let ev = events(&s, &zoe);
    let kinds: Vec<&str> = ev.iter().map(|e| e["event"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"proposed") && kinds.contains(&"corrected") && kinds.contains(&"approved"), "{kinds:?}");
    let proposed = ev.iter().find(|e| e["event"] == "proposed").unwrap();
    assert_eq!(proposed["who"], json!({"type": "agent", "name": "scribe"}));
    assert_eq!(proposed["note"], json!("year from the user"));
    let corrected = ev.iter().find(|e| e["event"] == "corrected").unwrap();
    assert_eq!(corrected["changes"][0], json!({"field": "birth_date", "from": "1990", "to": "1991"}));
    assert_eq!(corrected["note"], json!("her passport says 1991"), "the user's one-line reason reaches the agent, tidied");
    let h = person_history(&s, &zoe, None).unwrap();
    assert_eq!(h["user_confirmed"]["birth_date"]["value"], json!("1991"));

    // A later hand edit by the user is an `edited` event and stays user-confirmed.
    s.update_person(&CTX, &zoe, PersonPatch { note: Field::Set("born at home".into()), ..Default::default() }, "human").unwrap();
    let h = person_history(&s, &zoe, None).unwrap();
    assert!(h["history"].as_array().unwrap().iter().any(|e| e["event"] == "edited" && e["who"]["type"] == "user"));
    assert_eq!(h["user_confirmed"]["note"]["value"], json!("born at home"));
}

#[test]
fn history_shows_rejected_edits_and_is_capped() {
    let (mut s, kim) = base();
    let body = json!({"summary": "Kim was born in 1961", "changes": [{"target_type": "person", "target_id": kim, "op": "update", "agent_note": "a guess",
        "fields": {"birth_date": {"from": "1960", "to": "1961"}}}]});
    let r = submit(&mut s, "scribe", &body).unwrap();
    // While pending it is a suggestion; then the user rejects it with a reason.
    assert!(events(&s, &kim).iter().any(|e| e["event"] == "edit_proposed"));
    reject_with_note(&mut s, &r.proposal_id, "human", Some("not_wanted")).unwrap();
    let ev = events(&s, &kim);
    let rej = ev.iter().find(|e| e["event"] == "edit_rejected").unwrap();
    assert_eq!(rej["rejection_note"], json!("not_wanted"));
    assert_eq!(rej["changes"][0]["to"], json!("1961"));

    // Many events: the default view returns the newest 20 and says how many there are.
    for i in 0..30 {
        s.update_person(&CTX, &kim, PersonPatch { note: Field::Set(format!("note {i}")), ..Default::default() }, "human").unwrap();
    }
    let h = person_history(&s, &kim, Some(family_review::history::DEFAULT_LIMIT)).unwrap();
    assert_eq!(h["history"].as_array().unwrap().len(), 20);
    assert!(h["history_total"].as_u64().unwrap() > 30);
    assert_eq!(h["history"].as_array().unwrap().last().unwrap()["changes"][0]["to"], json!("note 29"));
}

#[test]
fn a_long_note_is_cut_and_stays_under_note() {
    let (mut s, kim) = base();
    let mut body = add_zoe(&kim);
    body["people"][0]["agent_note"] = json!("x".repeat(900));
    let r = submit(&mut s, "scribe", &body).unwrap();
    let zoe = review(&s, &r.proposal_id).unwrap().new_people[0].id.clone();
    let proposed = events(&s, &zoe).into_iter().find(|e| e["event"] == "proposed").unwrap();
    assert!(proposed["note"].as_str().unwrap().chars().count() <= 300);
}

#[test]
fn an_agent_can_withdraw_or_replace_its_own_pending_proposal() {
    use family_review::apply::withdraw;
    let (mut s, kim) = base();
    let first = submit(&mut s, "scribe", &add_zoe(&kim)).unwrap();
    // Not yours: refused. Yours: withdrawn, nothing left in the tree, and it is not a rejection by the user.
    assert!(withdraw(&mut s, &first.proposal_id, "agent:other", None).is_err());
    withdraw(&mut s, &first.proposal_id, "agent:scribe", Some("duplicate")).unwrap();
    let p = s.get_proposal(&CTX, &first.proposal_id).unwrap();
    assert!(p.withdrawn && p.status == "rejected");
    assert!(p.decision_note.unwrap().contains("withdrawn by the agent: duplicate"));
    // The same request can be sent again at once (a withdrawal is not the user's no).
    let second = submit(&mut s, "scribe", &add_zoe(&kim)).unwrap();
    // Replace: the old one is withdrawn and the new one takes its place.
    let mut better = add_zoe(&kim);
    better["people"][0]["birth_date"] = json!("1991");
    better["replaces"] = json!(second.proposal_id);
    let third = submit(&mut s, "scribe", &better).unwrap();
    assert_eq!(third.replaced.as_deref(), Some(second.proposal_id.as_str()));
    assert!(s.get_proposal(&CTX, &second.proposal_id).unwrap().withdrawn);
    assert_eq!(s.get_proposal(&CTX, &third.proposal_id).unwrap().status, "pending");
    // Someone else's proposal cannot be replaced.
    let mut steal = add_zoe(&kim);
    steal["people"][0]["birth_date"] = json!("1992");
    steal["replaces"] = json!(third.proposal_id);
    assert_eq!(invalid(submit(&mut s, "other", &steal))[0].1, "not_yours");
    // A replacement may not point at people the old proposal adds.
    let zoe = review(&s, &third.proposal_id).unwrap().new_people[0].id.clone();
    let bad = json!({"summary": "x", "replaces": third.proposal_id, "relationships": [{"type": "partnership", "a": zoe, "b": kim, "subtype": "unknown"}]});
    assert_eq!(invalid(submit(&mut s, "scribe", &bad))[0].1, "refers_to_replaced");
    // A withdrawn edit shows in the history as the agent's own doing, not as the user's no.
    let edit = submit(&mut s, "scribe", &json!({"summary": "x", "changes": [{"target_type": "person", "target_id": kim, "op": "update", "fields": {"note": {"from": null, "to": "hi"}}}]})).unwrap();
    withdraw(&mut s, &edit.proposal_id, "agent:scribe", None).unwrap();
    let ev = events(&s, &kim);
    assert!(ev.iter().any(|e| e["event"] == "edit_withdrawn" && e["who"]["type"] == "agent"), "{ev:?}");
    // Another proposal that depends on it blocks the withdrawal.
    let dep = json!({"summary": "Add Zoe's son", "people": [{"ref": "s", "given_name": "Sam", "sex": "M"}], "relationships": [{"type": "parent_child", "a": zoe, "b": "s"}]});
    let d = submit(&mut s, "scribe", &dep).unwrap();
    match withdraw(&mut s, &third.proposal_id, "agent:scribe", None) {
        Err(ReviewError::Invalid(v)) => assert_eq!(v[0].code, "has_dependents"),
        other => panic!("{other:?}"),
    }
    withdraw(&mut s, &d.proposal_id, "agent:scribe", None).unwrap();
    withdraw(&mut s, &third.proposal_id, "agent:scribe", None).unwrap();
}

#[test]
fn duplicates_are_found_among_waiting_people_and_judged_by_relatives() {
    let (mut s, kim) = base();
    let fakher = |year: &str| {
        json!({"summary": "Add Fakher", "people": [{"ref": "f", "given_name": "فاخر", "family_name": "فرحی", "sex": "F", "birth_date": year}],
               "relationships": [{"type": "partnership", "a": kim, "b": "f", "subtype": "married"}]})
    };
    let first = submit(&mut s, "scribe", &fakher("1930")).unwrap();
    assert!(first.warnings.iter().all(|w| w.code != "likely_duplicate" && w.code != "possible_duplicate"), "{:?}", first.warnings);
    // Same name, same spouse, another proposal (still waiting): very likely the same person.
    let second = submit(&mut s, "scribe", &fakher("1930")).unwrap();
    let w = second.warnings.iter().find(|w| w.code == "likely_duplicate").expect("likely_duplicate");
    let c = &w.params["candidates"][0];
    assert_eq!(c["status"], json!("pending"));
    assert_eq!(c["shares_a_relative"], json!(true));
    assert_eq!(c["partners"][0], json!("Kim Tester"));
    assert!(c["proposal_id"].is_string());
    // Same name and spouse but clearly another birth year: only a "possible" duplicate.
    let third = submit(&mut s, "scribe", &fakher("1960")).unwrap();
    assert!(third.warnings.iter().any(|w| w.code == "possible_duplicate"));
    assert!(third.warnings.iter().all(|w| w.code != "likely_duplicate"));
    // A different name is left alone.
    let other = submit(&mut s, "scribe", &json!({"summary": "x", "people": [{"ref": "o", "given_name": "Nima", "family_name": "Farahi"}]})).unwrap();
    assert!(other.warnings.iter().all(|w| !w.code.ends_with("duplicate")));
}
