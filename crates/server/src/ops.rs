//! The agent operations, written once. The HTTP routes and the MCP tools both call these, so both
//! doors have exactly the same powers: read, search, relationship, graph, create proposal,
//! list own proposals. Nothing here edits, deletes, approves or rejects.

use crate::{ApiError, Shared};
use family_review::{query, submit};
use family_store::AccessContext;
use serde_json::{json, Value};

const CTX: AccessContext = AccessContext::Everything;

fn lock(st: &Shared) -> std::sync::MutexGuard<'_, family_store::Store> {
    st.store.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn search_people(st: &Shared, q: &str, limit: usize, pending: bool) -> Result<Value, ApiError> {
    if q.trim().is_empty() {
        return Err(ApiError::bad_request("`q` (part of a name) is required"));
    }
    let store = lock(st);
    let people = query::search(&store, q, limit, pending)?;
    let rejected = previously_rejected(&store, q);
    Ok(json!({ "people": people, "includes_pending": pending, "previously_rejected": rejected }))
}

pub fn get_person(st: &Shared, id: &str) -> Result<Value, ApiError> {
    get_person_history(st, id, false)
}

/// The person with their links and open suggestions, plus `history` (the newest 20 events, or all with `all`),
/// `history_total` and `user_confirmed`.
pub fn get_person_history(st: &Shared, id: &str, all: bool) -> Result<Value, ApiError> {
    let store = lock(st);
    let mut v = serde_json::to_value(query::person_detail(&store, id)?).unwrap_or_default();
    let h = family_review::history::person_history(&store, id, if all { None } else { Some(family_review::history::DEFAULT_LIMIT) })?;
    if let (Some(o), Some(h)) = (v.as_object_mut(), h.as_object()) {
        for (k, x) in h {
            o.insert(k.clone(), x.clone());
        }
    }
    Ok(v)
}

/// Rejected proposals that asked for a person like this: so an agent does not look for them in the tree and then
/// propose them again.
fn previously_rejected(store: &family_store::Store, q: &str) -> Vec<Value> {
    let needle = q.trim().to_lowercase();
    let mut out = vec![];
    for (p, fps) in store.rejected_fingerprints(&CTX, 300).unwrap_or_default() {
        for f in &fps {
            let mut parts = f.split('|');
            if parts.next() != Some("p") {
                continue;
            }
            let (name, _sex, year) = (parts.next().unwrap_or(""), parts.next(), parts.next().unwrap_or(""));
            if !needle.is_empty() && name.contains(&needle) {
                out.push(json!({"proposal_id": p.id, "name": name, "birth_year": year, "rejected_at": p.decided_at, "note": p.decision_note, "hint": "The user rejected a proposal that included this person. Do not propose them again unless the note says why it can be fixed."}));
            }
        }
        if out.len() >= 5 {
            break;
        }
    }
    out
}

pub fn find_relationship(st: &Shared, a: &str, b: &str, pending: bool) -> Result<Value, ApiError> {
    Ok(serde_json::to_value(query::relationship(&lock(st), a, b, pending, false)?).unwrap_or_default())
}

pub fn get_graph(st: &Shared, focus: &str, up: u32, down: u32, pending: bool) -> Result<Value, ApiError> {
    Ok(query::neighbourhood(&lock(st), focus, up, down, pending)?)
}

pub fn create_proposal(st: &Shared, agent: &str, body: &Value) -> Result<Value, ApiError> {
    let r = submit::submit(&mut lock(st), agent, body)?;
    Ok(serde_json::to_value(r).unwrap_or_default())
}

pub fn list_my_proposals(st: &Shared, agent: &str) -> Result<Value, ApiError> {
    let store = lock(st);
    let created_by = format!("agent:{}", family_review::clean_agent_name(agent));
    let mut out = vec![];
    for p in store.list_proposals(&CTX, None, Some(&created_by), 50)? {
        let changes: Vec<Value> =
            store.changes_of(&CTX, &p.id)?.into_iter().map(|c| json!({"id": c.id, "target_type": c.target_type, "target_id": c.target_id, "op": c.op, "status": c.status})).collect();
        let corrections: Vec<Value> = store
            .reviewer_edits(&p.id)?
            .into_iter()
            .map(|e| json!({"kind": e["kind"], "target_id": e["target_id"], "field": e["field"], "you_proposed": e["original"], "approved_as": e["current"]}))
            .collect();
        out.push(json!({"id": p.id, "summary": p.summary, "status": p.status, "created_at": p.created_at, "decided_at": p.decided_at, "rejection_note": p.decision_note, "reviewer_corrections": corrections, "changes": changes}));
    }
    Ok(json!({ "proposals": out, "note": "`pending` = waiting for the user; nothing in a pending proposal is final." }))
}
