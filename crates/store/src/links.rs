//! Relationships: parent-child links (with `kind`) and partnerships.

use crate::access::AccessContext;
use crate::dates::PartialDate;
use crate::error::{Result, StoreError};
use crate::ids::{new_id, now_iso};
use crate::model::*;
use crate::store::*;
use rusqlite::{params, Connection, OptionalExtension, Row};

const REL_COLS: &str =
    "r.id, r.type, r.person_a, r.person_b, r.kind, r.subtype, r.start_date, r.end_date, pl.id, pl.name, r.note, r.status, r.proposal_id, r.created_by, r.agent_note, r.created_at, r.updated_at";
const REL_FROM: &str = "FROM relationship r LEFT JOIN place pl ON pl.id = r.place_id";

type RawRel = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    String,
);

fn raw_rel(r: &Row<'_>) -> rusqlite::Result<RawRel> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
        r.get(12)?,
        r.get(13)?,
        r.get(14)?,
        r.get(15)?,
        r.get(16)?,
    ))
}

fn build_rel(raw: RawRel) -> Result<Relationship> {
    let (id, t, a, b, kind, subtype, start, end, pl_id, pl_name, note, status, proposal_id, created_by, agent_note, created_at, updated_at) = raw;
    let corrupt = |what: &str, v: &str| StoreError::Corrupt(format!("stored {what} '{v}'"));
    Ok(Relationship {
        id,
        rel_type: RelType::parse(&t).map_err(|_| corrupt("type", &t))?,
        person_a: a,
        person_b: b,
        kind: kind.map(|k| LinkKind::parse(&k).map_err(|_| corrupt("kind", &k))).transpose()?,
        subtype: subtype.map(|s| PartnershipState::parse(&s).map_err(|_| corrupt("subtype", &s))).transpose()?,
        start_date: start,
        end_date: end,
        place: pl_id.zip(pl_name).map(|(id, name)| PlaceRef { id, name }),
        note,
        status: Status::parse(&status).map_err(|_| corrupt("status", &status))?,
        proposal_id,
        created_by,
        agent_note,
        created_at,
        updated_at,
    })
}

pub(crate) fn load_relationship(conn: &Connection, ctx: &AccessContext, id: &str) -> Result<Relationship> {
    let sql = format!("SELECT {REL_COLS} {REL_FROM} WHERE r.id = ?1 AND r.deleted_at IS NULL AND {}", ctx.row_filter("r"));
    let raw = conn.query_row(&sql, [id], raw_rel).optional()?;
    build_rel(raw.ok_or_else(|| StoreError::NotFound { kind: "relationship", id: id.to_string() })?)
}

/// Would adding `parent -> child` make `child` an ancestor of `parent` (or the same person)?
fn creates_cycle(conn: &Connection, parent: &str, child: &str) -> Result<bool> {
    let hit: Option<i64> = conn
        .query_row(
            "WITH RECURSIVE anc(id) AS (
               SELECT ?1
               UNION
               SELECT r.person_a FROM anc CROSS JOIN relationship r INDEXED BY idx_rel_b
               WHERE r.person_b = anc.id AND r.type = 'parent_child' AND r.deleted_at IS NULL
             ) SELECT 1 FROM anc WHERE id = ?2 LIMIT 1",
            params![parent, child],
            |r| r.get(0),
        )
        .optional()?;
    Ok(hit.is_some())
}

