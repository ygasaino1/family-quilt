#![cfg(feature = "sqlcipher")]
mod common;
use common::*;
use family_store::tree::{default_backup_dir, key_file_path};
use family_store::*;
use std::path::Path;
use std::time::{Duration, Instant};

const KDF: KdfParams = KdfParams::FAST_FOR_TESTS;
const NO_DELAY: OpenOptions = OpenOptions { wrong_password_delay: Duration::ZERO };

fn header(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()[..16].to_vec()
}
fn is_plain_sqlite(p: &Path) -> bool {
    header(p) == b"SQLite format 3\0"
}
fn protected(dir: &Path, name: &str, pw: &str) -> (std::path::PathBuf, String, String) {
    let path = dir.join(name);
    let (mut s, rk) = Store::create_tree(&path, Protection::Password { password: pw, kdf: KDF }).unwrap();
    let id = person(&mut s, "Sara", "Adams", Sex::Female);
    s.meta_set(&CTX, "title", "Secret tree").unwrap();
    s.close().unwrap();
    (path, id, rk.unwrap().as_str().to_string())
}

#[test]
fn protected_tree_needs_the_password_and_is_not_plain_sqlite() {
    let d = tempfile::tempdir().unwrap();
    let (path, id, _rk) = protected(d.path(), "family.db", "correct horse");
    assert!(!is_plain_sqlite(&path), "header must not be 'SQLite format 3'");
    assert_eq!(tree_state(&path).unwrap(), TreeState::Protected);
    assert!(key_file_path(&path).exists());
    // the plaintext must not appear anywhere in the file
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.windows(4).any(|w| w == b"Sara") && !bytes.windows(6).any(|w| w == b"Secret"));

    // a plain SQLite connection (no key) cannot read it
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert!(raw.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).is_err());

    assert!(matches!(Store::open_tree(&path, Credentials::None, &NO_DELAY), Err(StoreError::PasswordRequired)));
    assert!(matches!(Store::open_tree(&path, Credentials::Password("wrong"), &NO_DELAY), Err(StoreError::WrongPassword)));
    assert!(matches!(Store::open_tree(&path, Credentials::Password(""), &NO_DELAY), Err(StoreError::WrongPassword)));
    let s = Store::open_tree(&path, Credentials::Password("correct horse"), &NO_DELAY).unwrap();
    assert!(s.is_protected());
    assert_eq!(s.get_person(&CTX, &id).unwrap().display_name, "Sara Adams");
    assert_eq!(s.meta_get(&CTX, "title").unwrap().as_deref(), Some("Secret tree"));
}

#[test]
fn wrong_password_is_slow_and_harmless() {
    let d = tempfile::tempdir().unwrap();
    let (path, id, _) = protected(d.path(), "family.db", "pw");
    let before = std::fs::read(&path).unwrap();
    let t = Instant::now();
    let r = Store::open_tree(&path, Credentials::Password("nope"), &OpenOptions { wrong_password_delay: Duration::from_millis(300) });
    assert!(matches!(r, Err(StoreError::WrongPassword)));
    assert!(t.elapsed() >= Duration::from_millis(300), "a failed attempt is delayed");
    assert_eq!(std::fs::read(&path).unwrap(), before, "a failed attempt does not touch the file");
    assert!(Store::open_tree(&path, Credentials::Password("pw"), &NO_DELAY).unwrap().get_person(&CTX, &id).is_ok());
}

#[test]
fn recovery_key_opens_and_resets_the_password() {
    let d = tempfile::tempdir().unwrap();
    let (path, id, rk) = protected(d.path(), "family.db", "forgotten");
    assert!(Store::open_tree(&path, Credentials::RecoveryKey(&rk), &NO_DELAY).unwrap().get_person(&CTX, &id).is_ok());
    assert!(matches!(Store::open_tree(&path, Credentials::RecoveryKey("AAAA-AAAA"), &NO_DELAY), Err(StoreError::WrongPassword)));

    reset_password_with_recovery_key(&path, &rk, "brand new", &NO_DELAY, None).unwrap();
    assert!(matches!(Store::open_tree(&path, Credentials::Password("forgotten"), &NO_DELAY), Err(StoreError::WrongPassword)));
    assert!(Store::open_tree(&path, Credentials::Password("brand new"), &NO_DELAY).unwrap().get_person(&CTX, &id).is_ok());
    // the recovery key keeps working after a reset
    assert!(Store::open_tree(&path, Credentials::RecoveryKey(&rk), &NO_DELAY).is_ok());
}

