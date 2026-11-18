//! A tree on disk: create, open, protect with a password, change it, recover, back up.
//!
//! Layout next to the database file `family.db`:
//!   * `family.db.keys`  key file (only for protected trees; see `crypto`)
//!   * `family-backups/` default folder for pre-change safety backups
//!
//! Whether a tree is protected is decided by its **file header**: a plain SQLite
//! file starts with `SQLite format 3\0`, a SQLCipher file looks like random bytes.
//! All functions here that change a file on disk need the tree to be **closed**
//! (no `Store` open on it) and take a backup first.

use crate::crypto::{generate_data_key, sqlcipher_raw_key, DataKey, KdfParams, KeyFile, RecoveryKey};
use crate::error::{Result, StoreError};
use crate::ids::now_iso;
use crate::store::{apply_key, open_connection, Store};
use rusqlite::Connection;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

const PLAIN_HEADER: &[u8; 16] = b"SQLite format 3\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeState {
    Missing,
    Plain,
    Protected,
}

#[derive(Debug, Clone, Copy)]
pub enum Credentials<'a> {
    /// For unprotected trees.
    None,
    Password(&'a str),
    RecoveryKey(&'a str),
}

#[derive(Debug, Clone, Copy)]
pub enum Protection<'a> {
    None,
    Password { password: &'a str, kdf: KdfParams },
}

#[derive(Debug, Clone, Copy)]
pub struct OpenOptions {
    /// Pause after a wrong password (slows down guessing). Tests set it to zero.
    pub wrong_password_delay: Duration,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions { wrong_password_delay: Duration::from_millis(1000) }
    }
}

/// `family.db` -> `family.db.keys` (keeps non-ASCII names intact).
pub fn key_file_path(db: &Path) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(".keys");
    PathBuf::from(s)
}

/// Default folder for backups of `db`: `<dir>/<stem>-backups`.
pub fn default_backup_dir(db: &Path) -> PathBuf {
    let stem = db.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "tree".into());
    db.parent().unwrap_or_else(|| Path::new(".")).join(format!("{stem}-backups"))
}

fn sibling(db: &Path, suffix: &str) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// `20261117T101903123Z`, safe in file names on every OS.
fn compact_timestamp() -> String {
    now_iso().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// A path as SQL string literal contents (single quotes doubled). Must be valid UTF-8.
fn sql_path(p: &Path) -> Result<String> {
    p.to_str().map(|s| s.replace('\'', "''")).ok_or_else(|| StoreError::invalid("path", "the folder or file name contains characters that cannot be used"))
}

pub fn tree_state(path: &Path) -> Result<TreeState> {
    let mut f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TreeState::Missing),
        Err(e) => return Err(e.into()),
    };
    let mut head = [0u8; 16];
    let n = f.read(&mut head)?;
    Ok(if n == 0 || (n == 16 && &head == PLAIN_HEADER) { TreeState::Plain } else { TreeState::Protected })
}

