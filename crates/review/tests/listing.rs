//! list_people: the sex filter really filters, bad values are errors, and the compact options agree with the full list.

use family_review::query::{list_people, ListOpts};
use family_store::*;
use serde_json::Value;

const CTX: AccessContext = AccessContext::Everything;

fn person(s: &mut Store, name: &str, sex: Sex) -> String {
    s.create_person(&CTX, NewPerson { sex, names: vec![NewName::full(name)], ..Default::default() }, &Origin::human()).unwrap().id
}

fn opts(f: impl FnOnce(&mut ListOpts)) -> ListOpts {
    let mut o = ListOpts::default();
    f(&mut o);
    o
}

#[test]
fn sex_filter_accepts_words_and_codes_and_rejects_the_rest() {
    let mut s = Store::open_in_memory().unwrap();
    for (n, x) in [("Ann One", Sex::Female), ("Bea Two", Sex::Female), ("Cy Three", Sex::Male), ("Di Four", Sex::Unknown)] {
        person(&mut s, n, x);
    }
    let total = |s: &Store, sex: &str| list_people(s, &opts(|o| o.sex = Some(sex.into()))).unwrap()["total"].as_u64().unwrap();
    for f in ["F", "f", "female", "Female", "FEMALE"] {
        assert_eq!(total(&s, f), 2, "{f}");
    }
    for m in ["M", "male"] {
        assert_eq!(total(&s, m), 1, "{m}");
    }
    assert_eq!((total(&s, "unknown"), total(&s, "X"), total(&s, "other")), (1, 0, 0));
    let echoed = list_people(&s, &opts(|o| o.sex = Some("female".into()))).unwrap();
    assert_eq!(echoed["filters"]["sex"], "female");
    for bad in ["banana", "woman", "2"] {
        let e = list_people(&s, &opts(|o| o.sex = Some(bad.into()))).unwrap_err().to_string();
        assert!(e.contains("male, female, other, unknown"), "{e}");
    }
    assert!(list_people(&s, &opts(|o| o.status = "everything".into())).is_err());
}

#[test]
fn projection_columns_groups_and_version() {
    let mut s = Store::open_in_memory().unwrap();
    let a = person(&mut s, "Ann One", Sex::Female);
    let before = list_people(&s, &ListOpts::default()).unwrap();
    person(&mut s, "Cy Three", Sex::Male);
    let after = list_people(&s, &ListOpts::default()).unwrap();
    assert_ne!(before["tree_version"], after["tree_version"], "the version must change when the tree does");
    assert_eq!(after["people"].as_array().unwrap().len(), 2);
    assert!(after["people"][0].get("parents").is_some(), "no fields: the full card");

    let p = list_people(&s, &opts(|o| o.fields = Some(vec!["id".into(), "name".into()]))).unwrap();
    assert_eq!(p["people"][0].as_object().unwrap().len(), 2);
    let c = list_people(
        &s,
        &opts(|o| {
            o.fields = Some(vec!["name".into(), "sex".into(), "birth_year".into()]);
            o.columnar = true;
        }),
    )
    .unwrap();
    assert_eq!(c["columns"], serde_json::json!(["name", "sex", "birth_year"]));
    assert_eq!(c["rows"].as_array().unwrap().len(), 2);
    assert!(c.get("people").is_none());
    // Expensive fields still work in a projection.
    let e = list_people(&s, &opts(|o| o.fields = Some(vec!["id".into(), "children_count".into()]))).unwrap();
    assert_eq!(e["people"][0]["children_count"], 0);
    assert!(list_people(&s, &opts(|o| o.fields = Some(vec!["nope".into()]))).is_err());

    let g: Value = list_people(&s, &opts(|o| o.group_by = Some("sex".into()))).unwrap();
    assert_eq!((g["groups"]["female"].as_u64(), g["groups"]["male"].as_u64(), g["total"].as_u64()), (Some(1), Some(1), Some(2)));
    assert!(g.get("people").is_none());
    assert!(list_people(&s, &opts(|o| o.group_by = Some("birth".into()))).is_err());
    let _ = a;
}

#[test]
fn cursor_paging_is_stateless_complete_and_notices_changes() {
    let mut s = Store::open_in_memory().unwrap();
    for i in 0..7 {
        person(&mut s, &format!("Person{i} Test"), Sex::Female);
    }
    // Read everything three at a time, each call using only the previous answer.
    let mut seen: Vec<String> = vec![];
    let mut after: Option<String> = None;
    let mut pages = 0;
    loop {
        let r = list_people(
            &s,
            &opts(|o| {
                o.limit = 3;
                o.after = after.clone();
                o.fields = Some(vec!["id".into()]);
            }),
        )
        .unwrap();
        pages += 1;
        assert_eq!(r["total"], 7);
        if pages == 1 {
            assert!(r.get("changed_since_start").is_none(), "the first page has nothing to compare");
        } else {
            assert_eq!(r["changed_since_start"], false);
        }
        seen.extend(r["people"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()));
        match r["next_cursor"].as_str() {
            Some(c) => after = Some(c.to_string()),
            None => {
                assert_eq!(r["remaining_after_this_page"], 0);
                break;
            }
        }
    }
    assert_eq!(pages, 3);
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!((seen.len(), unique.len()), (7, 7), "nobody twice, nobody missed");
    assert_eq!(seen, unique, "in id order");

    // A change made between pages is reported, and it does not shift the page after the cursor.
    let first = list_people(&s, &opts(|o| o.limit = 3)).unwrap();
    let cursor = first["next_cursor"].as_str().unwrap().to_string();
    let next_before = list_people(&s, &opts(|o| (o.limit = 3, o.after = Some(cursor.clone())).0)).unwrap();
    person(&mut s, "Late Arrival", Sex::Male);
    let next_after = list_people(&s, &opts(|o| (o.limit = 3, o.after = Some(cursor.clone())).0)).unwrap();
    assert_eq!(next_before["changed_since_start"], false);
    assert_eq!(next_after["changed_since_start"], true);
    let ids = |v: &Value| v["people"].as_array().unwrap().iter().map(|p| p["id"].clone()).collect::<Vec<_>>();
    // The new person has the newest id, so it comes at the end; the page after the cursor is exactly the same.
    assert_eq!(ids(&next_after), ids(&next_before));

    // Junk cursors are errors, not an empty or restarted list.
    for bad in ["garbage", "abc~", "~"] {
        assert!(list_people(&s, &opts(|o| o.after = Some(bad.into()))).is_err(), "{bad}");
    }
}
