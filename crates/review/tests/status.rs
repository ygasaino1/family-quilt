//! One word for "how much of the tree to look at": `status` = verified (default), pending or all.

use family_review::query::{search_cards, Scope};
use family_review::submit::submit;
use family_store::*;
use serde_json::json;

const CTX: AccessContext = AccessContext::Everything;

fn tree() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.create_person(&CTX, NewPerson { sex: Sex::Female, names: vec![NewName::full("Kim Tester")], ..Default::default() }, &Origin::human()).unwrap();
    submit(&mut s, "scribe", &json!({"summary": "add", "people": [{"ref": "k", "full_name": "Kimberly Newcomer"}]})).unwrap();
    s
}

#[test]
fn scope_parses_the_three_words_and_nothing_else() {
    assert_eq!(Scope::parse("verified").unwrap(), Scope::Verified);
    assert_eq!(Scope::parse("").unwrap(), Scope::Verified, "no status means verified");
    assert_eq!(Scope::parse("pending").unwrap(), Scope::Pending);
    assert_eq!(Scope::parse("all").unwrap(), Scope::All);
    assert_eq!(Scope::parse("ALL").unwrap(), Scope::All);
    let e = Scope::parse("everything").unwrap_err().to_string();
    assert!(e.contains("verified, pending, all"), "{e}");
}

#[test]
fn search_looks_at_verified_pending_or_all() {
    let s = tree();
    let names = |sc| search_cards(&s, "kim", 10, sc).unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(Scope::Verified), ["Kim Tester"]);
    assert_eq!(names(Scope::Pending), ["Kimberly Newcomer"]);
    assert_eq!(names(Scope::All).len(), 2);
}