fn remove_if_exists(p: &Path) -> Result<()> {
    match fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Stale `-wal`/`-shm` files must never be paired with a different main file.
fn remove_wal_files(db: &Path) -> Result<()> {
    remove_if_exists(&sibling(db, "-wal"))?;
    remove_if_exists(&sibling(db, "-shm"))
}

fn unlock(path: &Path, creds: Credentials<'_>, delay: Duration) -> Result<DataKey> {
    let kf = KeyFile::read(&key_file_path(path))?;
    let result = match creds {
        Credentials::None => return Err(StoreError::PasswordRequired),
        Credentials::Password(p) => kf.unlock_with_password(p),
        Credentials::RecoveryKey(r) => kf.unlock_with_recovery_key(r),
    };
    if matches!(result, Err(StoreError::WrongPassword)) {
        std::thread::sleep(delay);
    }
    result
}

impl Store {
    /// Create a new tree file. Returns the recovery key when the tree is protected (show it once).
    pub fn create_tree(path: &Path, protection: Protection<'_>) -> Result<(Store, Option<RecoveryKey>)> {
        if path.exists() {
            return Err(StoreError::AlreadyExists(path.display().to_string()));
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let result = match protection {
            Protection::None => open_connection(path, None).and_then(|c| Store::from_connection(c, path.to_path_buf(), false)).map(|s| (s, None)),
            Protection::Password { password, kdf } => (|| {
                let master = generate_data_key();
                let (kf, rk) = KeyFile::create(&master, password, kdf)?;
                kf.write_atomic(&key_file_path(path))?;
                let conn = open_connection(path, Some(&master))?;
                Ok((Store::from_connection(conn, path.to_path_buf(), true)?, Some(rk)))
            })(),
        };
        if result.is_err() {
            // Do not leave a half-made tree behind.
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(key_file_path(path));
            let _ = remove_wal_files(path);
        }
        result
    }

    /// Open an existing tree. Protected trees need `Password` or `RecoveryKey`.
    pub fn open_tree(path: &Path, creds: Credentials<'_>, opts: &OpenOptions) -> Result<Store> {
        match tree_state(path)? {
            TreeState::Missing => Err(StoreError::TreeMissing(path.display().to_string())),
            TreeState::Plain => Store::from_connection(open_connection(path, None)?, path.to_path_buf(), false),
            TreeState::Protected => {
                let key = unlock(path, creds, opts.wrong_password_delay)?;
                Store::from_connection(open_connection(path, Some(&key))?, path.to_path_buf(), true)
            }
        }
    }

    /// Flush the write-ahead log and close the file.
    pub fn close(self) -> Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        self.conn.close().map_err(|(_, e)| StoreError::Sqlite(e))
    }

    /// Consistent copy of the whole tree into a new file (`VACUUM INTO`). A protected tree gives a
    /// protected backup (the key file is copied next to it); if the copy came out unencrypted it is
    /// deleted and an error is returned, so a backup can never silently leak data.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            return Err(StoreError::AlreadyExists(dest.display().to_string()));
        }
        if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        self.conn.execute_batch(&format!("VACUUM INTO '{}'", sql_path(dest)?))?;
        if self.protected {
            if tree_state(dest)? != TreeState::Protected {
                let _ = fs::remove_file(dest);
                return Err(StoreError::EncryptionUnavailable);
            }
            let src = self.path.as_deref().ok_or_else(|| StoreError::KeyFile("no key file for an in-memory tree".into()))?;
            fs::copy(key_file_path(src), key_file_path(dest))?;
        }
        Ok(())
    }

    /// Timestamped backup inside `dir` (default folder: [`default_backup_dir`]). Returns the new file.
    pub fn backup_now(&self, dir: Option<&Path>, label: &str) -> Result<PathBuf> {
        let src = self.path.as_deref().ok_or_else(|| StoreError::invalid("path", "an in-memory tree has no file"))?;
        let dir = dir.map(Path::to_path_buf).unwrap_or_else(|| default_backup_dir(src));
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "tree".into());
        let dest = dir.join(format!("{stem}-{label}-{}.db", compact_timestamp()));
        self.backup_to(&dest)?;
        Ok(dest)
    }
}

/// Export every table into a new database with the given key (`None` = plain), then check it.
fn export_copy(src: &Connection, dest: &Path, dest_key: Option<&DataKey>, version: u32) -> Result<()> {
    let key_clause = match dest_key {
        Some(k) => {
            let raw = sqlcipher_raw_key(k); // x'..'
            Zeroizing::new(format!("'{}'", raw.replace('\'', "''")))
        }
        None => Zeroizing::new("''".to_string()),
    };
    remove_if_exists(dest)?;
    let attach = Zeroizing::new(format!("ATTACH DATABASE '{}' AS exported KEY {};", sql_path(dest)?, *key_clause));
    src.execute_batch(&attach)?;
    let run = || -> Result<()> {
        let mut s = src.prepare("SELECT sqlcipher_export('exported')")?;
        let mut rows = s.query([])?;
        while rows.next()?.is_some() {}
        src.execute_batch(&format!("PRAGMA exported.user_version = {version};"))?;
        Ok(())
    };
    let r = run();
    let d = src.execute_batch("DETACH DATABASE exported;");
    r?;
    d?;
    // Verify: opens with the right key and passes SQLite's own check.
    let check = open_connection(dest, dest_key)?;
    let verdict: String = check.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if verdict != "ok" {
        return Err(StoreError::Corrupt(format!("the converted copy failed its check: {verdict}")));
    }
    let v: u32 = check.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if v != version {
        return Err(StoreError::Corrupt("the converted copy lost its schema version".into()));
    }
    check.close().map_err(|(_, e)| StoreError::Sqlite(e))?;
    Ok(())
}

