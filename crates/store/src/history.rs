//! Read access to the revision log (the History list of section 19).

use crate::access::AccessContext;
use crate::error::Result;
use crate::store::Store;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Revision {
    pub id: String,
    pub target_type: String,
    pub target_id: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub by: String,
    pub at: String,
    pub proposal_id: Option<String>,
}

impl Store {
    /// Revisions of one record, oldest first.
    pub fn revisions_of(&self, ctx: &AccessContext, target_type: &str, target_id: &str) -> Result<Vec<Revision>> {
        let sql = format!("SELECT id, target_type, target_id, before, after, by, at, proposal_id FROM revision r WHERE target_type = ?1 AND target_id = ?2 AND {} ORDER BY id", ctx.row_filter("r"));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([target_type, target_id], |r| {
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
        })?;
        rows.map(|row| {
            let (id, target_type, target_id, before, after, by, at, proposal_id) = row?;
            Ok(Revision { id, target_type, target_id, before: before.map(|s| serde_json::from_str(&s)).transpose()?, after: after.map(|s| serde_json::from_str(&s)).transpose()?, by, at, proposal_id })
        })
        .collect()
    }
}
