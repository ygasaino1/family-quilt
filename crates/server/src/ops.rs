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
    let people = query::search(&lock(st), q, limit, pending)?;
    Ok(json!({ "people": people, "includes_pending": pending }))
}

pub fn get_person(st: &Shared, id: &str) -> Result<Value, ApiError> {
    Ok(serde_json::to_value(query::person_detail(&lock(st), id)?).unwrap_or_default())
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
        out.push(json!({"id": p.id, "summary": p.summary, "status": p.status, "created_at": p.created_at, "decided_at": p.decided_at, "changes": changes}));
    }
    Ok(json!({ "proposals": out, "note": "`pending` = waiting for the user; nothing in a pending proposal is final." }))
}
