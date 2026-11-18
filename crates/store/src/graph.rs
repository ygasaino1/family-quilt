//! In-memory graph snapshots built by the store layer.
//!
//! Family derivation, layout and relationship finding (phases 2-3) work on this
//! snapshot, never on SQL, so data a context may not see is simply absent
//! (section 16). People are addressed by dense integer indexes.

use crate::access::AccessContext;
use crate::dates::FuzzyDate;
use crate::error::{Result, StoreError};
use crate::model::*;
use crate::store::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GraphFilter {
    /// Include pending people and links (`?pending=1`). Default: verified data only.
    pub include_pending: bool,
}

#[derive(Debug, Clone)]
pub struct GraphPerson {
    pub id: String,
    pub display_name: String,
    pub sex: Sex,
    pub is_living: bool,
    pub birth: Option<FuzzyDate>,
    pub death: Option<FuzzyDate>,
    pub status: Status,
}

#[derive(Debug, Clone)]
pub struct GraphLink {
    pub id: String,
    pub rel_type: RelType,
    /// Index into `Graph::people`. parent_child: the parent.
    pub a: u32,
    /// Index into `Graph::people`. parent_child: the child.
    pub b: u32,
    pub kind: Option<LinkKind>,
    pub subtype: Option<PartnershipState>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    /// Pending itself, or one of its two people is pending.
    pub pending: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub people: Vec<GraphPerson>,
    pub links: Vec<GraphLink>,
    /// True when the context hid some data (always false in v1).
    pub incomplete: bool,
    index: HashMap<String, u32>,
}

impl Graph {
    /// Build a snapshot from parts (tests, importers). Links must reference valid people indexes.
    pub fn from_parts(people: Vec<GraphPerson>, links: Vec<GraphLink>) -> Graph {
        let index = people.iter().enumerate().map(|(i, p)| (p.id.clone(), i as u32)).collect();
        Graph { people, links, incomplete: false, index }
    }
    pub fn index_of(&self, id: &str) -> Option<u32> {
        self.index.get(id).copied()
    }
    pub fn person(&self, idx: u32) -> &GraphPerson {
        &self.people[idx as usize]
    }
}

impl Store {
    /// Load everything visible to `ctx` into memory. People are ordered by id (= creation order).
    pub fn load_graph(&self, ctx: &AccessContext, filter: GraphFilter) -> Result<Graph> {
        let mut g = Graph { incomplete: ctx.is_incomplete_view(), ..Default::default() };
        let pending_ok = filter.include_pending as i64;

        let sql = format!(
            "SELECT p.id, COALESCE((SELECT full_name FROM person_name WHERE person_id = p.id AND deleted_at IS NULL ORDER BY is_primary DESC, sort_order, id LIMIT 1), '(unnamed)'),
                    p.sex, p.is_living, p.birth_date, p.birth_qualifier, p.death_date, p.death_qualifier, p.status
             FROM person p
             WHERE p.deleted_at IS NULL AND p.merged_into IS NULL AND (p.status = 'verified' OR ?1 = 1) AND {}
             ORDER BY p.id",
            ctx.row_filter("p")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([pending_ok], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
            ))
        })?;
        for row in rows {
            let (id, display_name, sex, living, bd, bq, dd, dq, status) = row?;
            let corrupt = |w: &str, v: &str| StoreError::Corrupt(format!("stored {w} '{v}'"));
            g.index.insert(id.clone(), g.people.len() as u32);
            g.people.push(GraphPerson {
                id,
                display_name,
                sex: Sex::parse(&sex).map_err(|_| corrupt("sex", &sex))?,
                is_living: living == 1,
                birth: decode_fuzzy(bd, bq)?,
                death: decode_fuzzy(dd, dq)?,
                status: Status::parse(&status).map_err(|_| corrupt("status", &status))?,
            });
        }

        let sql = format!(
            "SELECT r.id, r.type, r.person_a, r.person_b, r.kind, r.subtype, r.start_date, r.end_date, r.status
             FROM relationship r
             WHERE r.deleted_at IS NULL AND (r.status = 'verified' OR ?1 = 1) AND {}
             ORDER BY r.id",
            ctx.row_filter("r")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([pending_ok], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
            ))
        })?;
        for row in rows {
            let (id, t, a, b, kind, subtype, start_date, end_date, status) = row?;
            // A link is only usable if both people are in this snapshot.
            let (Some(ia), Some(ib)) = (g.index_of(&a), g.index_of(&b)) else { continue };
            let corrupt = |w: &str, v: &str| StoreError::Corrupt(format!("stored {w} '{v}'"));
            let pending = status == "pending" || g.person(ia).status == Status::Pending || g.person(ib).status == Status::Pending;
            g.links.push(GraphLink {
                id,
                rel_type: RelType::parse(&t).map_err(|_| corrupt("type", &t))?,
                a: ia,
                b: ib,
                kind: kind.map(|k| LinkKind::parse(&k).map_err(|_| corrupt("kind", &k))).transpose()?,
                subtype: subtype.map(|s| PartnershipState::parse(&s).map_err(|_| corrupt("subtype", &s))).transpose()?,
                start_date,
                end_date,
                pending,
            });
        }
        Ok(g)
    }
}
