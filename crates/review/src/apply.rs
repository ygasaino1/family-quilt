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
    /// New people of this proposal to leave out. Their links go with them, and so do this proposal's edits to them.
    #[serde(default)]
    pub exclude_people: Vec<String>,
    /// New links of this proposal to leave out.
    #[serde(default)]
    pub exclude_relationships: Vec<String>,
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
    pub people_removed: usize,
    /// New people and links the human left out of this approval (links of a left-out person included).
    pub people_excluded: usize,
    pub relationships_excluded: usize,
    /// Other proposals that lost a link or an edit because a person they used was left out.
    pub affected_proposals: Vec<String>,
    /// Proposed person removals left alone because they were not ticked explicitly.
    pub removals_not_confirmed: Vec<String>,
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

    let mut report = ApproveReport {
        proposal_id: proposal_id.to_string(),
        people_verified: 0,
        relationships_verified: 0,
        changes_applied: 0,
        changes_rejected: 0,
        conflicts_skipped: vec![],
        people_removed: 0,
        people_excluded: 0,
        relationships_excluded: 0,
        affected_proposals: vec![],
        removals_not_confirmed: vec![],
        warnings,
    };
    // Items left out: a left-out person takes their links along; other proposals' links to them would dangle.
    let recs = store.proposal_records(&CTX, proposal_id)?;
    let mut bad: Vec<FieldError> = vec![];
    for (i, id) in req.exclude_people.iter().enumerate() {
        if !recs.people.contains(id) {
            bad.push(FieldError::new(format!("exclude_people[{i}]"), "unknown_id", format!("'{id}' is not a new person of this proposal")));
        }
    }
    for (i, id) in req.exclude_relationships.iter().enumerate() {
        if !recs.relationships.contains(id) {
            bad.push(FieldError::new(format!("exclude_relationships[{i}]"), "unknown_id", format!("'{id}' is not a new link of this proposal")));
        }
    }
    if !bad.is_empty() {
        return Err(ReviewError::Invalid(bad));
    }
    let mut out_people: Vec<String> = req.exclude_people.clone();
    out_people.sort();
    out_people.dedup();
    let mut out_rels: Vec<String> = req.exclude_relationships.clone();
    let mut orphan_links: Vec<(String, Option<String>)> = vec![];
    let mut orphan_changes: Vec<(String, String)> = vec![];
    for pid in &out_people {
        for r in store.relationships_of(&CTX, pid)? {
            if r.proposal_id.as_deref() == Some(proposal_id) {
                out_rels.push(r.id);
            } else {
                orphan_links.push((r.id.clone(), r.proposal_id.clone()));
            }
        }
        for c in store.open_changes_for(&CTX, "person", pid)? {
            if c.proposal_id != proposal_id {
                orphan_changes.push((c.id, c.proposal_id));
            }
        }
    }
    out_rels.sort();
    out_rels.dedup();
    let keep = family_store::ProposalRecords {
        people: recs.people.iter().filter(|id| !out_people.contains(id)).cloned().collect(),
        relationships: recs.relationships.iter().filter(|id| !out_rels.contains(id)).cloned().collect(),
    };
    let out = family_store::ProposalRecords { people: out_people.clone(), relationships: out_rels.clone() };
    let marker = store.revision_marker();
    let rep = &mut report;
    store.atomic(|s| {
        for (id, _) in &orphan_links {
            s.remove_relationship(&CTX, id, by)?;
        }
        for (id, _) in &orphan_changes {
            s.set_change_status(&CTX, id, "conflicting")?;
        }
        s.verify_records(proposal_id, &keep, by)?;
        s.discard_records(proposal_id, &out, by)?;
        rep.people_verified = keep.people.len();
        rep.relationships_verified = keep.relationships.len();
        rep.people_excluded = out.people.len();
        rep.relationships_excluded = out.relationships.len();
        for c in &changes {
            let v = &views[&c.id];
            let d = decisions.get(c.id.as_str());
            if out.people.contains(&c.target_id) || out.relationships.contains(&c.target_id) {
                // an edit to something that was left out
                s.set_change_status(&CTX, &c.id, "rejected")?;
                rep.changes_rejected += 1;
                continue;
            }
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
            if c.op == "remove" && c.target_type == "person" {
                // Deleting a person is never applied by default: the human must tick it explicitly.
                if !d.is_some_and(|d| d.accept) {
                    s.set_change_status(&CTX, &c.id, "rejected")?;
                    rep.changes_rejected += 1;
                    rep.removals_not_confirmed.push(v.target_label.clone());
                    continue;
                }
                if v.stale && !overwrite.iter().any(|f| f == "remove") {
                    s.set_change_status(&CTX, &c.id, "conflicting")?;
                    rep.conflicts_skipped.push(format!("{}: the person changed after the agent read them", c.id));
                    continue;
                }
                s.soft_delete_person(&CTX, &c.target_id, by)?;
                s.set_change_status(&CTX, &c.id, "applied")?;
                rep.changes_applied += 1;
                rep.people_removed += 1;
                continue;
            }
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
    let mut affected: Vec<String> = orphan_links.into_iter().filter_map(|(_, p)| p).chain(orphan_changes.into_iter().map(|(_, p)| p)).collect();
    affected.sort();
    affected.dedup();
    report.affected_proposals = affected;
    let _ = norm;
    Ok(report)
}

/// Reject: nothing changes; the proposal's pending records are removed.
pub fn reject(store: &mut Store, proposal_id: &str, by: &str) -> Result<RejectReport> {
    reject_with_note(store, proposal_id, by, None)
}

/// Reject, with the reason the agent will read (kept short).
pub fn reject_with_note(store: &mut Store, proposal_id: &str, by: &str, note: Option<&str>) -> Result<RejectReport> {
    reject_impl(store, proposal_id, by, note, false)
}

/// An agent takes back its own pending proposal (nothing of it was ever approved). Refused when another proposal
/// depends on it: that one has to be dealt with first. The user is not asked, and the resend guard does not count it.
pub fn withdraw(store: &mut Store, proposal_id: &str, agent_by: &str, reason: Option<&str>) -> Result<RejectReport> {
    let proposal = store.get_proposal(&CTX, proposal_id)?;
    if proposal.created_by != agent_by {
        let name = |s: &str| s.strip_prefix("agent:").unwrap_or(s).to_string();
        return Err(ReviewError::Invalid(vec![FieldError::new(
            "$.id",
            "not_yours",
            format!("only the agent that sent a proposal can withdraw it; this one was sent by '{}' and you are '{}'", name(&proposal.created_by), name(agent_by)),
        )]));
    }
    if proposal.status != "pending" {
        return Err(StoreError::Conflict(format!("this proposal was already {}; ask the user to undo it from History if it was approved", proposal.status)).into());
    }
    let dependents = dependent_proposals(store, proposal_id)?;
    if !dependents.is_empty() {
        return Err(ReviewError::Invalid(vec![FieldError::new(
            "$.id",
            "has_dependents",
            format!("these pending proposals use records from this one: {}. Withdraw them first, or let the user decide.", dependents.join(", ")),
        )]));
    }
    let note = format!("withdrawn by the agent{}", reason.map(str::trim).filter(|r| !r.is_empty()).map(|r| format!(": {r}")).unwrap_or_default());
    reject_impl(store, proposal_id, agent_by, Some(&note), true)
}

/// Other pending proposals that link to, or edit, records this proposal adds.
pub fn dependent_proposals(store: &Store, proposal_id: &str) -> Result<Vec<String>> {
    let recs = store.proposal_records(&CTX, proposal_id)?;
    let mut out: Vec<String> = vec![];
    for pid in &recs.people {
        for r in store.relationships_of(&CTX, pid)? {
            if let Some(p) = r.proposal_id.filter(|p| p != proposal_id) {
                out.push(p);
            }
        }
        for c in store.open_changes_for(&CTX, "person", pid)? {
            if c.proposal_id != proposal_id {
                out.push(c.proposal_id);
            }
        }
    }
    for rid in &recs.relationships {
        for c in store.open_changes_for(&CTX, "relationship", rid)? {
            if c.proposal_id != proposal_id {
                out.push(c.proposal_id);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn reject_impl(store: &mut Store, proposal_id: &str, by: &str, note: Option<&str>, withdrawn: bool) -> Result<RejectReport> {
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
        if withdrawn {
            s.set_withdrawn(proposal_id)?;
        }
        if let Some(n) = note.map(str::trim).filter(|n| !n.is_empty()) {
            s.set_decision_note(proposal_id, &n.chars().take(500).collect::<String>())?;
        }
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
