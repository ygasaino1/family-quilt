//! UUIDv7 ids that are **strictly increasing within one process**.
//!
//! Layout (RFC 9562): 48-bit unix-ms timestamp, version nibble 7, a 12-bit
//! counter (in the `rand_a` field), variant bits, then random bits. The counter
//! makes ids made in the same millisecond sort in creation order; if the
//! counter overflows we borrow the next millisecond; if the wall clock steps
//! backwards we keep using the last timestamp. So `new_id() < new_id()` always.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

struct State {
    ms: u64,
    seq: u16,
}

static STATE: Mutex<State> = Mutex::new(State { ms: 0, seq: 0 });

fn wall_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the operating system random generator failed");
    b
}

/// A fresh UUIDv7.
pub fn new_uuid() -> Uuid {
    let rnd = random::<10>();
    // Random start below 0x400 leaves headroom (3072 ids) before borrowing a millisecond.
    let start = u16::from_be_bytes([rnd[8] & 0x03, rnd[9]]);
    let (ms, seq) = {
        let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let wall = wall_ms();
        if wall > st.ms {
            st.ms = wall;
            st.seq = start;
        } else if st.seq >= 0x0fff {
            st.ms += 1;
            st.seq = start;
        } else {
            st.seq += 1;
        }
        (st.ms, st.seq)
    };
    let mut b = [0u8; 16];
    b[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
    b[6] = 0x70 | (seq >> 8) as u8;
    b[7] = (seq & 0xff) as u8;
    b[8] = 0x80 | (rnd[0] & 0x3f);
    b[9..].copy_from_slice(&rnd[1..8]);
    Uuid::from_bytes(b)
}

/// Text form used in the database (lower-case, hyphenated).
pub fn new_id() -> String {
    new_uuid().hyphenated().to_string()
}

/// True if `s` looks like one of our ids (any UUID text form we could have written).
pub fn is_valid_id(s: &str) -> bool {
    s.len() == 36 && Uuid::parse_str(s).is_ok()
}

/// Current time as UTC ISO-8601 with millisecond precision, e.g. `2026-11-17T10:19:03.123Z`.
pub fn now_iso() -> String {
    let ms = wall_ms();
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = crate::dates::civil_from_days(days as i32);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, (rem % 3600) / 60, rem % 60, ms % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_is_uuid_v7() {
        let id = new_id();
        assert_eq!(id.len(), 36);
        let u = Uuid::parse_str(&id).unwrap();
        assert_eq!(u.get_version_num(), 7);
        assert!(is_valid_id(&id));
        assert!(!is_valid_id("not-an-id"));
    }

    #[test]
    fn strictly_increasing_in_one_thread() {
        let mut prev = new_id();
        for _ in 0..50_000 {
            let next = new_id();
            assert!(next > prev, "{next} should sort after {prev}");
            prev = next;
        }
    }

    #[test]
    fn unique_across_threads() {
        let handles: Vec<_> = (0..8).map(|_| std::thread::spawn(|| (0..20_000).map(|_| new_id()).collect::<Vec<_>>())).collect();
        let mut all: Vec<String> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n);
    }

    #[test]
    fn survives_counter_overflow_and_clock_going_backwards() {
        // Force the shared state far into the future, then make many ids: the
        // counter must overflow into later milliseconds without going backwards.
        {
            let mut st = STATE.lock().unwrap();
            st.ms = wall_ms() + 60_000;
            st.seq = 0x0ff0;
        }
        let mut prev = new_id();
        for _ in 0..10_000 {
            let next = new_id();
            assert!(next > prev);
            prev = next;
        }
    }

    #[test]
    fn now_iso_shape() {
        let t = now_iso();
        assert_eq!(t.len(), 24);
        assert!(t.ends_with('Z'));
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[10..11], "T");
    }
}
