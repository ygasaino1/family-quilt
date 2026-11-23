//! Proposals, changes and undo (section 19). The review *logic* (validation, warnings, applying
//! accepted changes) lives in `family-review`; everything that touches SQL stays here.

use crate::access::AccessContext;
use crate::error::{Result, StoreError};
use crate::history::Revision;
use crate::ids::{new_id, now_iso};
use crate::store::*;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct ProposalRow {
    pub id: String,
    pub summary: Option<String>,
    pub source_text: Option<String>,
    pub created_by: String,
    pub status: String,
    pub created_at: String,
    pub decided_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeRow {
    pub id: String,
    pub proposal_id: String,
    pub target_type: String,
    pub target_id: String,
    pub op: String,
    /// `{field: {from, to}}`; `null` for `remove`.
    pub patch: Value,
    pub base_updated_at: Option<String>,
    pub agent_note: Option<String>,
    pub status: String,
}

/// Records that belong to a proposal and are still pending.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProposalRecords {
    pub people: Vec<String>,
    pub relationships: Vec<String>,
}

fn proposal_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProposalRow> {
    Ok(ProposalRow { id: r.get(0)?, summary: r.get(1)?, source_text: r.get(2)?, created_by: r.get(3)?, status: r.get(4)?, created_at: r.get(5)?, decided_at: r.get(6)? })
}

const PROPOSAL_COLS: &str = "id, summary, source_text, created_by, status, created_at, decided_at";

type RawChange = (String, String, String, String, String, Option<String>, Option<String>, Option<String>, String);

fn change_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawChange> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))
}

fn build_change(t: RawChange) -> Result<ChangeRow> {
    let (id, proposal_id, target_type, target_id, op, patch, base_updated_at, agent_note, status) = t;
    Ok(ChangeRow { id, proposal_id, target_type, target_id, op, patch: patch.map(|s| serde_json::from_str(&s)).transpose()?.unwrap_or(Value::Null), base_updated_at, agent_note, status })
}

const CHANGE_COLS: &str = "id, proposal_id, target_type, target_id, op, patch, base_updated_at, agent_note, status";

