//! Correcting a pending proposal in place. The reviewer fixes a value (a date, a name, a link kind, the new value of a
//! proposed edit) and then approves; the agent's own wording is kept in `reviewer_edits` (original and current value),
//! so the change can be undone with one click and the agent can later be told what was corrected.
//!
//! New people and links are real pending records, so a correction is written straight to them (and appears in the
//! history, tagged with the proposal). Who a link joins cannot be changed here: reject the item instead.

use crate::fields::{allowed_fields, apply_name, apply_person, apply_relationship, check_to, current_name_field, current_person_field, current_rel_field, norm};
use crate::{FieldError, Result, ReviewError};
use family_store::{AccessContext, RelType, Store, StoreError};
use serde::Deserialize;
use serde_json::{json, Map, Value};

const CTX: AccessContext = AccessContext::Everything;

type Reader = Box<dyn Fn(&str) -> Value>;

#[derive(Debug, Deserialize)]
pub struct EditRequest {
    /// `person`, `person_name`, `relationship` (new records of this proposal) or `change` (a proposed edit).
    pub kind: String,
    pub target_id: String,
    /// `{field: new value}`; `null` clears a field that may be cleared.
    pub fields: Map<String, Value>,
}

/// Apply the corrections; returns the proposal's list of corrections as it now stands.
pub fn edit(store: &mut Store, proposal_id: &str, req: &EditRequest, by: &str) -> Result<Vec<Value>> {
    let proposal = store.get_proposal(&CTX, proposal_id)?;
    if proposal.status != "pending" {
        return Err(StoreError::Conflict(format!("this proposal was already {}", proposal.status)).into());
    }
    if req.fields.is_empty() {
        return Err(ReviewError::Invalid(vec![FieldError::new("$.fields", "empty", "nothing to change")]));
    }
    let recs = store.proposal_records(&CTX, proposal_id)?;
    let not_here = || ReviewError::Invalid(vec![FieldError::new("$.target_id", "not_in_proposal", "that item does not belong to this proposal")]);

    // What each field is now, which fields may be edited, and the link type (some checks depend on it).
    let (allowed, rel_type, current): (&[&str], Option<RelType>, Reader) = match req.kind.as_str() {
        "person" => {
            if !recs.people.contains(&req.target_id) {
                return Err(not_here());
            }
            let p = store.get_person(&CTX, &req.target_id)?;
            (allowed_fields("person", None), None, Box::new(move |f| current_person_field(&p, f)))
        }
        "person_name" => {
            let (pid, n) = store.find_name(&CTX, &req.target_id).map_err(|_| not_here())?;
            if !recs.people.contains(&pid) {
                return Err(not_here());
            }
            (allowed_fields("person_name", None), None, Box::new(move |f| current_name_field(&n, f)))
        }
        "relationship" => {
            if !recs.relationships.contains(&req.target_id) {
                return Err(not_here());
            }
            let r = store.get_relationship(&CTX, &req.target_id)?;
            let t = r.rel_type;
            (allowed_fields("relationship", Some(t)), Some(t), Box::new(move |f| current_rel_field(&r, f)))
        }
        "change" => {
            let c = store.changes_of(&CTX, proposal_id)?.into_iter().find(|c| c.id == req.target_id).ok_or_else(not_here)?;
            let rel = if c.target_type == "relationship" { store.get_relationship(&CTX, &c.target_id).ok().map(|r| r.rel_type) } else { None };
            let patch = c.patch.clone();
            (allowed_fields(&c.target_type, rel), rel, Box::new(move |f| norm(&patch[f]["to"])))
        }
        _ => return Err(ReviewError::Invalid(vec![FieldError::new("$.kind", "invalid_value", "kind must be person, person_name, relationship or change")])),
    };

    // Check everything first: nothing is written unless every value is acceptable.
    let mut errs = vec![];
    for (f, v) in &req.fields {
        if !allowed.contains(&f.as_str()) {
            errs.push(FieldError::new(format!("$.fields.{f}"), "not_editable", format!("'{f}' cannot be edited here")));
        } else if let Err(m) = check_to(f, rel_type, v) {
            errs.push(FieldError::new(format!("$.fields.{f}"), "invalid_value", m));
        }
    }
    if !errs.is_empty() {
        return Err(ReviewError::Invalid(errs));
    }

    let mut log = store.reviewer_edits(proposal_id)?;
    let before: Vec<(String, Value)> = req.fields.keys().map(|f| (f.clone(), norm(&current(f)))).collect();
    let marker = store.revision_marker();
    store.atomic(|s| {
        let vals: Vec<(&str, &Value)> = req.fields.iter().map(|(k, v)| (k.as_str(), v)).collect();
        match req.kind.as_str() {
            "person" => apply_person(s, &CTX, &req.target_id, &vals, by)?,
            "person_name" => apply_name(s, &CTX, &req.target_id, &vals, by)?,
            "relationship" => apply_relationship(s, &CTX, &req.target_id, &vals, by)?,
            _ => {
                for (f, v) in &req.fields {
                    s.set_change_to(&req.target_id, f, &norm(v))?;
                }
            }
        }
        s.tag_revisions_since(&marker, proposal_id)?;
        Ok(())
    })?;

    // Keep the first value the agent gave; drop the entry again when the reviewer puts it back.
    for (f, was) in before {
        let now = norm(&req.fields[&f]);
        let pos = log.iter().position(|e| e["kind"] == req.kind && e["target_id"] == req.target_id && e["field"] == f);
        let original = pos.map(|i| log[i]["original"].clone()).unwrap_or(was);
        if let Some(i) = pos {
            log.remove(i);
        }
        if now != original {
            log.push(json!({"kind": req.kind, "target_id": req.target_id, "field": f, "original": original, "current": now, "at": family_store::ids::now_iso()}));
        }
    }
    store.set_reviewer_edits(proposal_id, &log)?;
    Ok(log)
}