impl Store {
    /// Add a link. Rules enforced here, in one transaction: both people exist and are not deleted,
    /// no self-links, no duplicate (same type and pair), and no parent-child cycle.
    pub fn add_relationship(&mut self, ctx: &AccessContext, new: NewRelationship, origin: &Origin) -> Result<Relationship> {
        if origin.status == Status::Pending && origin.proposal_id.is_none() {
            return Err(StoreError::invalid("status", "pending data must belong to a proposal"));
        }
        if new.person_a == new.person_b {
            return Err(StoreError::invalid("person_b", "a person cannot be linked to themselves"));
        }
        let (kind, subtype) = match new.rel_type {
            RelType::ParentChild => {
                if new.subtype.is_some() {
                    return Err(StoreError::invalid("subtype", "only partnerships have a subtype"));
                }
                (Some(new.kind.unwrap_or_default()), None)
            }
            RelType::Partnership => {
                if new.kind.is_some() {
                    return Err(StoreError::invalid("kind", "only parent-child links have a kind"));
                }
                (None, Some(new.subtype.unwrap_or_default()))
            }
        };
        // Partnerships are stored once, smaller id first.
        let (a, b) = if new.rel_type == RelType::Partnership && new.person_a > new.person_b { (new.person_b.clone(), new.person_a.clone()) } else { (new.person_a.clone(), new.person_b.clone()) };
        let note = clean(new.note.as_deref());
        check_len("note", note.as_deref().unwrap_or(""), MAX_NOTE)?;
        let by = origin.created_by.as_str();

        let tx = self.begin()?;
        require_live_person(&tx, &a)?;
        require_live_person(&tx, &b)?;
        if let Some(existing) = tx
            .query_row("SELECT id FROM relationship WHERE type = ?1 AND person_a = ?2 AND person_b = ?3 AND deleted_at IS NULL", params![new.rel_type.as_str(), a, b], |r| r.get::<_, String>(0))
            .optional()?
        {
            return Err(StoreError::Duplicate { what: "relationship", id: existing });
        }
        if new.rel_type == RelType::ParentChild && creates_cycle(&tx, &a, &b)? {
            return Err(StoreError::Cycle);
        }
        let place = clean(new.place.as_deref()).map(|p| resolve_place(&tx, &p, by)).transpose()?;
        let id = new_id();
        let t = now_iso();
        tx.execute(
            "INSERT INTO relationship(id, type, person_a, person_b, kind, subtype, start_date, end_date, place_id, note, status, proposal_id, created_by, agent_note, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15)",
            params![
                id,
                new.rel_type.as_str(),
                a,
                b,
                kind.map(|k| k.as_str()),
                subtype.map(|s| s.as_str()),
                new.start_date.map(|d| d.to_string()),
                new.end_date.map(|d| d.to_string()),
                place,
                note,
                origin.status.as_str(),
                origin.proposal_id,
                origin.created_by,
                clean(origin.agent_note.as_deref()),
                t
            ],
        )?;
        let after = snapshot(&tx, "relationship", &id)?;
        write_revision(&tx, "relationship", &id, None, after.as_ref(), by, origin.proposal_id.as_deref())?;
        let out = load_relationship(&tx, ctx, &id)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn get_relationship(&self, ctx: &AccessContext, id: &str) -> Result<Relationship> {
        load_relationship(&self.conn, ctx, id)
    }

    /// Every live link the person is in (as parent, child or partner), oldest first.
    pub fn relationships_of(&self, ctx: &AccessContext, person_id: &str) -> Result<Vec<Relationship>> {
        let sql = format!("SELECT {REL_COLS} {REL_FROM} WHERE (r.person_a = ?1 OR r.person_b = ?1) AND r.deleted_at IS NULL AND {} ORDER BY r.id", ctx.row_filter("r"));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([person_id], raw_rel)?;
        rows.map(|r| build_rel(r?)).collect()
    }

    /// Every live link visible to `ctx`, ordered by id. For exports and tools.
    pub fn all_relationships(&self, ctx: &AccessContext, include_pending: bool) -> Result<Vec<Relationship>> {
        let sql = format!("SELECT {REL_COLS} {REL_FROM} WHERE r.deleted_at IS NULL AND (r.status = 'verified' OR ?1 = 1) AND {} ORDER BY r.id", ctx.row_filter("r"));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([include_pending as i64], raw_rel)?;
        rows.map(|r| build_rel(r?)).collect()
    }

    /// Change kind / state / dates / place / note. Type and the two people cannot change (remove and add instead).
    pub fn update_relationship(&mut self, ctx: &AccessContext, id: &str, patch: RelationshipPatch, by: &str) -> Result<Relationship> {
        let tx = self.begin()?;
        let cur = load_relationship(&tx, ctx, id)?;
        if cur.rel_type == RelType::ParentChild && patch.subtype.is_some() {
            return Err(StoreError::invalid("subtype", "only partnerships have a subtype"));
        }
        if cur.rel_type == RelType::Partnership && patch.kind.is_some() {
            return Err(StoreError::invalid("kind", "only parent-child links have a kind"));
        }
        let before = snapshot(&tx, "relationship", id)?;
        let date = |f: &Field<PartialDate>, cur: &Option<String>| -> Option<String> {
            match f {
                Field::Unchanged => cur.clone(),
                Field::Clear => None,
                Field::Set(d) => Some(d.to_string()),
            }
        };
        let place = match &patch.place {
            Field::Unchanged => cur.place.as_ref().map(|p| p.id.clone()),
            Field::Clear => None,
            Field::Set(name) => Some(resolve_place(&tx, name, by)?),
        };
        let note = match &patch.note {
            Field::Unchanged => cur.note.clone(),
            Field::Clear => None,
            Field::Set(n) => clean(Some(n)),
        };
        check_len("note", note.as_deref().unwrap_or(""), MAX_NOTE)?;
        tx.execute(
            "UPDATE relationship SET kind=?2, subtype=?3, start_date=?4, end_date=?5, place_id=?6, note=?7, updated_at=?8 WHERE id=?1",
            params![
                id,
                patch.kind.or(cur.kind).map(|k| k.as_str()),
                patch.subtype.or(cur.subtype).map(|s| s.as_str()),
                date(&patch.start_date, &cur.start_date),
                date(&patch.end_date, &cur.end_date),
                place,
                note,
                now_iso()
            ],
        )?;
        let after = snapshot(&tx, "relationship", id)?;
        if let (Some(b), Some(a)) = (&before, &after) {
            if same_content(b, a) {
                return Ok(cur);
            }
        }
        write_revision(&tx, "relationship", id, before.as_ref(), after.as_ref(), by, None)?;
        let out = load_relationship(&tx, ctx, id)?;
        tx.commit()?;
        Ok(out)
    }

    /// Soft-delete one link.
    pub fn remove_relationship(&mut self, ctx: &AccessContext, id: &str, by: &str) -> Result<()> {
        let tx = self.begin()?;
        load_relationship(&tx, ctx, id)?;
        let before = snapshot(&tx, "relationship", id)?;
        tx.execute("UPDATE relationship SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1", params![id, now_iso()])?;
        let after = snapshot(&tx, "relationship", id)?;
        write_revision(&tx, "relationship", id, before.as_ref(), after.as_ref(), by, None)?;
        tx.commit()?;
        Ok(())
    }
}
