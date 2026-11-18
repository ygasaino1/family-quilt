//! The `Store`: the only place that talks SQL (section 16, "seams").
//!
//! Every public function takes an [`AccessContext`]. Reads and writes happen in
//! transactions; every change to a person, name, link or place also writes a
//! `revision` row so history is complete (section 19).

use crate::access::AccessContext;
use crate::crypto::{sqlcipher_raw_key, DataKey};
use crate::dates::{FuzzyDate, PartialDate, Qualifier};
use crate::error::{Result, StoreError};
use crate::ids::{new_id, now_iso};
use crate::names::search_key;
use crate::schema;
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OptionalExtension, Savepoint, Transaction, TransactionBehavior};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

/// One transaction (top level) or savepoint (inside `atomic`).
pub(crate) enum Tx<'a> {
    Top(Transaction<'a>),
    Nested(Savepoint<'a>),
}

impl std::ops::Deref for Tx<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            Tx::Top(t) => t,
            Tx::Nested(s) => s,
        }
    }
}

impl Tx<'_> {
    pub(crate) fn commit(self) -> Result<()> {
        match self {
            Tx::Top(t) => Ok(t.commit()?),
            Tx::Nested(s) => Ok(s.commit()?),
        }
    }
}

pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) path: Option<PathBuf>,
    pub(crate) protected: bool,
    in_batch: bool,
}

/// Open a connection, apply the key (if any), set the pragmas. Does not migrate.
pub(crate) fn open_connection(path: &Path, key: Option<&DataKey>) -> Result<Connection> {
    let conn = Connection::open(path)?;
    if let Some(k) = key {
        apply_key(&conn, k)?;
    }
    conn.busy_timeout(Duration::from_secs(5))?;
    // WAL + FULL sync: a crash or power cut never loses a committed edit or corrupts the file.
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;")?;
    Ok(conn)
}

/// `PRAGMA key` with a raw 256-bit key. Verifies that this build really has SQLCipher
/// (plain SQLite would silently ignore the pragma and write an **unencrypted** file).
pub(crate) fn apply_key(conn: &Connection, key: &DataKey) -> Result<()> {
    let raw = sqlcipher_raw_key(key);
    let stmt = Zeroizing::new(format!("PRAGMA key = \"{}\";", *raw));
    conn.execute_batch(&stmt)?;
    let version: Option<String> = conn.query_row("PRAGMA cipher_version", [], |r| r.get(0)).optional()?;
    if version.is_none_or(|v| v.is_empty()) {
        return Err(StoreError::EncryptionUnavailable);
    }
    // First real read: fails if the key does not match the file.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).map_err(|_| StoreError::KeyFile("the database cannot be decrypted with the key from the key file".into()))?;
    Ok(())
}

