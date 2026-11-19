//! The whitelist of fields an agent may propose to change (section 19), how to read the current
//! value of each, and how to apply an accepted value through the normal store calls.
//!
//! Not in any list (so rejected with a field-level error): `id`, `status`, `vault_id`, `created_by`,
//! `proposal_id`, `merged_into`, timestamps, and everything about deleting or merging people.

use family_store::{AccessContext, Field, FuzzyDate, NamePatch, NameType, PartialDate, Person, PersonName, PersonPatch, Qualifier, RelType, Relationship, RelationshipPatch, Sex, Store};
use family_store::{LinkKind, PartnershipState};
use serde_json::Value;

/// Reads the current value of one field of a record.
pub type FieldReader = Box<dyn Fn(&str) -> Value>;

pub const PERSON_FIELDS: &[&str] = &["sex", "is_living", "birth_date", "birth_qualifier", "birth_place", "death_date", "death_qualifier", "death_place", "note"];
pub const NAME_FIELDS: &[&str] = &["name_type", "given", "family", "full_name", "script"];
pub const PARENT_CHILD_FIELDS: &[&str] = &["kind", "start_date", "end_date", "place", "note"];
pub const PARTNERSHIP_FIELDS: &[&str] = &["subtype", "start_date", "end_date", "place", "note"];

pub fn allowed_fields(target_type: &str, rel_type: Option<RelType>) -> &'static [&'static str] {
    match (target_type, rel_type) {
        ("person", _) => PERSON_FIELDS,
        ("person_name", _) => NAME_FIELDS,
        ("relationship", Some(RelType::Partnership)) => PARTNERSHIP_FIELDS,
        ("relationship", _) => PARENT_CHILD_FIELDS,
        _ => &[],
    }
}

/// Fields that may be cleared (`to: null`).
fn clearable(field: &str) -> bool {
    !matches!(field, "sex" | "is_living" | "kind" | "subtype" | "full_name" | "name_type")
}

/// Trim strings and treat "" as null, so `""` and `null` compare equal.
pub fn norm(v: &Value) -> Value {
    match v {
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                Value::Null
            } else {
                Value::String(t.to_string())
            }
        }
        other => other.clone(),
    }
}

fn s(v: Option<String>) -> Value {
    v.map(Value::String).unwrap_or(Value::Null)
}

pub fn current_person_field(p: &Person, field: &str) -> Value {
    match field {
        "sex" => Value::from(p.sex.as_str()),
        "is_living" => Value::Bool(p.is_living),
        "birth_date" => s(p.birth.map(|d| d.date.to_string())),
        "birth_qualifier" => s(p.birth.map(|d| d.qualifier.as_str().to_string())),
        "birth_place" => s(p.birth_place.as_ref().map(|x| x.name.clone())),
        "death_date" => s(p.death.map(|d| d.date.to_string())),
        "death_qualifier" => s(p.death.map(|d| d.qualifier.as_str().to_string())),
        "death_place" => s(p.death_place.as_ref().map(|x| x.name.clone())),
        "note" => s(p.note.clone()),
        _ => Value::Null,
    }
}

pub fn current_name_field(n: &PersonName, field: &str) -> Value {
    match field {
        "name_type" => Value::from(n.name_type.as_str()),
        "given" => s(n.given.clone()),
        "family" => s(n.family.clone()),
        "full_name" => Value::from(n.full_name.clone()),
        "script" => s(n.script.clone()),
        _ => Value::Null,
    }
}

pub fn current_rel_field(r: &Relationship, field: &str) -> Value {
    match field {
        "kind" => s(r.kind.map(|k| k.as_str().to_string())),
        "subtype" => s(r.subtype.map(|k| k.as_str().to_string())),
        "start_date" => s(r.start_date.clone()),
        "end_date" => s(r.end_date.clone()),
        "place" => s(r.place.as_ref().map(|p| p.name.clone())),
        "note" => s(r.note.clone()),
        _ => Value::Null,
    }
}

/// Check one proposed new value. `Err` is a plain-language message for the agent.
pub fn check_to(field: &str, rel_type: Option<RelType>, to: &Value) -> std::result::Result<(), String> {
    let to = norm(to);
    if to.is_null() {
        return if clearable(field) { Ok(()) } else { Err(format!("'{field}' cannot be cleared")) };
    }
    let text = |max: usize| -> std::result::Result<&str, String> {
        let t = to.as_str().ok_or_else(|| format!("'{field}' must be a string"))?;
        if t.chars().count() > max {
            return Err(format!("'{field}' is longer than {max} characters"));
        }
        Ok(t)
    };
    match field {
        "sex" => Sex::parse(text(1)?).map(|_| ()).map_err(|e| e.to_string()),
        "is_living" => to.as_bool().map(|_| ()).ok_or_else(|| "'is_living' must be true or false".to_string()),
        "birth_date" | "death_date" | "start_date" | "end_date" => PartialDate::parse(text(10)?).map(|_| ()).map_err(|e| format!("{e} (use YYYY, YYYY-MM or YYYY-MM-DD)")),
        "birth_qualifier" | "death_qualifier" => Qualifier::parse(text(10)?).map(|_| ()).map_err(|e| e.to_string()),
        "birth_place" | "death_place" | "place" | "given" | "family" | "full_name" | "script" => text(300).map(|_| ()),
        "note" => text(20_000).map(|_| ()),
        "name_type" => NameType::parse(text(20)?).map(|_| ()).map_err(|e| e.to_string()),
        "kind" => {
            if rel_type == Some(RelType::Partnership) {
                return Err("only parent-child links have a 'kind'".into());
            }
            LinkKind::parse(text(20)?).map(|_| ()).map_err(|e| e.to_string())
        }
        "subtype" => {
            if rel_type == Some(RelType::ParentChild) {
                return Err("only partnerships have a 'subtype'".into());
            }
            PartnershipState::parse(text(20)?).map(|_| ()).map_err(|e| e.to_string())
        }
        _ => Err(format!("'{field}' cannot be edited")),
    }
}

