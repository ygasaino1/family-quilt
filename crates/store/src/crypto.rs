//! Key handling for password-protected trees (section 16).
//!
//! Design (the "recovery key" proposal from the spec):
//!
//! * The database file is encrypted by SQLCipher with a **random 256-bit data
//!   key** that never leaves memory except inside the key file.
//! * A small JSON **key file** `<tree>.keys` sits next to the database. It holds
//!   the data key wrapped **twice**: once under a key derived (Argon2id) from the
//!   password, once under a key derived from a generated **recovery key**
//!   (160 random bits, shown once, base32). Wrapping is XChaCha20-Poly1305.
//! * Changing the password only re-wraps the data key (instant, atomic rename).
//!   Losing both the password and the recovery key means the data is gone.
//!
//! Nothing here is ever logged. Secrets live in `Zeroizing` buffers.

use crate::error::{Result, StoreError};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, Zeroizing};

pub const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const RECOVERY_BYTES: usize = 20;
const FORMAT_VERSION: u32 = 1;

pub type DataKey = Zeroizing<[u8; KEY_LEN]>;

/// Argon2id cost settings, stored in the key file so they can change later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    /// Iterations.
    pub t: u32,
    /// Lanes.
    pub p: u32,
}

impl KdfParams {
    /// Default for real use: 64 MiB, 3 passes (a few hundred ms on a normal PC).
    pub const STANDARD: KdfParams = KdfParams { m_kib: 64 * 1024, t: 3, p: 1 };
    /// Cheap settings so tests run fast. Never use for real data.
    pub const FAST_FOR_TESTS: KdfParams = KdfParams { m_kib: 8, t: 1, p: 1 };

    /// A tampered key file must not be able to make us allocate gigabytes.
    fn check(&self) -> Result<()> {
        if self.m_kib < 8 || self.m_kib > 512 * 1024 || self.t == 0 || self.t > 20 || self.p == 0 || self.p > 8 {
            return Err(StoreError::KeyFile("unreasonable key-derivation settings".into()));
        }
        Ok(())
    }
}

/// The printable recovery key. `Debug` never shows it.
pub struct RecoveryKey(Zeroizing<String>);

impl RecoveryKey {
    fn from_bytes(b: &[u8; RECOVERY_BYTES]) -> Self {
        let raw = base32_encode(b);
        let grouped: Vec<&str> = raw.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).expect("ascii")).collect();
        RecoveryKey(Zeroizing::new(grouped.join("-")))
    }
    pub fn generate() -> Self {
        let mut b = Zeroizing::new([0u8; RECOVERY_BYTES]);
        fill_random(&mut *b);
        Self::from_bytes(&b)
    }
    /// The text to show the user once, e.g. `ABCD-EFGH-IJKL-MNOP-QRST-UVWX-YZ23-4567`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryKey(..)")
    }
}

/// Parse what a person typed (any case, with or without dashes/spaces).
fn parse_recovery_key(input: &str) -> Result<Zeroizing<Vec<u8>>> {
    let cleaned: Zeroizing<String> = Zeroizing::new(input.chars().filter(|c| !c.is_whitespace() && *c != '-').flat_map(|c| c.to_uppercase()).collect());
    match base32_decode(&cleaned) {
        Some(b) if b.len() == RECOVERY_BYTES => Ok(Zeroizing::new(b)),
        _ => Err(StoreError::WrongPassword),
    }
}

pub fn fill_random(buf: &mut [u8]) {
    getrandom::fill(buf).expect("the operating system random generator failed");
}

pub fn generate_data_key() -> DataKey {
    let mut k = Zeroizing::new([0u8; KEY_LEN]);
    fill_random(&mut *k);
    k
}

/// The SQLCipher raw-key pragma value for a data key: `x'<64 hex digits>'`.
pub fn sqlcipher_raw_key(key: &DataKey) -> Zeroizing<String> {
    Zeroizing::new(format!("x'{}'", hex_encode(&key[..])))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Slot {
    salt: String,
    nonce: String,
    wrapped: String,
}

/// Contents of `<tree>.keys`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFile {
    version: u32,
    kdf: KdfParams,
    password: Slot,
    recovery: Slot,
}

fn normalize_password(pw: &str) -> Result<Zeroizing<Vec<u8>>> {
    if pw.is_empty() {
        return Err(StoreError::EmptyPassword);
    }
    // NFC so the same password typed on different keyboards/systems matches.
    let nfc: Zeroizing<String> = Zeroizing::new(pw.nfc().collect());
    Ok(Zeroizing::new(nfc.as_bytes().to_vec()))
}

