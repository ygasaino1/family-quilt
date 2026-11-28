//! History pages backwards with a cursor: every event once, nothing repeated when new events arrive meanwhile,
//! and what the user set by hand is reported whatever page you are on.

use family_review::history::{person_history, DEFAULT_LIMIT};
use family_store::*;
use serde_json::Value;

const CTX: AccessContext = AccessContext::Everything;

/// Kim, whose sex the user corrected long ago, then 45 later edits of her note.
fn busy() -> (Store, String) {
    let mut s = Store::open_in_memory().unwrap();
    let kim = s.create_person(&CTX, NewPerson { sex: Sex::Unknown, names: vec![NewName::full("Kim Tester")], ..Default::default() }, &Origin::human()).unwrap().id;
    s.update_person(&CTX, &kim, PersonPatch { sex: Some(Sex::Female), ..Default::default() }, "human").unwrap();
    for i in 0..45 {
        s.update_person(&CTX, &kim, PersonPatch { note: Field::Set(format!("note {i}")), ..Default::default() }, "human").unwrap();
    }
    (s, kim)
}

fn notes(h: &Value) -> Vec<String> {
    h["history"].as_array().unwrap().iter().filter_map(|e| e["changes"][0]["to"].as_str().map(String::from)).collect()
}

#[test]
fn pages_cover_the_whole_history_once_newest_first_and_each_page_reads_oldest_to_newest() {
    let (s, kim) = busy();
    let whole = person_history(&s, &kim, 1000, None).unwrap();
    let total = whole["history_total"].as_u64().unwrap() as usize;
    assert!(total >= 46 && whole["history_next_cursor"].is_null());

    let (mut seen, mut pages, mut before): (Vec<Value>, usize, Option<String>) = (vec![], 0, None);
    loop {
        let h = person_history(&s, &kim, 10, before.as_deref()).unwrap();
        assert_eq!(h["history_total"].as_u64().unwrap() as usize, total);
        let page = h["history"].as_array().unwrap().clone();
        assert!(page.len() <= 10 && !page.is_empty());
        // Older pages are prepended, so the final list reads oldest to newest like the whole history.
        seen = page.into_iter().chain(seen).collect();
        pages += 1;
        match h["history_next_cursor"].as_str() {
            Some(c) => before = Some(c.to_string()),
            None => break,
        }
        assert!(pages < 50);
    }
    assert_eq!(seen, whole["history"].as_array().unwrap().clone(), "every event once, in order");
    assert_eq!(pages, total.div_ceil(10));
}

#[test]
fn the_default_is_the_newest_twenty_and_says_where_older_ones_start() {
    let (s, kim) = busy();
    let h = person_history(&s, &kim, DEFAULT_LIMIT, None).unwrap();
    assert_eq!(h["history"].as_array().unwrap().len(), 20);
    assert_eq!(notes(&h).last().unwrap(), "note 44");
    assert!(h["history_next_cursor"].is_string());
}

#[test]
fn new_events_between_pages_do_not_shift_or_repeat_older_ones() {
    let (mut s, kim) = busy();
    let first = person_history(&s, &kim, 10, None).unwrap();
    let before = first["history_next_cursor"].as_str().unwrap().to_string();
    for i in 0..7 {
        s.update_person(&CTX, &kim, PersonPatch { note: Field::Set(format!("later {i}")), ..Default::default() }, "human").unwrap();
    }
    let second = person_history(&s, &kim, 10, Some(&before)).unwrap();
    let (a, b) = (notes(&first), notes(&second));
    assert!(a.iter().all(|n| !b.contains(n)), "an event from page 1 came back on page 2: {a:?} / {b:?}");
    // Page 2 ends exactly where page 1 began: note 34 is the newest of it (page 1 holds notes 35..44).
    assert_eq!(b.last().unwrap(), "note 34");
    assert!(b.iter().all(|n| !n.starts_with("later")));
}

#[test]
fn what_the_user_set_is_reported_on_every_page_not_only_the_page_that_holds_it() {
    let (s, kim) = busy();
    let newest = person_history(&s, &kim, 5, None).unwrap();
    assert!(!notes(&newest).iter().any(|n| n.is_empty()));
    // The sex correction is 46 events back, far outside this page, but it is still the user's.
    assert_eq!(newest["user_confirmed"]["sex"]["value"], "female", "{}", newest["user_confirmed"]);
    assert!(newest["user_confirmed"].get("note").is_some());
}

#[test]
fn a_made_up_or_outdated_cursor_is_refused_with_what_to_do() {
    let (s, kim) = busy();
    for junk in ["", "abc", "h~x~2026", "h~9999~2026-01-01T00:00:00Z", "h~3~not-the-time"] {
        let e = person_history(&s, &kim, 10, Some(junk)).unwrap_err().to_string();
        assert!(e.contains("not a cursor") || e.contains("no longer matches"), "{junk}: {e}");
    }
}

#[test]
fn limits_are_checked() {
    let (s, kim) = busy();
    assert_eq!(person_history(&s, &kim, 0, None).unwrap()["history"].as_array().unwrap().len(), 20, "0 means the default");
    assert_eq!(person_history(&s, &kim, 5000, None).unwrap()["history"].as_array().unwrap().len() as u64, person_history(&s, &kim, 200, None).unwrap()["history_total"].as_u64().unwrap().min(200));
}