fn str_of(v: &Value) -> Option<String> {
    norm(v).as_str().map(str::to_string)
}

fn text_field(v: &Value) -> Field<String> {
    match str_of(v) {
        Some(t) => Field::Set(t),
        None => Field::Clear,
    }
}

/// Apply the given `(field, new value)` pairs to a person. Errors come back as store errors.
pub fn apply_person(store: &mut Store, ctx: &AccessContext, id: &str, vals: &[(&str, &Value)], by: &str) -> family_store::Result<()> {
    let cur = store.get_person(ctx, id)?;
    let get = |f: &str| vals.iter().find(|(k, _)| *k == f).map(|(_, v)| *v);
    let mut patch = PersonPatch::default();
    if let Some(v) = get("sex") {
        patch.sex = Some(Sex::parse(v.as_str().unwrap_or(""))?);
    }
    if let Some(v) = get("is_living") {
        patch.is_living = v.as_bool();
    }
    for (date_f, qual_f, cur_d) in [("birth_date", "birth_qualifier", cur.birth), ("death_date", "death_qualifier", cur.death)] {
        let (d, q) = (get(date_f), get(qual_f));
        if d.is_none() && q.is_none() {
            continue;
        }
        let date = match d {
            Some(v) => str_of(v),
            None => cur_d.map(|x| x.date.to_string()),
        };
        let qual = match q {
            Some(v) => str_of(v).map(|t| Qualifier::parse(&t)).transpose()?.unwrap_or_default(),
            None => cur_d.map(|x| x.qualifier).unwrap_or_default(),
        };
        let new = match date {
            Some(ds) => Field::Set(FuzzyDate::parse(&ds, qual)?),
            None => Field::Clear,
        };
        if date_f == "birth_date" {
            patch.birth = new;
        } else {
            patch.death = new;
        }
    }
    if let Some(v) = get("birth_place") {
        patch.birth_place = text_field(v);
    }
    if let Some(v) = get("death_place") {
        patch.death_place = text_field(v);
    }
    if let Some(v) = get("note") {
        patch.note = text_field(v);
    }
    store.update_person(ctx, id, patch, by)?;
    Ok(())
}

pub fn apply_name(store: &mut Store, ctx: &AccessContext, name_id: &str, vals: &[(&str, &Value)], by: &str) -> family_store::Result<()> {
    let get = |f: &str| vals.iter().find(|(k, _)| *k == f).map(|(_, v)| *v);
    let mut patch = NamePatch::default();
    if let Some(v) = get("name_type") {
        patch.name_type = Some(NameType::parse(v.as_str().unwrap_or(""))?);
    }
    if let Some(v) = get("given") {
        patch.given = text_field(v);
    }
    if let Some(v) = get("family") {
        patch.family = text_field(v);
    }
    if let Some(v) = get("full_name") {
        patch.full_name = str_of(v);
    }
    if let Some(v) = get("script") {
        patch.script = text_field(v);
    }
    store.update_name(ctx, name_id, patch, by)?;
    Ok(())
}

pub fn apply_relationship(store: &mut Store, ctx: &AccessContext, id: &str, vals: &[(&str, &Value)], by: &str) -> family_store::Result<()> {
    let get = |f: &str| vals.iter().find(|(k, _)| *k == f).map(|(_, v)| *v);
    let mut patch = RelationshipPatch::default();
    if let Some(v) = get("kind") {
        patch.kind = Some(LinkKind::parse(v.as_str().unwrap_or(""))?);
    }
    if let Some(v) = get("subtype") {
        patch.subtype = Some(PartnershipState::parse(v.as_str().unwrap_or(""))?);
    }
    for (f, slot) in [("start_date", &mut patch.start_date), ("end_date", &mut patch.end_date)] {
        if let Some(v) = get(f) {
            *slot = match str_of(v) {
                Some(t) => Field::Set(PartialDate::parse(&t)?),
                None => Field::Clear,
            };
        }
    }
    if let Some(v) = get("place") {
        patch.place = text_field(v);
    }
    if let Some(v) = get("note") {
        patch.note = text_field(v);
    }
    store.update_relationship(ctx, id, patch, by)?;
    Ok(())
}