fn derive_kek(secret: &[u8], salt: &[u8], p: &KdfParams) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    p.check()?;
    let params = Params::new(p.m_kib, p.t, p.p, Some(KEY_LEN)).map_err(|e| StoreError::KeyFile(format!("bad key-derivation settings: {e}")))?;
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(secret, salt, &mut *out).map_err(|e| StoreError::KeyFile(format!("key derivation failed: {e}")))?;
    Ok(out)
}

fn aad(role: &str) -> Vec<u8> {
    format!("family-quilt/data-key/v{FORMAT_VERSION}/{role}").into_bytes()
}

fn wrap(master: &DataKey, secret: &[u8], role: &str, p: &KdfParams) -> Result<Slot> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    fill_random(&mut salt);
    fill_random(&mut nonce);
    let kek = derive_kek(secret, &salt, p)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&kek[..]));
    let ct = cipher.encrypt(XNonce::from_slice(&nonce), Payload { msg: &master[..], aad: &aad(role) }).map_err(|_| StoreError::KeyFile("could not wrap the data key".into()))?;
    Ok(Slot { salt: hex_encode(&salt), nonce: hex_encode(&nonce), wrapped: hex_encode(&ct) })
}

fn unwrap(slot: &Slot, secret: &[u8], role: &str, p: &KdfParams) -> Result<DataKey> {
    let bad = |what: &str| StoreError::KeyFile(format!("{what} is damaged"));
    let salt = hex_decode(&slot.salt).ok_or_else(|| bad("salt"))?;
    let nonce = hex_decode(&slot.nonce).filter(|n| n.len() == NONCE_LEN).ok_or_else(|| bad("nonce"))?;
    let wrapped = hex_decode(&slot.wrapped).ok_or_else(|| bad("wrapped key"))?;
    if salt.len() < 8 {
        return Err(bad("salt"));
    }
    let kek = derive_kek(secret, &salt, p)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&kek[..]));
    let mut pt = cipher.decrypt(XNonce::from_slice(&nonce), Payload { msg: &wrapped, aad: &aad(role) }).map_err(|_| StoreError::WrongPassword)?;
    if pt.len() != KEY_LEN {
        pt.zeroize();
        return Err(bad("wrapped key"));
    }
    let mut k = Zeroizing::new([0u8; KEY_LEN]);
    k.copy_from_slice(&pt);
    pt.zeroize();
    Ok(k)
}

impl KeyFile {
    /// Wrap `master` under the password and under a new recovery key. Returns both.
    pub fn create(master: &DataKey, password: &str, params: KdfParams) -> Result<(KeyFile, RecoveryKey)> {
        let pw = normalize_password(password)?;
        let mut rk = Zeroizing::new([0u8; RECOVERY_BYTES]);
        fill_random(&mut *rk);
        let recovery_key = RecoveryKey::from_bytes(&rk);
        let kf = KeyFile { version: FORMAT_VERSION, kdf: params, password: wrap(master, &pw, "password", &params)?, recovery: wrap(master, &rk[..], "recovery", &params)? };
        Ok((kf, recovery_key))
    }

    pub fn unlock_with_password(&self, password: &str) -> Result<DataKey> {
        self.check_version()?;
        let pw = normalize_password(password).map_err(|_| StoreError::WrongPassword)?;
        unwrap(&self.password, &pw, "password", &self.kdf)
    }

    pub fn unlock_with_recovery_key(&self, recovery_key: &str) -> Result<DataKey> {
        self.check_version()?;
        let rk = parse_recovery_key(recovery_key)?;
        unwrap(&self.recovery, &rk, "recovery", &self.kdf)
    }

    /// Replace the password wrapper (the recovery wrapper and the data key stay the same).
    pub fn with_new_password(&self, master: &DataKey, new_password: &str) -> Result<KeyFile> {
        let pw = normalize_password(new_password)?;
        Ok(KeyFile { password: wrap(master, &pw, "password", &self.kdf)?, ..self.clone() })
    }

    fn check_version(&self) -> Result<()> {
        if self.version != FORMAT_VERSION {
            return Err(StoreError::KeyFile(format!("unknown key file version {}", self.version)));
        }
        self.kdf.check()
    }

