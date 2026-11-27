//! One shape for a link between two people, wherever an agent meets it: `id, type, a, b, kind, subtype, status`
//! (a person's own links add `role`, `other_id`, `other_name` for convenience). For parent_child, `a` is the parent.

use family_review::query::{neighbourhood, person_detail};
use family_review::submit::submit;
use family_store::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const CTX: AccessContext = AccessContext::Everything;
const EDGE: [&str; 7] = ["a", "b", "id", "kind", "status", "subtype", "type"];

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

fn tree() -> (Store, String, String) {
    let mut s = Store::open_in_memory().unwrap();
    let mum = s.create_person(&CTX, NewPerson { sex: Sex::Female, names: vec![NewName::full("Mum Tester")], ..Default::default() }, &Origin::human()).unwrap().id;
    let kid = s.create_person(&CTX, NewPerson { sex: Sex::Male, names: vec![NewName::full("Kid Tester")], ..Default::default() }, &Origin::human()).unwrap().id;
    s.add_relationship(&CTX, NewRelationship::parent_child(&mum, &kid, LinkKind::Birth), &Origin::human()).unwrap();
    submit(&mut s, "scribe", &json!({"summary": "add", "people": [{"ref": "n", "full_name": "Newcomer Tester"}], "relationships": [{"type": "parent_child", "a": mum, "b": "n"}]})).unwrap();
    (s, mum, kid)
}

#[test]
fn a_persons_links_use_the_common_edge_shape() {
    let (s, mum, kid) = tree();
    let d = serde_json::to_value(person_detail(&s, &mum).unwrap()).unwrap();
    let links = d["links"].as_array().unwrap();
    assert!(links.len() >= 2);
    for l in links {
        let k = keys(l);
        assert!(EDGE.iter().all(|e| k.contains(*e)), "missing a common key: {k:?}");
        assert!(!k.contains("link_id") && !k.contains("rel_type"), "old names: {k:?}");
        assert_eq!(l["type"], "parent_child");
        assert_eq!(l["a"], mum.as_str(), "the parent is a");
        assert_eq!((l["role"].as_str(), l["other_id"] == l["b"]), (Some("child"), true));
    }
    let kid_d = serde_json::to_value(person_detail(&s, &kid).unwrap()).unwrap();
    assert_eq!(kid_d["links"][0]["role"], "parent");
    assert_eq!(kid_d["links"][0]["a"], mum.as_str());
    let st: BTreeSet<&str> = links.iter().map(|l| l["status"].as_str().unwrap()).collect();
    assert_eq!(st, BTreeSet::from(["pending", "verified"]), "status is a word on every link");
}

#[test]
fn the_graph_uses_the_same_edge_keys_and_a_status_word_not_a_flag() {
    let (s, mum, _) = tree();
    let g = neighbourhood(&s, &mum, 1, 1, true).unwrap();
    for l in g["links"].as_array().unwrap() {
        assert_eq!(keys(l), EDGE.iter().map(|k| k.to_string()).collect::<BTreeSet<_>>(), "{l}");
    }
    let st: BTreeSet<&str> = g["links"].as_array().unwrap().iter().map(|l| l["status"].as_str().unwrap()).collect();
    assert_eq!(st, BTreeSet::from(["pending", "verified"]));
    let verified_only = neighbourhood(&s, &mum, 1, 1, false).unwrap();
    assert!(verified_only["links"].as_array().unwrap().iter().all(|l| l["status"] == "verified"));
}