/// Turn an unprotected tree into a protected one. A backup of the plain tree is made first
/// (in `backup_dir`, default next to the tree). Returns the recovery key to show once.
pub fn protect_tree(path: &Path, password: &str, kdf: KdfParams, backup_dir: Option<&Path>) -> Result<RecoveryKey> {
    match tree_state(path)? {
        TreeState::Missing => return Err(StoreError::TreeMissing(path.display().to_string())),
        TreeState::Protected => return Err(StoreError::Conflict("this tree already has a password".into())),
        TreeState::Plain => {}
    }
    let master = generate_data_key();
    let (kf, rk) = KeyFile::create(&master, password, kdf)?; // also rejects an empty password
    let store = Store::open_tree(path, Credentials::None, &OpenOptions::default())?;
    store.backup_now(backup_dir, "before-password")?;
    let version = store.schema_version()?;
    store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let tmp = sibling(path, ".protect.tmp");
    let converted = export_copy(&store.conn, &tmp, Some(&master), version);
    store.close()?;
    if let Err(e) = converted {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Key file first, then swap the database in: a crash in between leaves the old plain tree usable.
    kf.write_atomic(&key_file_path(path))?;
    remove_wal_files(path)?;
    fs::rename(&tmp, path)?;
    tracing::info!("tree is now password protected");
    Ok(rk)
}

/// Remove the password: the tree becomes a plain SQLite file again. Backup (protected) first.
pub fn unprotect_tree(path: &Path, creds: Credentials<'_>, opts: &OpenOptions, backup_dir: Option<&Path>) -> Result<()> {
    if tree_state(path)? != TreeState::Protected {
        return Err(StoreError::Conflict("this tree has no password".into()));
    }
    let store = Store::open_tree(path, creds, opts)?;
    store.backup_now(backup_dir, "before-password-removal")?;
    let version = store.schema_version()?;
    store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let tmp = sibling(path, ".unprotect.tmp");
    let converted = export_copy(&store.conn, &tmp, None, version);
    store.close()?;
    if let Err(e) = converted {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    remove_wal_files(path)?;
    fs::rename(&tmp, path)?;
    remove_if_exists(&key_file_path(path))?;
    tracing::info!("password protection removed");
    Ok(())
}

/// Change the password. The data key stays the same and is re-wrapped (atomic file swap), so the
/// database itself is untouched; the old key file is copied into the backup folder first.
pub fn change_password(path: &Path, current_password: &str, new_password: &str, opts: &OpenOptions, backup_dir: Option<&Path>) -> Result<()> {
    rewrap(path, Credentials::Password(current_password), new_password, opts, backup_dir)
}

/// Forgot the password: set a new one using the recovery key.
pub fn reset_password_with_recovery_key(path: &Path, recovery_key: &str, new_password: &str, opts: &OpenOptions, backup_dir: Option<&Path>) -> Result<()> {
    rewrap(path, Credentials::RecoveryKey(recovery_key), new_password, opts, backup_dir)
}

fn rewrap(path: &Path, creds: Credentials<'_>, new_password: &str, opts: &OpenOptions, backup_dir: Option<&Path>) -> Result<()> {
    if tree_state(path)? != TreeState::Protected {
        return Err(StoreError::Conflict("this tree has no password".into()));
    }
    let master = unlock(path, creds, opts.wrong_password_delay)?;
    let kp = key_file_path(path);
    let kf = KeyFile::read(&kp)?;
    let new_kf = kf.with_new_password(&master, new_password)?;
    // Safety copy of the old key file.
    let dir = backup_dir.map(Path::to_path_buf).unwrap_or_else(|| default_backup_dir(path));
    fs::create_dir_all(&dir)?;
    let name = kp.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "tree.keys".into());
    fs::copy(&kp, dir.join(format!("{name}.{}.bak", compact_timestamp())))?;
    new_kf.write_atomic(&kp)?;
    // Prove the new key file really opens the database before reporting success.
    let key = new_kf.unlock_with_password(new_password)?;
    let conn = Connection::open(path)?;
    apply_key(&conn, &key)?;
    Ok(())
}