    pub fn read(path: &Path) -> Result<KeyFile> {
        let bytes = fs::read(path).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { StoreError::KeyFile("the key file is missing".into()) } else { e.into() })?;
        serde_json::from_slice(&bytes).map_err(|e| StoreError::KeyFile(format!("cannot read it: {e}")))
    }

    /// Write to a temporary file, flush it to disk, then rename over the target.
    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("keys.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(self)?)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

// ───────────────────────────── small encoders ─────────────────────────────

pub fn hex_encode(b: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for &x in b {
        s.push(H[(x >> 4) as usize] as char);
        s.push(H[(x & 15) as usize] as char);
    }
    s
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    let val = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    s.as_bytes().chunks(2).map(|p| Some(val(p[0])? << 4 | val(p[1])?)).collect()
}

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = buf << 8 | u32::from(b);
        bits += 8;
        while bits >= 5 {
            out.push(B32[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = B32.iter().position(|&x| x == c)? as u32;
        buf = buf << 5 | v;
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: KdfParams = KdfParams::FAST_FOR_TESTS;

    #[test]
    fn hex_and_base32_round_trip() {
        let data: Vec<u8> = (0..=255).collect();
        assert_eq!(hex_decode(&hex_encode(&data)).unwrap(), data);
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
        let b: Vec<u8> = (1..=20).collect();
        let e = base32_encode(&b);
        assert_eq!(e.len(), 32);
        assert_eq!(base32_decode(&e).unwrap(), b);
        // RFC 4648 test vector
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn password_and_recovery_both_open_the_same_key() {
        let master = generate_data_key();
        let (kf, rk) = KeyFile::create(&master, "correct horse", P).unwrap();
        assert_eq!(&kf.unlock_with_password("correct horse").unwrap()[..], &master[..]);
        assert_eq!(&kf.unlock_with_recovery_key(rk.as_str()).unwrap()[..], &master[..]);
        // typing style does not matter
        let lower = rk.as_str().to_lowercase().replace('-', " ");
        assert_eq!(&kf.unlock_with_recovery_key(&lower).unwrap()[..], &master[..]);
    }

    #[test]
    fn wrong_secrets_are_rejected() {
        let master = generate_data_key();
        let (kf, rk) = KeyFile::create(&master, "pw", P).unwrap();
        assert!(matches!(kf.unlock_with_password("PW"), Err(StoreError::WrongPassword)));
        assert!(matches!(kf.unlock_with_password(""), Err(StoreError::WrongPassword)));
        assert!(matches!(kf.unlock_with_recovery_key("AAAA-BBBB"), Err(StoreError::WrongPassword)));
        // a password is not accepted as a recovery key and vice versa
        assert!(kf.unlock_with_password(rk.as_str()).is_err());
        assert!(matches!(KeyFile::create(&master, "", P), Err(StoreError::EmptyPassword)));
    }

    #[test]
    fn unicode_passwords_are_normalised() {
        let master = generate_data_key();
        // "é" as one code point vs "e" + combining accent
        let (kf, _) = KeyFile::create(&master, "caf\u{e9}-رمز", P).unwrap();
        assert!(kf.unlock_with_password("cafe\u{301}-رمز").is_ok());
    }

    #[test]
    fn changing_password_keeps_recovery_working() {
        let master = generate_data_key();
        let (kf, rk) = KeyFile::create(&master, "old", P).unwrap();
        let kf2 = kf.with_new_password(&master, "new").unwrap();
        assert!(kf2.unlock_with_password("old").is_err());
        assert_eq!(&kf2.unlock_with_password("new").unwrap()[..], &master[..]);
        assert_eq!(&kf2.unlock_with_recovery_key(rk.as_str()).unwrap()[..], &master[..]);
    }

    #[test]
    fn tampering_is_detected() {
        let master = generate_data_key();
        let (mut kf, _) = KeyFile::create(&master, "pw", P).unwrap();
        let mut w = hex_decode(&kf.password.wrapped).unwrap();
        w[0] ^= 1;
        kf.password.wrapped = hex_encode(&w);
        assert!(kf.unlock_with_password("pw").is_err());
        // absurd cost settings are refused instead of allocating gigabytes
        let (mut kf, _) = KeyFile::create(&master, "pw", P).unwrap();
        kf.kdf.m_kib = u32::MAX;
        assert!(matches!(kf.unlock_with_password("pw"), Err(StoreError::KeyFile(_))));
    }

    #[test]
    fn slots_are_bound_to_their_role() {
        let master = generate_data_key();
        let (mut kf, _) = KeyFile::create(&master, "pw", P).unwrap();
        kf.recovery = kf.password.clone(); // swap a slot into the other role
        assert!(kf.unlock_with_recovery_key("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").is_err());
    }

    #[test]
    fn debug_output_hides_secrets() {
        let rk = RecoveryKey::generate();
        assert_eq!(format!("{rk:?}"), "RecoveryKey(..)");
        assert_eq!(rk.as_str().len(), 32 + 7);
    }
}