#[test]
fn change_password_is_atomic_and_keeps_a_safety_copy() {
    let d = tempfile::tempdir().unwrap();
    let (path, id, rk) = protected(d.path(), "family.db", "old");
    assert!(matches!(change_password(&path, "not the old one", "new", &NO_DELAY, None), Err(StoreError::WrongPassword)));
    assert!(Store::open_tree(&path, Credentials::Password("old"), &NO_DELAY).is_ok(), "failed change leaves everything as it was");
    assert!(matches!(change_password(&path, "old", "", &NO_DELAY, None), Err(StoreError::EmptyPassword)));
    assert!(Store::open_tree(&path, Credentials::Password("old"), &NO_DELAY).is_ok());

    change_password(&path, "old", "new", &NO_DELAY, None).unwrap();
    assert!(Store::open_tree(&path, Credentials::Password("old"), &NO_DELAY).is_err());
    assert!(Store::open_tree(&path, Credentials::Password("new"), &NO_DELAY).unwrap().get_person(&CTX, &id).is_ok());
    assert!(Store::open_tree(&path, Credentials::RecoveryKey(&rk), &NO_DELAY).is_ok());
    let backups: Vec<_> = std::fs::read_dir(default_backup_dir(&path)).unwrap().collect();
    assert_eq!(backups.len(), 1, "old key file was copied first");
}

#[test]
fn backup_of_a_protected_tree_is_protected_and_restorable() {
    let d = tempfile::tempdir().unwrap();
    let (path, id, _) = protected(d.path(), "family.db", "pw");
    let s = Store::open_tree(&path, Credentials::Password("pw"), &NO_DELAY).unwrap();
    let dest = s.backup_now(None, "manual").unwrap();
    assert!(!is_plain_sqlite(&dest), "VACUUM INTO keeps SQLCipher encryption");
    assert!(key_file_path(&dest).exists());
    let bytes = std::fs::read(&dest).unwrap();
    assert!(!bytes.windows(4).any(|w| w == b"Sara"));
    assert!(matches!(s.backup_to(&dest), Err(StoreError::AlreadyExists(_))), "never overwrites a backup");
    drop(s);
    assert!(matches!(Store::open_tree(&dest, Credentials::None, &NO_DELAY), Err(StoreError::PasswordRequired)));
    let b = Store::open_tree(&dest, Credentials::Password("pw"), &NO_DELAY).unwrap();
    assert_eq!(b.get_person(&CTX, &id).unwrap().display_name, "Sara Adams");
}

#[test]
fn unprotected_tree_is_plain_sqlite_and_persists() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("plain.db");
    let (mut s, rk) = Store::create_tree(&path, Protection::None).unwrap();
    assert!(rk.is_none() && !s.is_protected());
    let id = person(&mut s, "Ali", "Karimi", Sex::Male);
    let b = s.backup_now(None, "manual").unwrap();
    assert!(is_plain_sqlite(&b) && !key_file_path(&b).exists());
    s.close().unwrap();
    assert!(is_plain_sqlite(&path) && !key_file_path(&path).exists());
    let s = Store::open_tree(&path, Credentials::None, &NO_DELAY).unwrap();
    assert_eq!(s.get_person(&CTX, &id).unwrap().display_name, "Ali Karimi");
    assert_eq!(s.schema_version().unwrap(), 1);
    // a password given to a plain tree is simply ignored
    assert!(Store::open_tree(&path, Credentials::Password("x"), &NO_DELAY).is_ok());
    assert!(matches!(Store::create_tree(&path, Protection::None), Err(StoreError::AlreadyExists(_))));
    assert!(matches!(Store::open_tree(&d.path().join("missing.db"), Credentials::None, &NO_DELAY), Err(StoreError::TreeMissing(_))));
}

