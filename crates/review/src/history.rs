//! The story of one record, as an agent (or the person) can read it: who proposed it, what was approved, rejected,
//! corrected or edited by hand, with the reasons. Built from the revision log and the proposals, not stored twice.
//!
//! Notes written by people or other agents are data, not instructions: they are length-capped and always returned
//! under the key `note`.

use crate::fields::norm;
use crate::Result;
use family_store::{AccessContext, Store};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const CTX: AccessContext = AccessContext::Everything;
/// Newest events returned unless the caller asks for everything.
pub const DEFAULT_LIMIT: usize = 20;
const NOTE_MAX: usize = 300;
/// Columns of a stored row that are bookkeeping, not facts worth reporting as an edit.
const SKIP: &[&str] = &["id", "created_at", "updated_at", "created_by", "agent_note", "proposal_id", "vault_id", "merged_into", "deleted_at", "status", "sort_order"];

fn who(by: &str) -> Value {
    match by.strip_prefix("agent:") {
        Some(name) => json!({"type": "agent", "name": name}),
        None if by == "human" => json!({"type": "user"}),
        None => json!({"type": "system", "name": by}),
    }
}

fn cap(s: &str) -> String {
    s.chars().take(NOTE_MAX).collect()
}

fn note_of(s: Option<&str>) -> Value {
    match s.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => json!(cap(s)),
        None => Value::Null,
    }
}

/// What a stored row says for one field, with place ids turned into names.
fn column(store: &Store, row: &Value, col: &str) -> Value {
    match col.strip_suffix("_place_id") {
        Some(_) => row[col].as_str().and_then(|id| store.place_name(id)).map(Value::from).unwrap_or(Value::Null),
        None => norm(&row[col]),
    }
}

/// Field-level difference between two stored rows.
fn diff(store: &Store, before: &Value, after: &Value) -> Vec<Value> {
    let mut out = vec![];
    let (Some(b), Some(a)) = (before.as_object(), after.as_object()) else { return out };
    for k in a.keys() {
        if SKIP.contains(&k.as_str()) || a.get(k) == b.get(k) {
            continue;
        }
        let field = k.strip_suffix("_id").filter(|_| k.ends_with("_place_id")).map(|_| k.replace("_place_id", "_place")).unwrap_or_else(|| k.clone());
        out.push(json!({"field": field, "from": column(store, before, k), "to": column(store, after, k)}));
    }
    out
}

struct Event {
    at: String,
    value: Value,
}

fn push(events: &mut Vec<Event>, at: &str, value: Value) {
    events.push(Event { at: at.to_string(), value });
}