impl Store {
    /// A throw-away in-memory tree (tests, tools). Never protected.
    pub fn open_in_memory() -> Result<Store> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        schema::migrate(&mut conn)?;
        Ok(Store { conn, path: None, protected: false, in_batch: false })
    }

    pub(crate) fn from_connection(mut conn: Connection, path: PathBuf, protected: bool) -> Result<Store> {
        schema::migrate(&mut conn)?;
        Ok(Store { conn, path: Some(path), protected, in_batch: false })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn is_protected(&self) -> bool {
        self.protected
    }
    pub fn schema_version(&self) -> Result<u32> {
        schema::current_version(&self.conn)
    }

    /// A transaction for one store call. Inside [`Store::atomic`] it is a savepoint instead,
    /// so several calls can succeed or fail together.
    pub(crate) fn begin(&mut self) -> Result<Tx<'_>> {
        if self.in_batch {
            Ok(Tx::Nested(self.conn.savepoint()?))
        } else {
            Ok(Tx::Top(self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?))
        }
    }

    /// Run several store calls as one all-or-nothing unit (imports, approved proposals).
    /// If `f` returns an error, nothing it did is kept. Calls cannot be nested.
    pub fn atomic<T>(&mut self, f: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
        if self.in_batch {
            return f(self);
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        self.in_batch = true;
        let result = f(self);
        self.in_batch = false;
        match result {
            Ok(v) => match self.conn.execute_batch("COMMIT") {
                Ok(()) => Ok(v),
                Err(e) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    Err(e.into())
                }
            },
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Minimal proposal row (people and links point to it). Review logic comes with the HTTP phase.
    pub fn insert_proposal(&mut self, _ctx: &AccessContext, summary: &str, source_text: &str, created_by: &str) -> Result<String> {
        let id = new_id();
        self.conn.execute("INSERT INTO proposal(id, summary, source_text, created_by, created_at) VALUES (?1,?2,?3,?4,?5)", params![id, summary, source_text, created_by, now_iso()])?;
        Ok(id)
    }

    // ───────────────────────────── settings ─────────────────────────────

    pub fn meta_get(&self, _ctx: &AccessContext, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn meta_set(&mut self, _ctx: &AccessContext, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }
}

// ───────────────────────────── shared helpers ─────────────────────────────

pub(crate) const MAX_NAME: usize = 300;
pub(crate) const MAX_NOTE: usize = 20_000;

pub(crate) fn check_len(field: &str, s: &str, max: usize) -> Result<()> {
    if s.chars().count() > max {
        return Err(StoreError::invalid(field, format!("longer than {max} characters")));
    }
    Ok(())
}

/// Trim; empty becomes `None`.
pub(crate) fn clean(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub(crate) fn decode_fuzzy(date: Option<String>, qualifier: Option<String>) -> Result<Option<FuzzyDate>> {
    match (date, qualifier) {
        (None, _) => Ok(None),
        (Some(d), q) => {
            let date = PartialDate::parse(&d).map_err(|_| StoreError::Corrupt(format!("date '{d}'")))?;
            let qualifier = Qualifier::parse(q.as_deref().unwrap_or("exact")).map_err(|_| StoreError::Corrupt("date qualifier".into()))?;
            Ok(Some(FuzzyDate { date, qualifier }))
        }
    }
}

/// JSON snapshot of one row (for the revision log). `table` comes from our own code only.
pub(crate) fn snapshot(conn: &Connection, table: &str, id: &str) -> Result<Option<Value>> {
    let table = match table {
        "person" | "person_name" | "relationship" | "place" => table,
        _ => return Err(StoreError::Corrupt(format!("cannot snapshot table {table}"))),
    };
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table} WHERE id = ?1"))?;
    let cols: Vec<String> = stmt.column_names().map(|c| c.to_string()).collect();
    let row = stmt
        .query_row([id], |r| {
            let mut m = Map::new();
            for (i, c) in cols.iter().enumerate() {
                let v = match r.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::from(n),
                    ValueRef::Real(f) => Value::from(f),
                    ValueRef::Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
                    ValueRef::Blob(_) => Value::Null,
                };
                m.insert(c.clone(), v);
            }
            Ok(Value::Object(m))
        })
        .optional()?;
    Ok(row)
}

pub(crate) fn write_revision(conn: &Connection, target_type: &str, target_id: &str, before: Option<&Value>, after: Option<&Value>, by: &str, proposal_id: Option<&str>) -> Result<()> {
    let b = before.map(serde_json::to_string).transpose()?;
    let a = after.map(serde_json::to_string).transpose()?;
    conn.execute(
        "INSERT INTO revision(id, target_type, target_id, before, after, by, at, proposal_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![new_id(), target_type, target_id, b, a, by, now_iso(), proposal_id],
    )?;
    Ok(())
}

/// Same row, ignoring `updated_at` (to detect edits that changed nothing).
pub(crate) fn same_content(a: &Value, b: &Value) -> bool {
    let strip = |v: &Value| {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("updated_at");
        }
        v
    };
    strip(a) == strip(b)
}

/// Find a place by (normalised) name or create it. Free-text places are the v1 model.
pub(crate) fn resolve_place(conn: &Connection, name: &str, by: &str) -> Result<String> {
    let name = name.trim();
    check_len("place", name, MAX_NAME)?;
    let key = search_key(name);
    if key.is_empty() {
        return Err(StoreError::invalid("place", "a place needs a name"));
    }
    if let Some(id) = conn.query_row("SELECT id FROM place WHERE name_key = ?1 AND parent_id IS NULL AND deleted_at IS NULL", [&key], |r| r.get::<_, String>(0)).optional()? {
        return Ok(id);
    }
    let id = new_id();
    let t = now_iso();
    conn.execute("INSERT INTO place(id, name, name_key, created_at, updated_at) VALUES (?1,?2,?3,?4,?4)", params![id, name, key, t])?;
    let after = snapshot(conn, "place", &id)?;
    write_revision(conn, "place", &id, None, after.as_ref(), by, None)?;
    Ok(id)
}

/// A person that links may point to: exists, not deleted, not merged away.
pub(crate) fn require_live_person(conn: &Connection, id: &str) -> Result<()> {
    let ok: Option<i64> = conn.query_row("SELECT 1 FROM person WHERE id = ?1 AND deleted_at IS NULL AND merged_into IS NULL", [id], |r| r.get(0)).optional()?;
    ok.map(|_| ()).ok_or_else(|| StoreError::NotFound { kind: "person", id: id.to_string() })
}

impl Store {
    /// Raw connection for tests that must prove database-level rules. Not for application code.
    #[doc(hidden)]
    pub fn conn_for_tests(&self) -> &Connection {
        &self.conn
    }
}