#[test]
fn protect_then_unprotect_keeps_all_data() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("family.db");
    let (mut s, _) = Store::create_tree(&path, Protection::None).unwrap();
    let a = person(&mut s, "A", "A", Sex::Male);
    let b = person(&mut s, "B", "B", Sex::Female);
    parent(&mut s, &a, &b, LinkKind::Adopted).unwrap();
    s.meta_set(&CTX, "title", "T").unwrap();
    s.close().unwrap();

    let rk = protect_tree(&path, "pw", KDF, None).unwrap();
    assert!(!is_plain_sqlite(&path));
    assert!(!std::path::PathBuf::from(format!("{}-wal", path.display())).exists());
    let backups: Vec<_> = std::fs::read_dir(default_backup_dir(&path)).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(backups.len(), 1, "a backup of the plain tree was taken first");
    assert!(is_plain_sqlite(&backups[0]));
    assert!(matches!(protect_tree(&path, "pw2", KDF, None), Err(StoreError::Conflict(_))), "already protected");

    let s = Store::open_tree(&path, Credentials::Password("pw"), &NO_DELAY).unwrap();
    assert_eq!(s.schema_version().unwrap(), 1);
    assert_eq!(s.meta_get(&CTX, "title").unwrap().as_deref(), Some("T"));
    assert_eq!(s.relationships_of(&CTX, &b).unwrap()[0].kind, Some(LinkKind::Adopted));
    s.close().unwrap();
    assert!(Store::open_tree(&path, Credentials::RecoveryKey(rk.as_str()), &NO_DELAY).is_ok());

    unprotect_tree(&path, Credentials::Password("pw"), &NO_DELAY, None).unwrap();
    assert!(is_plain_sqlite(&path) && !key_file_path(&path).exists());
    let s = Store::open_tree(&path, Credentials::None, &NO_DELAY).unwrap();
    assert_eq!(s.get_person(&CTX, &a).unwrap().display_name, "A A");
    assert_eq!(s.relationships_of(&CTX, &a).unwrap().len(), 1);
    assert!(matches!(unprotect_tree(&path, Credentials::None, &NO_DELAY, None), Err(StoreError::Conflict(_))));
}

#[test]
fn unprotect_with_wrong_password_changes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let (path, id, _) = protected(d.path(), "family.db", "pw");
    assert!(matches!(unprotect_tree(&path, Credentials::Password("bad"), &NO_DELAY, None), Err(StoreError::WrongPassword)));
    assert_eq!(tree_state(&path).unwrap(), TreeState::Protected);
    assert!(Store::open_tree(&path, Credentials::Password("pw"), &NO_DELAY).unwrap().get_person(&CTX, &id).is_ok());
}

#[test]
fn missing_key_file_gives_a_clear_error() {
    let d = tempfile::tempdir().unwrap();
    let (path, _, _) = protected(d.path(), "family.db", "pw");
    std::fs::remove_file(key_file_path(&path)).unwrap();
    assert!(matches!(Store::open_tree(&path, Credentials::Password("pw"), &NO_DELAY), Err(StoreError::KeyFile(_))));
}

#[test]
fn non_ascii_folders_and_names_work_for_plain_and_protected_trees() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("خانواده's tree ✓");
    let (path, id, rk) = protected(&dir, "درخت.db", "رمز عبور");
    let s = Store::open_tree(&path, Credentials::Password("رمز عبور"), &NO_DELAY).unwrap();
    assert!(s.get_person(&CTX, &id).is_ok());
    let b = s.backup_now(None, "دستی").unwrap();
    assert!(b.exists());
    drop(s);
    unprotect_tree(&path, Credentials::RecoveryKey(&rk), &NO_DELAY, None).unwrap();
    assert!(is_plain_sqlite(&path));
}

#[test]
fn failed_create_leaves_no_files_behind() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("x.db");
    assert!(matches!(Store::create_tree(&path, Protection::Password { password: "", kdf: KDF }), Err(StoreError::EmptyPassword)));
    assert!(!path.exists() && !key_file_path(&path).exists());
}
