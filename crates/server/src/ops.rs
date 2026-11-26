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

pub fn search_people(st: &Shared, q: &str, limit: usize, status: &str, offset: usize) -> Result<Value, ApiError> {
    if q.trim().is_empty() {
        return Err(ApiError::bad_request("`q` (part of a name) is required"));
    }
    let store = lock(st);
    let s = query::parse_status(status).map_err(|m| ApiError::bad_request(&m))?;
    let (people, next_cursor) = query::search_page(&store, q, limit, s.is_none(), offset)?;
    let rejected = previously_rejected(&store, q);
    Ok(json!({ "people": people, "status": s.map_or("all", |x| x.as_str()), "next_cursor": next_cursor, "previously_rejected": rejected, "tree_version": query::tree_version(&store)? }))
}

/// Page through people with filters (the search needs a name; this does not).
pub fn list_people(st: &Shared, o: &query::ListOpts) -> Result<Value, ApiError> {
    let store = lock(st);
    Ok(query::list_people(&store, o)?)
}

/// One of the agent's own proposals in full: its people, links and edits, with ids and current status.
pub fn get_proposal(st: &Shared, agent: &str, id: &str) -> Result<Value, ApiError> {
    let store = lock(st);
    let p = store.get_proposal(&CTX, id)?;
    let mine = format!("agent:{}", family_review::clean_agent_name(agent));
    if p.created_by != mine {
        return Err(ApiError::new(axum::http::StatusCode::NOT_FOUND, "not_found", "You have no proposal with that id (see proposals)."));
    }
    let people: Vec<Value> = store.people_of_proposal(&CTX, id)?.iter().filter_map(|i| query::card(&store, i).ok()).collect();
    let mut links = vec![];
    for lid in store.links_of_proposal(&CTX, id)? {
        let r = store.get_relationship(&CTX, &lid)?;
        links.push(json!({"id": r.id, "type": r.rel_type, "a": r.person_a, "b": r.person_b, "kind": r.kind, "subtype": r.subtype, "status": r.status}));
    }
    let changes: Vec<Value> =
        store.changes_of(&CTX, id)?.into_iter().map(|c| json!({"id": c.id, "target_type": c.target_type, "target_id": c.target_id, "op": c.op, "patch": c.patch, "status": c.status})).collect();
    Ok(json!({
        "id": p.id, "summary": p.summary, "status": if p.withdrawn { "withdrawn" } else { p.status.as_str() },
        "created_at": p.created_at, "decided_at": p.decided_at, "rejection_note": p.decision_note,
        "people": people, "relationships": links, "changes": changes,
        "note": "People of a rejected or withdrawn proposal are discarded and no longer exist."
    }))
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

pub fn find_relationship(st: &Shared, a: &str, b: &str, status: &str) -> Result<Value, ApiError> {
    Ok(serde_json::to_value(query::relationship(&lock(st), a, b, pending_of(status)?, false)?).unwrap_or_default())
}

pub fn get_graph(st: &Shared, focus: &str, up: u32, down: u32, status: &str) -> Result<Value, ApiError> {
    Ok(query::neighbourhood(&lock(st), focus, up, down, pending_of(status)?)?)
}

/// `verified` excludes unverified people and links; `pending`/`all` include them.
fn pending_of(status: &str) -> Result<bool, ApiError> {
    Ok(query::parse_status(status).map_err(|m| ApiError::bad_request(&m))?.is_none_or(|s| s != family_store::Status::Verified))
}

pub fn create_proposal(st: &Shared, agent: &str, body: &Value) -> Result<Value, ApiError> {
    let r = submit::submit(&mut lock(st), agent, body)?;
    Ok(serde_json::to_value(r).unwrap_or_default())
}

/// An agent takes back its own pending proposal.
pub fn withdraw_proposal(st: &Shared, agent: &str, id: &str, reason: Option<&str>) -> Result<Value, ApiError> {
    let by = format!("agent:{}", family_review::clean_agent_name(agent));
    let r = family_review::apply::withdraw(&mut lock(st), id, &by, reason)?;
    Ok(json!({"id": id, "status": "withdrawn", "note": "Nothing of it will appear for review. It does not count as a rejection by the user.", "affected_proposals": r.affected_proposals}))
}

/// The agent's own proposals, newest first, one page. `status` is `pending`, `decided`, `withdrawn` or `all`;
/// `before` is the id of the last proposal of the previous page; `next_cursor` is `Null` on the last page.
pub fn list_my_proposals(st: &Shared, agent: &str, status: &str, before: Option<&str>, limit: usize) -> Result<Value, ApiError> {
    let store = lock(st);
    let created_by = format!("agent:{}", family_review::clean_agent_name(agent));
    let status = match status.trim().to_lowercase().as_str() {
        "" | "all" => None,
        "pending" => Some("pending"),
        "decided" => Some("decided"),
        "withdrawn" => Some("withdrawn"),
        other => return Err(ApiError::bad_request(&format!("'{other}' is not a status: use one of pending, decided, withdrawn, all"))),
    };
    let limit = if limit == 0 { 20 } else { limit.min(100) };
    let rows = store.proposals_page(&CTX, &created_by, before, limit + 1)?;
    let next_cursor = rows.get(limit).map(|p| p.id.clone());
    let mut out = vec![];
    for p in rows.into_iter().take(limit) {
        if status.is_some() && status != Some(if p.withdrawn { "withdrawn" } else { p.status.as_str() }) {
            continue;
        }
        let changes: Vec<Value> =
            store.changes_of(&CTX, &p.id)?.into_iter().map(|c| json!({"id": c.id, "target_type": c.target_type, "target_id": c.target_id, "op": c.op, "status": c.status})).collect();
        let corrections: Vec<Value> = store
            .reviewer_edits(&p.id)?
            .into_iter()
            .map(|e| json!({"kind": e["kind"], "target_id": e["target_id"], "field": e["field"], "you_proposed": e["original"], "approved_as": e["current"], "note": e["note"]}))
            .collect();
        out.push(json!({"id": p.id, "summary": p.summary, "status": if p.withdrawn { "withdrawn" } else { p.status.as_str() }, "created_at": p.created_at, "decided_at": p.decided_at, "rejection_note": p.decision_note, "reviewer_corrections": corrections, "changes": changes}));
    }
    Ok(json!({ "proposals": out, "next_cursor": next_cursor, "filters": {"status": status.unwrap_or("all"), "mine": true}, "note": "`pending` = waiting for the user; nothing in a pending proposal is final. Read one in full with `proposals` and its `id`." }))
}
