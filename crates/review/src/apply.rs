//! Approve / reject. Approving applies the new records and the accepted changes in **one
//! transaction**; a stale change is never applied silently (section 19).

use crate::fields::{apply_name, apply_person, apply_relationship, norm};
use crate::review::{change_view, dependencies, proposal_warnings, Warning};
use crate::{FieldError, Result, ReviewError};
use family_store::{AccessContext, Store, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

const CTX: AccessContext = AccessContext::Everything;

fn yes() -> bool {
    true
}

/// The human's choice for one change. Changes without a decision are accepted (all non-conflicting fields).
#[derive(Debug, Clone, Deserialize)]
pub struct ChangeDecision {
    pub change_id: String,
    #[serde(default = "yes")]
    pub accept: bool,
    /// Accept only these fields (default: all).
    pub fields: Option<Vec<String>>,
    /// Fields (or `"remove"`) the human chose to apply **even though the record changed** since the agent read it.
    #[serde(default)]
    pub overwrite: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ApproveRequest {
    #[serde(default)]
    pub decisions: Vec<ChangeDecision>,
}

#[derive(Debug, Serialize)]
pub struct ApproveReport {
    pub proposal_id: String,
    pub people_verified: usize,
    pub relationships_verified: usize,
    pub changes_applied: usize,
    pub changes_rejected: usize,
    /// Changes (or fields) left unapplied because the record changed; the proposal is still approved for the rest.
    pub conflicts_skipped: Vec<String>,
    pub warnings: Vec<Warning>,
}

pub fn approve(store: &mut Store, proposal_id: &str, req: &ApproveRequest, by: &str) -> Result<ApproveReport> {
    let proposal = store.get_proposal(&CTX, proposal_id)?;
    if proposal.status != "pending" {
        return Err(StoreError::Conflict(format!("this proposal was already {}", proposal.status)).into());
    }
    // Blocked while it links to, or edits, people another proposal has not added yet.
    let waiting: Vec<_> = dependencies(store, proposal_id)?;
    if !waiting.is_empty() {
        let list: Vec<String> = waiting.iter().map(|d| format!("\"{}\" ({})", d.summary.clone().unwrap_or_default(), d.created_by)).collect();
        return Err(StoreError::Conflict(format!("approve the proposal it depends on first: {}", list.join(", "))).into());
    }
    let warnings = proposal_warnings(store, proposal_id)?;
    let changes = store.changes_of(&CTX, proposal_id)?;
    let known: HashMap<&str, ()> = changes.iter().map(|c| (c.id.as_str(), ())).collect();
    let bad: Vec<FieldError> = req
        .decisions
        .iter()
        .enumerate()
        .filter(|(_, d)| !known.contains_key(d.change_id.as_str()))
        .map(|(i, d)| FieldError::new(format!("decisions[{i}].change_id"), "unknown_id", format!("'{}' is not a change of this proposal", d.change_id)))
        .collect();
    if !bad.is_empty() {
        return Err(ReviewError::Invalid(bad));
    }
    let decisions: HashMap<&str, &ChangeDecision> = req.decisions.iter().map(|d| (d.change_id.as_str(), d)).collect();
    let views: HashMap<String, _> = changes.iter().map(|c| (c.id.clone(), change_view(store, c.clone()))).collect();

    let mut report =
        ApproveReport { proposal_id: proposal_id.to_string(), people_verified: 0, relationships_verified: 0, changes_applied: 0, changes_rejected: 0, conflicts_skipped: vec![], warnings };
    let marker = store.revision_marker();
    let rep = &mut report;
    store.atomic(|s| {
        let recs = s.verify_proposal_records(&CTX, proposal_id, by)?;
        rep.people_verified = recs.people.len();
        rep.relationships_verified = recs.relationships.len();
        for c in &changes {
            let v = &views[&c.id];
            let d = decisions.get(c.id.as_str());
            if d.is_some_and(|d| !d.accept) {
                s.set_change_status(&CTX, &c.id, "rejected")?;
                rep.changes_rejected += 1;
                continue;
            }
            if v.target_missing {
                s.set_change_status(&CTX, &c.id, "conflicting")?;
                rep.conflicts_skipped.push(format!("{}: the record no longer exists", c.id));
                continue;
            }
            let overwrite: &[String] = d.map(|d| d.overwrite.as_slice()).unwrap_or(&[]);
            if c.op == "remove" {
                if v.stale && !overwrite.iter().any(|f| f == "remove") {
                    s.set_change_status(&CTX, &c.id, "conflicting")?;
                    rep.conflicts_skipped.push(format!("{}: the link changed after the agent read it", c.id));
                    continue;
                }
                s.remove_relationship(&CTX, &c.target_id, by)?;
                s.set_change_status(&CTX, &c.id, "applied")?;
                rep.changes_applied += 1;
                continue;
            }
            let wanted = d.and_then(|d| d.fields.as_ref());
            let mut apply: Vec<(&str, &Value)> = vec![];
            let mut skipped = 0;
            for f in &v.fields {
                if wanted.is_some_and(|w| !w.contains(&f.field)) {
                    skipped += 1;
                    continue;
                }
                if f.conflict && !overwrite.contains(&f.field) {
                    rep.conflicts_skipped.push(format!("{}.{}: now {} (the agent saw {})", c.id, f.field, f.current, f.from));
                    skipped += 1;
                    continue;
                }
                apply.push((f.field.as_str(), &f.to));
            }
            if apply.is_empty() {
                s.set_change_status(&CTX, &c.id, if skipped > 0 && v.fields.iter().any(|f| f.conflict) { "conflicting" } else { "rejected" })?;
                continue;
            }
            match c.target_type.as_str() {
                "person" => apply_person(s, &CTX, &c.target_id, &apply, by)?,
                "person_name" => apply_name(s, &CTX, &c.target_id, &apply, by)?,
                _ => apply_relationship(s, &CTX, &c.target_id, &apply, by)?,
            }
            s.set_change_status(&CTX, &c.id, "applied")?;
            rep.changes_applied += 1;
        }
        s.decide_proposal(&CTX, proposal_id, "approved")?;
        s.tag_revisions_since(&marker, proposal_id)?;
        Ok(())
    })?;
    let _ = norm;
    Ok(report)
}

/// Reject: nothing changes; the proposal's pending records are removed.
pub fn reject(store: &mut Store, proposal_id: &str, by: &str) -> Result<RejectReport> {
    let proposal = store.get_proposal(&CTX, proposal_id)?;
    if proposal.status != "pending" {
        return Err(StoreError::Conflict(format!("this proposal was already {}", proposal.status)).into());
    }
    let changes = store.changes_of(&CTX, proposal_id)?;
    let recs = store.proposal_records(&CTX, proposal_id)?;
    // Other proposals' links to the people being removed would dangle: remove those links too and say so.
    let mut orphan_links: Vec<(String, Option<String>)> = vec![];
    let mut orphan_changes: Vec<(String, String)> = vec![];
    for pid in &recs.people {
        for r in store.relationships_of(&CTX, pid)? {
            if r.proposal_id.as_deref() != Some(proposal_id) {
                orphan_links.push((r.id.clone(), r.proposal_id.clone()));
            }
        }
        for c in store.open_changes_for(&CTX, "person", pid)? {
            if c.proposal_id != proposal_id {
                orphan_changes.push((c.id, c.proposal_id));
            }
        }
    }
    let marker = store.revision_marker();
    store.atomic(|s| {
        for (id, _) in &orphan_links {
            s.remove_relationship(&CTX, id, by)?;
        }
        for (id, _) in &orphan_changes {
            s.set_change_status(&CTX, id, "conflicting")?;
        }
        s.discard_proposal_records(&CTX, proposal_id, by)?;
        for c in &changes {
            s.set_change_status(&CTX, &c.id, "rejected")?;
        }
        s.decide_proposal(&CTX, proposal_id, "rejected")?;
        s.tag_revisions_since(&marker, proposal_id)?;
        Ok(())
    })?;
    let mut affected: Vec<String> = orphan_links.into_iter().filter_map(|(_, p)| p).chain(orphan_changes.into_iter().map(|(_, p)| p)).collect();
    affected.sort();
    affected.dedup();
    Ok(RejectReport { affected_proposals: affected })
}

/// Proposals that lost a link or an edit because the one they depended on was rejected.
#[derive(Debug, Serialize)]
pub struct RejectReport {
    pub affected_proposals: Vec<String>,
}
