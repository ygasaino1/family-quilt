//! One sex vocabulary for agents: they read words (`male`, `female`, `other`, `unknown`) and every place that
//! takes or echoes a value uses the same words, so a value read from one result can be sent back unchanged.
//! The stored codes (M, F, X, U) stay inside the store; the codes are still accepted as input.

use family_review::query::{card, list_people, ListOpts};
use family_review::submit::submit;
use family_store::*;
use serde_json::{json, Value};

const CTX: AccessContext = AccessContext::Everything;

fn kim() -> (Store, String) {
    let mut s = Store::open_in_memory().unwrap();
    let id = s.create_person(&CTX, NewPerson { sex: Sex::Female, names: vec![NewName::full("Kim Tester")], ..Default::default() }, &Origin::human()).unwrap().id;
    (s, id)
}

fn change_sex(id: &str, from: &str, to: &str) -> Value {
    json!({"summary": "fix sex", "changes": [{"target_type": "person", "target_id": id, "op": "update", "fields": {"sex": {"from": from, "to": to}}}]})
}

#[test]
fn a_sex_read_from_a_card_is_accepted_back_as_the_from_value() {
    let (mut s, id) = kim();
    let read = card(&s, &id).unwrap()["sex"].as_str().unwrap().to_string();
    assert_eq!(read, "female");
    let r = submit(&mut s, "scribe", &change_sex(&id, &read, "other"));
    assert!(r.is_ok(), "{:?}", r.err().map(|e| e.to_string()));
}

#[test]
fn the_stored_codes_are_still_accepted_and_either_vocabulary_can_be_mixed() {
    let (mut s, id) = kim();
    assert!(submit(&mut s, "scribe", &change_sex(&id, "F", "X")).is_ok());
    let (mut s, id) = kim();
    assert!(submit(&mut s, "scribe", &change_sex(&id, "female", "X")).is_ok());
    let (mut s, id) = kim();
    assert!(submit(&mut s, "scribe", &change_sex(&id, "F", "male")).is_ok());
}

#[test]
fn a_wrong_from_names_the_current_value_in_words() {
    let (mut s, id) = kim();
    let e = match submit(&mut s, "scribe", &change_sex(&id, "male", "other")) {
        Err(family_review::ReviewError::Invalid(v)) => v,
        other => panic!("expected a field error, got {:?}", other.map(|_| ())),
    };
    assert_eq!(e[0].code, "stale_from");
    assert!(e[0].message.contains("female") && !e[0].message.contains("\"F\""), "{}", e[0].message);
}

#[test]
fn an_unknown_sex_is_a_field_error_naming_the_words() {
    let (mut s, id) = kim();
    let e = match submit(&mut s, "scribe", &change_sex(&id, "female", "robot")) {
        Err(family_review::ReviewError::Invalid(v)) => v,
        other => panic!("expected a field error, got {:?}", other.map(|_| ())),
    };
    assert_eq!(e[0].code, "invalid_value");
    assert!(e[0].message.contains("male, female, other, unknown"), "{}", e[0].message);
}

#[test]
fn list_people_counts_and_filters_speak_the_same_words() {
    let (mut s, _) = kim();
    s.create_person(&CTX, NewPerson { sex: Sex::Male, names: vec![NewName::full("Max Tester")], ..Default::default() }, &Origin::human()).unwrap();
    let g = list_people(&s, &ListOpts { group_by: Some("sex".into()), ..Default::default() }).unwrap();
    assert_eq!((g["groups"]["female"].as_u64(), g["groups"]["male"].as_u64()), (Some(1), Some(1)));
    assert!(g["groups"].get("F").is_none());
    // A group key can be used as the filter value, and the filter is echoed back in the same word.
    let f = list_people(&s, &ListOpts { sex: Some("female".into()), ..Default::default() }).unwrap();
    assert_eq!(f["filters"]["sex"], "female");
    assert_eq!(f["total"], 1);
    let f2 = list_people(&s, &ListOpts { sex: Some("F".into()), ..Default::default() }).unwrap();
    assert_eq!(f2["filters"]["sex"], "female");
}