fn ident_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Store {
    pub fn get_proposal(&self, _ctx: &AccessContext, id: &str) -> Result<ProposalRow> {
        self.conn
            .query_row(&format!("SELECT {PROPOSAL_COLS} FROM proposal WHERE id = ?1"), [id], proposal_from)
            .optional()?
            .ok_or_else(|| StoreError::NotFound { kind: "proposal", id: id.to_string() })
    }

    /// Newest first. `status` and `created_by` are optional filters.
    pub fn list_proposals(&self, _ctx: &AccessContext, status: Option<&str>, created_by: Option<&str>, limit: usize) -> Result<Vec<ProposalRow>> {
        let limit = if limit == 0 { 100 } else { limit.min(1000) } as i64;
        let mut stmt = self.conn.prepare(&format!("SELECT {PROPOSAL_COLS} FROM proposal WHERE (?1 IS NULL OR status = ?1) AND (?2 IS NULL OR created_by = ?2) ORDER BY id DESC LIMIT ?3"))?;
        let rows = stmt.query_map(params![status, created_by, limit], proposal_from)?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// A short token that changes whenever anything a quilt shows may have changed (any revision, any proposal or decision).
    pub fn data_key(&self, _ctx: &AccessContext) -> Result<String> {
        let (rev, prop, pending, decided): (Option<String>, Option<String>, i64, i64) = self.conn.query_row(
            "SELECT (SELECT max(id) FROM revision), (SELECT max(id) FROM proposal), (SELECT count(*) FROM proposal WHERE status = 'pending'), (SELECT count(*) FROM proposal WHERE decided_at IS NOT NULL)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        Ok(format!("{}.{}.{pending}.{decided}", rev.unwrap_or_default().chars().rev().take(12).collect::<String>(), prop.unwrap_or_default().chars().rev().take(12).collect::<String>()))
    }

    pub fn count_proposals(&self, _ctx: &AccessContext, status: &str) -> Result<usize> {
        Ok(self.conn.query_row("SELECT count(*) FROM proposal WHERE status = ?1", [status], |r| r.get::<_, i64>(0))? as usize)
    }

    /// Mark a pending proposal approved or rejected. A decided proposal cannot be decided again.
    pub fn decide_proposal(&mut self, _ctx: &AccessContext, id: &str, status: &str) -> Result<()> {
        if status != "approved" && status != "rejected" {
            return Err(StoreError::invalid("status", "must be approved or rejected"));
        }
        let n = self.conn.execute("UPDATE proposal SET status = ?2, decided_at = ?3 WHERE id = ?1 AND status = 'pending'", params![id, status, now_iso()])?;
        if n == 0 {
            // Either missing or already decided.
            let cur = self.get_proposal(_ctx, id)?;
            return Err(StoreError::Conflict(format!("this proposal was already {}", cur.status)));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_change(
        &mut self,
        _ctx: &AccessContext,
        proposal_id: &str,
        target_type: &str,
        target_id: &str,
        op: &str,
        patch: &Value,
        base_updated_at: Option<&str>,
        agent_note: Option<&str>,
    ) -> Result<String> {
        let id = new_id();
        let patch = if patch.is_null() { None } else { Some(serde_json::to_string(patch)?) };
        self.conn.execute(
            "INSERT INTO change(id, proposal_id, target_type, target_id, op, patch, base_updated_at, agent_note) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, proposal_id, target_type, target_id, op, patch, base_updated_at, agent_note],
        )?;
        Ok(id)
    }

    pub fn changes_of(&self, _ctx: &AccessContext, proposal_id: &str) -> Result<Vec<ChangeRow>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {CHANGE_COLS} FROM change WHERE proposal_id = ?1 ORDER BY id"))?;
        let rows = stmt.query_map([proposal_id], change_from)?.collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(build_change).collect()
    }

    /// Open changes (proposal still pending) that point at one record: the sidebar's "suggested changes".
    pub fn open_changes_for(&self, _ctx: &AccessContext, target_type: &str, target_id: &str) -> Result<Vec<ChangeRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.proposal_id, c.target_type, c.target_id, c.op, c.patch, c.base_updated_at, c.agent_note, c.status
             FROM change c JOIN proposal p ON p.id = c.proposal_id
             WHERE c.target_type = ?1 AND c.target_id = ?2 AND p.status = 'pending' ORDER BY c.id",
        )?;
        let rows = stmt.query_map([target_type, target_id], change_from)?.collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(build_change).collect()
    }

    pub fn set_change_status(&mut self, _ctx: &AccessContext, id: &str, status: &str) -> Result<()> {
        if !["pending", "accepted", "rejected", "applied", "conflicting"].contains(&status) {
            return Err(StoreError::invalid("status", "unknown change status"));
        }
        self.conn.execute("UPDATE change SET status = ?2 WHERE id = ?1", params![id, status])?;
        Ok(())
    }

    /// Pending people and links created by this proposal.
    pub fn proposal_records(&self, _ctx: &AccessContext, proposal_id: &str) -> Result<ProposalRecords> {
        let ids = |sql: &str| -> Result<Vec<String>> {
            let mut s = self.conn.prepare(sql)?;
            let v = s.query_map([proposal_id], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            Ok(v)
        };
        Ok(ProposalRecords {
            people: ids("SELECT id FROM person WHERE proposal_id = ?1 AND status = 'pending' AND deleted_at IS NULL ORDER BY id")?,
            relationships: ids("SELECT id FROM relationship WHERE proposal_id = ?1 AND status = 'pending' AND deleted_at IS NULL ORDER BY id")?,
        })
    }

    /// Approve: this proposal's pending people and links become verified (one revision each).
    pub fn verify_proposal_records(&mut self, ctx: &AccessContext, proposal_id: &str, by: &str) -> Result<ProposalRecords> {
        let recs = self.proposal_records(ctx, proposal_id)?;
        self.verify_records(proposal_id, &recs, by)?;
        Ok(recs)
    }

    /// Verify exactly these records of the proposal (the rest of an approval that left some items out).
    pub fn verify_records(&mut self, proposal_id: &str, recs: &ProposalRecords, by: &str) -> Result<()> {
        for (table, ids) in [("person", &recs.people), ("relationship", &recs.relationships)] {
            for id in ids {
                let before = snapshot(&self.conn, table, id)?;
                self.conn.execute(&format!("UPDATE {table} SET status = 'verified', updated_at = ?2 WHERE id = ?1"), params![id, now_iso()])?;
                let after = snapshot(&self.conn, table, id)?;
                write_revision(&self.conn, table, id, before.as_ref(), after.as_ref(), by, Some(proposal_id))?;
            }
        }
        Ok(())
    }

    /// Reject: this proposal's pending people and links are soft-deleted (they were never visible as verified data).
    pub fn discard_proposal_records(&mut self, ctx: &AccessContext, proposal_id: &str, by: &str) -> Result<ProposalRecords> {
        let recs = self.proposal_records(ctx, proposal_id)?;
        self.discard_records(proposal_id, &recs, by)?;
        Ok(recs)
    }

    /// Soft-delete exactly these pending records (links first, then people).
    pub fn discard_records(&mut self, proposal_id: &str, recs: &ProposalRecords, by: &str) -> Result<()> {
        let t = now_iso();
        for (table, ids) in [("relationship", &recs.relationships), ("person", &recs.people)] {
            for id in ids {
                let before = snapshot(&self.conn, table, id)?;
                self.conn.execute(&format!("UPDATE {table} SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1"), params![id, t])?;
                let after = snapshot(&self.conn, table, id)?;
                write_revision(&self.conn, table, id, before.as_ref(), after.as_ref(), by, Some(proposal_id))?;
            }
        }
        Ok(())
    }

    /// A marker for [`Store::tag_revisions_since`]: ids are strictly increasing, so a fresh id sorts before every revision written after it.
    pub fn revision_marker(&self) -> String {
        new_id()
    }

    /// Stamp the revisions written since `marker` with the proposal that caused them.
    pub fn tag_revisions_since(&mut self, marker: &str, proposal_id: &str) -> Result<usize> {
        Ok(self.conn.execute("UPDATE revision SET proposal_id = ?2 WHERE id > ?1 AND proposal_id IS NULL", params![marker, proposal_id])?)
    }

    /// Newest first. `target` limits the list to one record (the sidebar's History).
    pub fn recent_revisions(&self, ctx: &AccessContext, target: Option<(&str, &str)>, limit: usize) -> Result<Vec<Revision>> {
        let limit = if limit == 0 { 50 } else { limit.min(500) } as i64;
        let (tt, ti) = target.map(|(a, b)| (Some(a), Some(b))).unwrap_or((None, None));
        let sql = format!(
            "SELECT id, target_type, target_id, before, after, by, at, proposal_id FROM revision r
             WHERE target_type <> 'place' AND (?1 IS NULL OR (target_type = ?1 AND target_id = ?2)) AND {} ORDER BY id DESC LIMIT ?3",
            ctx.row_filter("r")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![tt, ti, limit], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(id, target_type, target_id, before, after, by, at, proposal_id)| {
                Ok(Revision {
                    id,
                    target_type,
                    target_id,
                    before: before.map(|s| serde_json::from_str(&s)).transpose()?,
                    after: after.map(|s| serde_json::from_str(&s)).transpose()?,
                    by,
                    at,
                    proposal_id,
                })
            })
            .collect()
    }

    /// Undo one revision by writing the old values back as a **new** revision (history is never deleted).
    /// Refused if the record changed again since (undo the newer change first).
    pub fn undo_revision(&mut self, ctx: &AccessContext, revision_id: &str, by: &str) -> Result<()> {
        let rev = self
            .conn
            .query_row("SELECT target_type, target_id, before, after FROM revision WHERE id = ?1", [revision_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?))
            })
            .optional()?
            .ok_or_else(|| StoreError::NotFound { kind: "revision", id: revision_id.to_string() })?;
        let (table, id, before, after) = rev;
        if !["person", "person_name", "relationship"].contains(&table.as_str()) {
            return Err(StoreError::invalid("target_type", "this kind of change cannot be undone"));
        }
        let _ = ctx;
        let before: Option<Value> = before.map(|s| serde_json::from_str(&s)).transpose()?;
        let after: Value = after.map(|s| serde_json::from_str(&s)).transpose()?.ok_or_else(|| StoreError::Corrupt("revision without an after state".into()))?;
        self.atomic(|s| {
            let current = snapshot(&s.conn, &table, &id)?.ok_or_else(|| StoreError::NotFound { kind: "record", id: id.clone() })?;
            if !same_content(&current, &after) {
                return Err(StoreError::Conflict("this record was changed again after that edit; undo the newer change first".into()));
            }
            let t = now_iso();
            match &before {
                // The edit created the record: undoing it removes the record again (soft delete).
                None => {
                    s.conn.execute(&format!("UPDATE {table} SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1"), params![id, t])?;
                }
                Some(b) => {
                    let obj = b.as_object().ok_or_else(|| StoreError::Corrupt("revision snapshot".into()))?;
                    let cols: Vec<String> = {
                        let mut st = s.conn.prepare(&format!("PRAGMA table_info({table})"))?;
                        let v = st.query_map([], |r| r.get::<_, String>(1))?.collect::<std::result::Result<Vec<_>, _>>()?;
                        v
                    };
                    // One statement for all columns: table CHECKs (date and qualifier together) hold only for the whole row.
                    let mut sets: Vec<String> = vec![];
                    let mut vals: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(id.clone())];
                    for (k, v) in obj {
                        if k == "id" || k == "updated_at" || !cols.contains(k) || !ident_ok(k) {
                            continue;
                        }
                        vals.push(match v {
                            Value::Null => rusqlite::types::Value::Null,
                            Value::Bool(x) => rusqlite::types::Value::Integer(*x as i64),
                            Value::Number(n) => n.as_i64().map(rusqlite::types::Value::Integer).unwrap_or_else(|| rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0))),
                            Value::String(x) => rusqlite::types::Value::Text(x.clone()),
                            _ => continue,
                        });
                        sets.push(format!("{k} = ?{}", vals.len()));
                    }
                    if !sets.is_empty() {
                        s.conn.execute(&format!("UPDATE {table} SET {} WHERE id = ?1", sets.join(", ")), rusqlite::params_from_iter(vals.iter()))?;
                    }
                    s.conn.execute(&format!("UPDATE {table} SET updated_at = ?2 WHERE id = ?1"), params![id, t])?;
                }
            }
            let now = snapshot(&s.conn, &table, &id)?;
            write_revision(&s.conn, &table, &id, Some(&current), now.as_ref(), by, None)?;
            Ok(())
        })
    }
}
