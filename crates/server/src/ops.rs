//! The agent operations, written once. The HTTP routes and the MCP tools both call these, so both
//! doors have exactly the same powers: read, search, relationship, graph, create proposal,
//! list own proposals. Nothing here edits, deletes, approves or rejects.

use crate::{ApiError, Shared};
use family_review::query::{self, Scope};
use family_review::submit;
use family_store::AccessContext;
use serde_json::{json, Value};

const CTX: AccessContext = AccessContext::Everything;

fn lock(st: &Shared) -> std::sync::MutexGuard<'_, family_store::Store> {
    st.store.lock().unwrap_or_else(|e| e.into_inner())
}

/// A change's `{field: {from, to}}` with values in the words agents use.
fn agent_patch(patch: &Value) -> Value {
    match patch.as_object() {
        Some(o) => Value::Object(o.iter().map(|(f, ft)| (f.clone(), json!({"from": query::agent_value(f, &ft["from"]), "to": query::agent_value(f, &ft["to"])}))).collect()),
        None => patch.clone(),
    }
}

pub fn search_people(st: &Shared, q: &str, limit: usize, scope: Scope, after: Option<&str>) -> Result<Value, ApiError> {
    if q.trim().is_empty() {
        return Err(ApiError::bad_request("`q` (part of a name) is required"));
    }
    let store = lock(st);
    let page = query::search_page(&store, q, limit, scope, after)?;
    let people: Vec<Value> = page.people.iter().map(|p| query::card(&store, &p.id)).collect::<Result<_, _>>()?;
    let rejected = previously_rejected(&store, q);
    let mut out = json!({ "people": people, "status": scope.as_str(), "next_cursor": page.next_cursor, "previously_rejected": rejected, "tree_version": query::tree_version(&store)? });
    if let Some(changed) = page.changed_since_start {
        out["changed_since_start"] = json!(changed);
    }
    Ok(out)
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
        let theirs = p.created_by.strip_prefix("agent:").unwrap_or(&p.created_by);
        let me = family_review::clean_agent_name(agent);
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "not_yours",
            &format!("That proposal was sent by '{theirs}' and you are '{me}'. Proposals are private to the agent that sent them; if it was yours, your client name has changed since (it has to stay the same). List your own proposals to find yours."),
        ));
    }
    let people: Vec<Value> = store.people_of_proposal(&CTX, id)?.iter().filter_map(|i| query::card(&store, i).ok()).collect();
    let mut links = vec![];
    for lid in store.links_of_proposal(&CTX, id)? {
        let r = store.get_relationship(&CTX, &lid)?;
        links.push(json!({"id": r.id, "type": r.rel_type, "a": r.person_a, "b": r.person_b, "kind": r.kind, "subtype": r.subtype, "status": r.status}));
    }
    let changes: Vec<Value> = store
        .changes_of(&CTX, id)?
        .into_iter()
        .map(|c| json!({"id": c.id, "target_type": c.target_type, "target_id": c.target_id, "op": c.op, "patch": agent_patch(&c.patch), "status": c.status}))
        .collect();
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

pub fn find_relationship(st: &Shared, a: &str, b: &str, scope: Scope) -> Result<Value, ApiError> {
    let pending = scope.whole_tree()?;
    Ok(serde_json::to_value(query::relationship(&lock(st), a, b, pending, false)?).unwrap_or_default())
}

pub fn get_graph(st: &Shared, focus: &str, up: u32, down: u32, scope: Scope) -> Result<Value, ApiError> {
    let pending = scope.whole_tree()?;
    Ok(query::neighbourhood(&lock(st), focus, up, down, pending)?)
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
            .map(|e| json!({"kind": e["kind"], "target_id": e["target_id"], "field": e["field"], "you_proposed": query::agent_value(e["field"].as_str().unwrap_or(""), &e["original"]), "approved_as": query::agent_value(e["field"].as_str().unwrap_or(""), &e["current"]), "note": e["note"]}))
            .collect();
        out.push(json!({"id": p.id, "summary": p.summary, "status": if p.withdrawn { "withdrawn" } else { p.status.as_str() }, "created_at": p.created_at, "decided_at": p.decided_at, "rejection_note": p.decision_note, "reviewer_corrections": corrections, "changes": changes}));
    }
    Ok(json!({ "proposals": out, "note": "`pending` = waiting for the user; nothing in a pending proposal is final." }))
}
