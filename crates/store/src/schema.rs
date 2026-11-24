//! Embedded, numbered schema migrations driven by `PRAGMA user_version`.
//!
//! Rules: migration N is applied exactly once, inside one transaction, together
//! with the version bump, so a crash never leaves a half-migrated file. A file
//! whose version is **newer** than this build knows is refused, never altered.
//! Never edit a released migration: add the next number.

use crate::error::{Result, StoreError};
use rusqlite::Connection;

pub struct Migration {
    pub version: u32,
    pub sql: &'static str,
}

/// All migrations, in order. Version numbers start at 1 and have no gaps.
pub const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, sql: include_str!("../migrations/001_init.sql") },
    Migration { version: 2, sql: include_str!("../migrations/002_proposal_feedback.sql") },
    Migration { version: 3, sql: include_str!("../migrations/003_proposal_withdrawn.sql") },
];

pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

pub fn current_version(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Bring the database up to the latest schema. Safe to call on every start.
pub fn migrate(conn: &mut Connection) -> Result<u32> {
    migrate_with(conn, MIGRATIONS)
}

/// Same as [`migrate`] with an explicit list (used by tests to prove incremental upgrades).
pub fn migrate_with(conn: &mut Connection, migrations: &[Migration]) -> Result<u32> {
    let supported = migrations.last().map_or(0, |m| m.version);
    let mut version = current_version(conn)?;
    if version > supported {
        return Err(StoreError::SchemaTooNew { found: version, supported });
    }
    for (i, m) in migrations.iter().enumerate() {
        debug_assert_eq!(m.version as usize, i + 1, "migrations must be numbered 1, 2, 3 ... without gaps");
        if m.version <= version {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)?;
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
        tracing::info!(version = m.version, "applied schema migration");
        version = m.version;
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn fresh_database_gets_latest_version() {
        let mut c = mem();
        assert_eq!(current_version(&c).unwrap(), 0);
        assert_eq!(migrate(&mut c).unwrap(), latest_version());
        assert_eq!(current_version(&c).unwrap(), latest_version());
    }

    #[test]
    fn running_twice_changes_nothing() {
        let mut c = mem();
        migrate(&mut c).unwrap();
        c.execute("INSERT INTO meta(key,value) VALUES('k','v')", []).unwrap();
        migrate(&mut c).unwrap();
        migrate(&mut c).unwrap();
        let n: i64 = c.query_row("SELECT count(*) FROM meta", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn newer_file_is_refused_untouched() {
        let mut c = mem();
        c.pragma_update(None, "user_version", 99).unwrap();
        match migrate(&mut c) {
            Err(StoreError::SchemaTooNew { found: 99, supported }) => assert_eq!(supported, latest_version()),
            other => panic!("expected SchemaTooNew, got {other:?}"),
        }
        assert_eq!(current_version(&c).unwrap(), 99);
    }

    #[test]
    fn later_migrations_apply_on_top_and_failures_roll_back() {
        let mut c = mem();
        let v1 = [Migration { version: 1, sql: "CREATE TABLE a(x INTEGER);" }];
        migrate_with(&mut c, &v1).unwrap();
        c.execute("INSERT INTO a VALUES (7)", []).unwrap();

        // v2 adds a column; data from v1 survives.
        let v2 = [Migration { version: 1, sql: "CREATE TABLE a(x INTEGER);" }, Migration { version: 2, sql: "ALTER TABLE a ADD COLUMN y TEXT DEFAULT 'z';" }];
        assert_eq!(migrate_with(&mut c, &v2).unwrap(), 2);
        let (x, y): (i64, String) = c.query_row("SELECT x, y FROM a", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((x, y.as_str()), (7, "z"));

        // v3 is broken half-way: nothing of it may remain and the version stays 2.
        let v3 = [
            Migration { version: 1, sql: "CREATE TABLE a(x INTEGER);" },
            Migration { version: 2, sql: "ALTER TABLE a ADD COLUMN y TEXT DEFAULT 'z';" },
            Migration { version: 3, sql: "CREATE TABLE b(q); INSERT INTO nope VALUES (1);" },
        ];
        assert!(migrate_with(&mut c, &v3).is_err());
        assert_eq!(current_version(&c).unwrap(), 2);
        let b: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE name='b'", [], |r| r.get(0)).unwrap();
        assert_eq!(b, 0);
    }
}