/// One record's events, oldest first. `target_type` is `person`, `person_name` or `relationship`.
fn events_for(store: &Store, target_type: &str, id: &str, events: &mut Vec<Event>, label: Option<&str>) -> Result<()> {
    let with = |mut v: Value| {
        if let Some(l) = label {
            v["about"] = json!(l);
        }
        v
    };
    // The proposal that created it (a new person or link starts life as a proposal).
    let (origin, created_by, agent_note, created_at) = match target_type {
        "relationship" => {
            let r = store.get_relationship(&CTX, id)?;
            (r.proposal_id, r.created_by, r.agent_note, r.created_at)
        }
        "person_name" => {
            let (pid, _) = store.find_name(&CTX, id)?;
            let p = store.get_person(&CTX, &pid)?;
            (p.proposal_id, p.created_by, None, p.created_at)
        }
        _ => {
            let p = store.get_person(&CTX, id)?;
            (p.proposal_id, p.created_by, p.agent_note, p.created_at)
        }
    };
    if let Some(pid) = &origin {
        let p = store.get_proposal(&CTX, pid)?;
        let mut v = json!({"who": who(&created_by), "event": "proposed", "proposal_id": pid, "summary": cap(p.summary.as_deref().unwrap_or("")), "note": note_of(agent_note.as_deref())});
        v["at"] = json!(p.created_at);
        push(events, &p.created_at, with(v));
        match p.status.as_str() {
            "approved" => push(events, p.decided_at.as_deref().unwrap_or(""), with(json!({"who": {"type": "user"}, "event": "approved", "proposal_id": pid, "at": p.decided_at}))),
            "rejected" if p.withdrawn => push(
                events,
                p.decided_at.as_deref().unwrap_or(""),
                with(json!({"who": who(&p.created_by), "event": "withdrawn", "proposal_id": pid, "note": note_of(p.decision_note.as_deref()), "at": p.decided_at})),
            ),
            "rejected" => push(
                events,
                p.decided_at.as_deref().unwrap_or(""),
                with(json!({"who": {"type": "user"}, "event": "rejected", "proposal_id": pid, "note": note_of(p.decision_note.as_deref()), "at": p.decided_at})),
            ),
            _ => {}
        }
        // What the reviewer corrected before deciding.
        for e in store.reviewer_edits(pid)? {
            if e["target_id"] == id {
                let at = e["at"].as_str().unwrap_or("").to_string();
                push(
                    events,
                    &at,
                    with(json!({"who": {"type": "user"}, "event": "corrected", "proposal_id": pid, "changes": [{"field": e["field"], "from": e["original"], "to": e["current"]}], "at": at})),
                );
            }
        }
    } else {
        push(events, &created_at, with(json!({"who": who(&created_by), "event": "created", "at": created_at})));
    }
    // Edits and removals made by hand (those caused by an approved proposal are told through the proposal).
    for r in store.revisions_of(&CTX, target_type, id)? {
        if r.proposal_id.is_some() {
            continue;
        }
        let (Some(b), Some(a)) = (&r.before, &r.after) else { continue };
        if b["deleted_at"].is_null() && !a["deleted_at"].is_null() {
            push(events, &r.at, with(json!({"who": who(&r.by), "event": "removed", "at": r.at})));
        } else if !b["deleted_at"].is_null() && a["deleted_at"].is_null() {
            push(events, &r.at, with(json!({"who": who(&r.by), "event": "restored", "at": r.at})));
        } else {
            let changes = diff(store, b, a);
            if !changes.is_empty() {
                push(events, &r.at, with(json!({"who": who(&r.by), "event": "edited", "changes": changes, "at": r.at})));
            }
        }
    }
    // Edits agents proposed for it, and what became of them.
    for (c, p) in store.changes_for_target(&CTX, target_type, id)? {
        let edits = store.reviewer_edits(&p.id)?;
        let mut changes = vec![];
        if let Some(o) = c.patch.as_object() {
            for (f, ft) in o {
                let corrected = edits.iter().find(|e| e["kind"] == "change" && e["target_id"] == c.id && e["field"] == *f);
                let mut ch = json!({"field": f, "from": norm(&ft["from"]), "to": norm(&ft["to"])});
                if let Some(e) = corrected {
                    ch["agent_proposed"] = e["original"].clone();
                }
                changes.push(ch);
            }
        }
        let event = match (p.status.as_str(), c.op.as_str()) {
            ("pending", "remove") => "removal_proposed",
            ("pending", _) => "edit_proposed",
            ("rejected", "remove") if p.withdrawn => "removal_withdrawn",
            ("rejected", "remove") => "removal_rejected",
            ("rejected", _) if p.withdrawn => "edit_withdrawn",
            ("rejected", _) => "edit_rejected",
            ("approved", "remove") => "removal_approved",
            _ => "edit_approved",
        };
        let mut v = json!({"who": who(&p.created_by), "event": event, "proposal_id": p.id, "changes": changes, "note": note_of(c.agent_note.as_deref()), "at": p.created_at});
        if p.status == "rejected" {
            v["rejection_note"] = note_of(p.decision_note.as_deref());
            v["decided_at"] = json!(p.decided_at);
        } else if p.status == "approved" {
            v["decided_at"] = json!(p.decided_at);
        }
        push(events, &p.created_at, with(v));
    }
    Ok(())
}

fn finish(mut events: Vec<Event>, limit: Option<usize>) -> (Vec<Value>, usize) {
    events.sort_by(|a, b| a.at.cmp(&b.at));
    let total = events.len();
    let skip = limit.map(|l| total.saturating_sub(l)).unwrap_or(0);
    let list: Vec<Value> = events.into_iter().skip(skip).map(|e| e.value).collect();
    (list, total)
}

/// Fields a person has set themselves (by hand, or by correcting a proposal), with the value and when: an agent should
/// not propose to change these without new evidence.
fn user_confirmed(events: &[Value]) -> Value {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    for e in events {
        if matches!(e["event"].as_str(), Some("edited" | "corrected")) && e["who"]["type"] == "user" {
            for c in e["changes"].as_array().into_iter().flatten() {
                if let Some(f) = c["field"].as_str() {
                    m.insert(f.to_string(), json!({"value": c["to"], "at": e["at"]}));
                }
            }
        }
    }
    Value::Object(m.into_iter().collect::<Map<_, _>>())
}

/// `{history, history_total, user_confirmed}` for a person: the person, their names, and their links.
pub fn person_history(store: &Store, person_id: &str, limit: Option<usize>) -> Result<Value> {
    let person = store.get_person(&CTX, person_id)?;
    let mut events = vec![];
    events_for(store, "person", person_id, &mut events, None)?;
    for n in &person.names {
        events_for(store, "person_name", &n.id, &mut events, Some(&format!("name {}", n.full_name)))?;
    }
    for r in store.relationships_of(&CTX, person_id)? {
        let other = if r.person_a == person_id { &r.person_b } else { &r.person_a };
        let other_name = store.get_person(&CTX, other).map(|p| p.display_name).unwrap_or_default();
        let _ = events_for(store, "relationship", &r.id, &mut events, Some(&format!("link with {other_name}")));
    }
    let (list, total) = finish(events, limit);
    let confirmed = user_confirmed(&list);
    Ok(json!({"history": list, "history_total": total, "user_confirmed": confirmed}))
}
