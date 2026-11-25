//! Agents send Jalali dates with `calendar: "jalali"`; the tree stores Gregorian and reports what it did.

use family_review::submit::submit;
use family_review::ReviewError;
use family_store::*;
use serde_json::json;

const CTX: AccessContext = AccessContext::Everything;

#[test]
fn jalali_dates_are_converted_stored_and_reported() {
    let mut s = Store::open_in_memory().unwrap();
    let kim = s
        .create_person(&CTX, NewPerson { sex: Sex::Female, names: vec![NewName::full("Kim Tester")], birth: Some(FuzzyDate::exact("1960").unwrap()), ..Default::default() }, &Origin::human())
        .unwrap()
        .id;
    let r = submit(
        &mut s,
        "scribe",
        &json!({
            "summary": "Add Reza and his marriage",
            "people": [
                {"ref": "r", "full_name": "Reza Tester", "sex": "M", "birth_date": "1329-05-12", "calendar": "jalali"},
                {"ref": "y", "full_name": "Year Only", "birth_date": "1329", "calendar": "jalali"}
            ],
            "relationships": [{"type": "partnership", "a": kim, "b": "r", "subtype": "married", "start_date": "1350-02-03", "calendar": "jalali"}],
            "changes": [{"target_type": "person", "target_id": kim, "op": "update", "calendar": "jalali", "fields": {"death_date": {"from": null, "to": "1403-01-01"}}}]
        }),
    )
    .unwrap();
    assert_eq!(r.date_conversions.len(), 4);
    assert!(r.date_conversions.iter().any(|c| c.given == "1329-05-12" && c.stored == "1950-08-03" && !c.approximate));
    assert!(r.date_conversions.iter().any(|c| c.given == "1329" && c.stored == "1950" && c.approximate));

    let rid = &r.people.iter().find(|p| p.ref_ == "r").unwrap().id;
    let reza = s.get_person(&CTX, rid).unwrap();
    assert_eq!(reza.birth.unwrap().date.to_string(), "1950-08-03");
    assert!(reza.agent_note.unwrap().contains("12") && reza.birth.unwrap().qualifier == Qualifier::Exact);
    let yid = &r.people.iter().find(|p| p.ref_ == "y").unwrap().id;
    let y = s.get_person(&CTX, yid).unwrap().birth.unwrap();
    assert_eq!((y.date.to_string().as_str(), y.qualifier), ("1950", Qualifier::About));
    let link = s.get_relationship(&CTX, &r.relationships[0].id).unwrap();
    assert_eq!(link.start_date.as_deref(), Some("1971-04-23"));
    let c = &s.changes_of(&CTX, &r.proposal_id).unwrap()[0];
    assert_eq!(c.patch["death_date"]["to"], "2024-03-20");
}

#[test]
fn unsupported_calendars_and_impossible_dates_are_field_errors() {
    let mut s = Store::open_in_memory().unwrap();
    let bad = |s: &mut Store, v: serde_json::Value| match submit(s, "scribe", &v) {
        Err(ReviewError::Invalid(e)) => e.into_iter().map(|e| (e.path, e.message)).collect::<Vec<_>>(),
        _ => panic!("expected field errors"),
    };
    let e = bad(&mut s, json!({"summary": "x", "people": [{"ref": "a", "full_name": "A", "birth_date": "1329-05-12", "calendar": "hijri"}]}));
    assert_eq!(e[0].0, "$.people[0].calendar");
    assert!(e[0].1.contains("gregorian or jalali"));
    let e = bad(&mut s, json!({"summary": "x", "people": [{"ref": "a", "full_name": "A", "birth_date": "1404-12-30", "calendar": "jalali"}]}));
    assert_eq!(e[0].0, "$.people[0].birth_date");
    assert!(e[0].1.contains("29 days"), "{}", e[0].1);
    // Nothing was written.
    assert!(s.search_people(&CTX, &SearchQuery { text: "a".into(), status: None, limit: 10 }).unwrap().is_empty());
}
