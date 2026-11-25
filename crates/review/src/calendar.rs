//! Dates in other calendars. The store keeps one universal calendar (Gregorian); an agent may say
//! a date is Jalali with `"calendar": "jalali"` on a person, relationship or change, and the server
//! converts it here, before anything else looks at the proposal. The agent never converts anything.
//!
//! A full Jalali date maps to one Gregorian day. A Jalali month or year does not map to one Gregorian
//! month or year (1329 runs from 1950-03-21 to 1951-03-20), so it is stored at the same precision
//! using its middle (the Gregorian year or month most of it falls in), marked `about`, and reported.

use crate::FieldError;
use family_store::dates::civil_from_days;
use family_store::jalali;
use serde::Serialize;
use serde_json::{json, Value};

pub const CALENDARS: &[&str] = &["gregorian", "jalali"];
const DATE_FIELDS: &[&str] = &["birth_date", "death_date", "start_date", "end_date"];

#[derive(Debug, Serialize, Clone)]
pub struct Conversion {
    pub path: String,
    pub given: String,
    pub calendar: &'static str,
    pub stored: String,
    /// True when the Jalali value was a month or year, so the stored date is the closest Gregorian one.
    pub approximate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn iso(z: i32) -> String {
    let (y, m, d) = civil_from_days(z);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `(stored gregorian text, approximate, note)` for one Jalali `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
fn convert(s: &str) -> Result<(String, bool, Option<String>), String> {
    let bad = || format!("'{s}' is not a Jalali date: use YYYY, YYYY-MM or YYYY-MM-DD with digits only");
    if !s.is_ascii() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return Err(bad());
    }
    let parts: Vec<&str> = s.split('-').collect();
    let num = |p: &str, len: usize| -> Result<i32, String> {
        if p.len() != len {
            return Err(bad());
        }
        p.parse::<i32>().map_err(|_| bad())
    };
    let (y, m, d) = match parts.as_slice() {
        [y] => (num(y, 4)?, None, None),
        [y, m] => (num(y, 4)?, Some(num(m, 2)? as u32), None),
        [y, m, d] => (num(y, 4)?, Some(num(m, 2)? as u32), Some(num(d, 2)? as u32)),
        _ => return Err(bad()),
    };
    if let Some(why) = jalali::check(y, m.unwrap_or(1), d.unwrap_or(1)) {
        return Err(format!("{why} ({s})"));
    }
    let (first, last, mid) = match (m, d) {
        (Some(m), Some(d)) => {
            let z = jalali::to_days(y, m, d);
            return fits((iso(z), false, None));
        }
        (Some(m), None) => (jalali::to_days(y, m, 1), jalali::to_days(y, m, jalali::days_in_month(y, m)), jalali::to_days(y, m, 15)),
        (None, _) => (jalali::to_days(y, 1, 1), jalali::to_days(y, 12, jalali::days_in_month(y, 12)), jalali::to_days(y, 7, 1)),
    };
    let (gy, gm, _) = civil_from_days(mid);
    let stored = if m.is_some() { format!("{gy:04}-{gm:02}") } else { format!("{gy:04}") };
    let note = format!("Jalali {s} runs from {} to {}; stored as about {stored}", iso(first), iso(last));
    fits((stored, true, Some(note)))
}

fn fits(r: (String, bool, Option<String>)) -> Result<(String, bool, Option<String>), String> {
    let year: i32 = r.0.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0);
    if (1..=9999).contains(&year) {
        Ok(r)
    } else {
        Err("that date is outside the years 0001-9999 the tree can store".into())
    }
}

struct Ctx {
    errs: Vec<FieldError>,
    out: Vec<Conversion>,
}

/// Convert every date in an object that has a `calendar` (and drop the key).
fn object(o: &mut serde_json::Map<String, Value>, path: &str, ctx: &mut Ctx, change: bool) {
    let Some(cal) = o.remove("calendar") else { return };
    let cal_s = match &cal {
        Value::Null => return,
        Value::String(s) => s.trim().to_lowercase(),
        _ => {
            ctx.errs.push(FieldError::new(format!("{path}.calendar"), "wrong_type", format!("must be one of: {}", CALENDARS.join(", "))));
            return;
        }
    };
    if !CALENDARS.contains(&cal_s.as_str()) {
        ctx.errs.push(FieldError::new(format!("{path}.calendar"), "invalid_value", format!("calendar '{}' is not supported; use {}", cal.as_str().unwrap_or(""), CALENDARS.join(" or "))));
        return;
    }
    if cal_s == "gregorian" {
        return;
    }
    let mut notes: Vec<String> = vec![];
    let mut approx_fields: Vec<&str> = vec![];
    for &f in DATE_FIELDS {
        // In a change the new value is `fields.<f>.to`; the old one (`from`) was read from the tree and is Gregorian.
        let slot: Option<&mut Value> = if change { o.get_mut("fields").and_then(|x| x.get_mut(f)).and_then(|x| x.get_mut("to")) } else { o.get_mut(f) };
        let Some(slot) = slot else { continue };
        let Some(given) = slot.as_str().map(str::to_string) else { continue };
        let p = if change { format!("{path}.fields.{f}.to") } else { format!("{path}.{f}") };
        match convert(&given) {
            Ok((stored, approximate, note)) => {
                *slot = json!(stored);
                notes.push(format!("{f} {given} (Jalali) -> {stored}"));
                if approximate {
                    approx_fields.push(f);
                }
                ctx.out.push(Conversion { path: p, given, calendar: "jalali", stored, approximate, note });
            }
            Err(e) => ctx.errs.push(FieldError::new(p, "invalid_date", e)),
        }
    }
    // A person's birth/death qualifier: an approximate conversion is "about" unless the agent said before/after.
    for f in approx_fields {
        let q = f.replace("_date", "_qualifier");
        if !change && (f == "birth_date" || f == "death_date") {
            let cur = o.get(&q).and_then(Value::as_str).unwrap_or("exact");
            if cur == "exact" {
                o.insert(q, json!("about"));
            }
        }
    }
    if !notes.is_empty() {
        let add = format!("[dates given in the Jalali calendar: {}]", notes.join("; "));
        let old = o.get("agent_note").and_then(Value::as_str).unwrap_or("").to_string();
        let joined = if old.is_empty() { add } else { format!("{old} {add}") };
        if joined.chars().count() <= 1000 {
            o.insert("agent_note".into(), json!(joined));
        }
    }
}

/// The body with every non-Gregorian date converted and `calendar` removed, plus what was converted.
pub fn normalize(body: &Value) -> Result<(Value, Vec<Conversion>), Vec<FieldError>> {
    let mut b = body.clone();
    let mut ctx = Ctx { errs: vec![], out: vec![] };
    for (key, change) in [("people", false), ("relationships", false), ("changes", true)] {
        if let Some(arr) = b.get_mut(key).and_then(Value::as_array_mut) {
            for (i, item) in arr.iter_mut().enumerate() {
                if let Some(o) = item.as_object_mut() {
                    object(o, &format!("$.{key}[{i}]"), &mut ctx, change);
                }
            }
        }
    }
    if ctx.errs.is_empty() {
        Ok((b, ctx.out))
    } else {
        Err(ctx.errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_dates_convert_exactly_and_partial_ones_say_so() {
        // 12 Mordad 1329 = 1950-08-03; 1 Farvardin 1403 = 2024-03-20 (leap-year Nowruz).
        assert_eq!(convert("1329-05-12").unwrap(), ("1950-08-03".to_string(), false, None));
        assert_eq!(convert("1403-01-01").unwrap().0, "2024-03-20");
        assert_eq!(convert("1403-12-30").unwrap().0, "2025-03-20");
        let (s, approx, note) = convert("1329").unwrap();
        assert_eq!((s.as_str(), approx), ("1950", true));
        let note = note.unwrap();
        assert!(note.contains("1950-03-21 to 1951-03-21"), "{note}");
        assert_eq!(convert("1329-05").unwrap().0, "1950-08");
        // Nowruz edge: 1 Farvardin starts a year, 29 Esfand ends the previous one.
        assert_eq!(convert("1404-01-01").unwrap().0, "2025-03-21");
        assert_eq!(convert("1403-12-29").unwrap().0, "2025-03-19");
    }

    #[test]
    fn impossible_dates_are_refused() {
        for s in ["1404-12-30", "1329-07-31", "1329-13-01", "1329-00-10", "29-01-01", "1329/05/12", "1329-5-12", ""] {
            assert!(convert(s).is_err(), "{s}");
        }
    }

    #[test]
    fn normalize_rewrites_people_relationships_and_changes() {
        let body = json!({
            "summary": "x",
            "people": [{"ref": "a", "full_name": "A", "birth_date": "1329", "calendar": "jalali", "agent_note": "from the user"}],
            "relationships": [{"type": "partnership", "a": "x", "b": "a", "start_date": "1350-02-03", "calendar": "Jalali"}],
            "changes": [{"target_type": "person", "target_id": "i", "op": "update", "calendar": "jalali", "fields": {"death_date": {"from": "1990", "to": "1370-06-31"}}}]
        });
        let (b, conv) = normalize(&body).unwrap();
        assert_eq!(b["people"][0]["birth_date"], "1950");
        assert_eq!(b["people"][0]["birth_qualifier"], "about");
        assert!(b["people"][0]["agent_note"].as_str().unwrap().starts_with("from the user [dates given in the Jalali calendar"));
        assert!(b["people"][0].get("calendar").is_none());
        assert_eq!(b["relationships"][0]["start_date"], "1971-04-23");
        assert_eq!(b["changes"][0]["fields"]["death_date"]["to"], "1991-09-22");
        assert_eq!(conv.len(), 3);
        // Gregorian and absent calendars change nothing but drop the key.
        let (b2, c2) = normalize(&json!({"people": [{"birth_date": "1950", "calendar": "gregorian"}]})).unwrap();
        assert_eq!(b2["people"][0]["birth_date"], "1950");
        assert!(c2.is_empty() && b2["people"][0].get("calendar").is_none());
        // Errors carry paths.
        let e = normalize(&json!({"people": [{"birth_date": "1329-13-01", "calendar": "jalali"}, {"calendar": "hijri"}]})).unwrap_err();
        assert_eq!(e[0].path, "$.people[0].birth_date");
        assert_eq!(e[1].path, "$.people[1].calendar");
        assert!(e[1].message.contains("gregorian or jalali"));
    }
}
