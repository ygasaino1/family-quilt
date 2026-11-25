//! Jalali (Solar Hijri) calendar, converted exactly to and from the Gregorian day count.
//!
//! The leap rule is the Borkowski cycle table used by the `jalaali-js` library, which follows the
//! official Iranian calendar for the years people in a family tree can have (about 1 AP to 3177 AP).
//! The test compares 3,000+ days with that library's output.
//! Months 1-6 have 31 days, 7-11 have 30, and 12 has 29 (30 in a leap year).

use crate::dates::{civil_from_days, days_from_civil, Day};

const BREAKS: [i32; 20] = [-61, 9, 38, 199, 426, 686, 756, 818, 1111, 1181, 1210, 1635, 2060, 2097, 2192, 2262, 2324, 2394, 2456, 3178];
pub const MIN_YEAR: i32 = BREAKS[0];
pub const MAX_YEAR: i32 = BREAKS[19] - 1;

/// `(years since the last leap year (0 = this year is leap), gregorian year, March day of 1 Farvardin)`.
fn jal_cal(jy: i32) -> (i32, i32, i32) {
    let gy = jy + 621;
    let mut leap_j: i32 = -14;
    let mut jp = BREAKS[0];
    let (mut jump, mut jm) = (0, 0);
    for &b in &BREAKS[1..] {
        jm = b;
        jump = jm - jp;
        if jy < jm {
            break;
        }
        leap_j += (jump / 33) * 8 + (jump % 33) / 4;
        jp = jm;
    }
    let _ = jm;
    let n = jy - jp;
    leap_j += (n / 33) * 8 + ((n % 33) + 3) / 4;
    if jump % 33 == 4 && jump - n == 4 {
        leap_j += 1;
    }
    let leap_g = gy / 4 - ((gy / 100 + 1) * 3) / 4 - 150;
    let march = 20 + leap_j - leap_g;
    let mut adj = n;
    if jump - n < 6 {
        adj = n - jump + ((jump + 4) / 33) * 33;
    }
    let mut leap = ((adj + 1) % 33 - 1) % 4;
    if leap == -1 {
        leap = 4;
    }
    (leap, gy, march)
}

pub fn is_leap(jy: i32) -> bool {
    (MIN_YEAR..=MAX_YEAR).contains(&jy) && jal_cal(jy).0 == 0
}

pub fn days_in_month(jy: i32, jm: u32) -> u32 {
    match jm {
        1..=6 => 31,
        7..=11 => 30,
        12 if is_leap(jy) => 30,
        12 => 29,
        _ => 0,
    }
}

/// Why a Jalali date is not valid, or `None` if it is.
pub fn check(jy: i32, jm: u32, jd: u32) -> Option<String> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&jy) {
        return Some(format!("Jalali year must be between {MIN_YEAR} and {MAX_YEAR}"));
    }
    if !(1..=12).contains(&jm) {
        return Some("Jalali month must be 01-12".into());
    }
    if jd == 0 || jd > days_in_month(jy, jm) {
        return Some(format!("month {jm:02} of Jalali year {jy} has {} days", days_in_month(jy, jm)));
    }
    None
}

/// Jalali date (validated by the caller with [`check`]) to days since 1970-01-01.
pub fn to_days(jy: i32, jm: u32, jd: u32) -> Day {
    let (_, gy, march) = jal_cal(jy);
    let jm = jm as i32;
    days_from_civil(gy, 3, march as u32) + (jm - 1) * 31 - (jm / 7) * (jm - 7) + jd as i32 - 1
}

/// Days since 1970-01-01 to `(year, month, day)` in the Jalali calendar.
pub fn from_days(z: Day) -> (i32, u32, u32) {
    let (gy, _, _) = civil_from_days(z);
    let mut jy = (gy - 621).min(MAX_YEAR);
    let (leap, g, march) = jal_cal(jy);
    let mut k = z - days_from_civil(g, 3, march as u32);
    if k >= 0 {
        if k <= 185 {
            return (jy, 1 + (k / 31) as u32, (k % 31) as u32 + 1);
        }
        k -= 186;
    } else {
        jy -= 1;
        k += 179;
        if leap == 1 {
            k += 1;
        }
    }
    (jy, 7 + (k / 30) as u32, (k % 30) as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nowruz_and_leap_years() {
        // 1 Farvardin 1403 = 2024-03-20; 1 Farvardin 1404 = 2025-03-21.
        assert_eq!(to_days(1403, 1, 1), days_from_civil(2024, 3, 20));
        assert_eq!(to_days(1404, 1, 1), days_from_civil(2025, 3, 21));
        assert!(is_leap(1403) && !is_leap(1404));
        assert_eq!(days_in_month(1403, 12), 30);
        assert_eq!(days_in_month(1404, 12), 29);
        // Leap years follow the 33-year pattern: 1325, 1329, 1333 ... 1399, 1403 (1330 and 1400 are not).
        assert!(is_leap(1325) && is_leap(1329) && !is_leap(1330) && is_leap(1399) && !is_leap(1400));
        assert!(check(1404, 12, 30).is_some() && check(1403, 12, 30).is_none());
        assert!(check(1329, 7, 31).is_some() && check(1329, 13, 1).is_some() && check(1329, 1, 31).is_none());
    }

    #[test]
    fn round_trips_every_day_for_four_centuries() {
        let (a, b) = (days_from_civil(1700, 1, 1), days_from_civil(2100, 1, 1));
        for z in (a..b).step_by(1) {
            let (y, m, d) = from_days(z);
            assert!(check(y, m, d).is_none(), "{z}: {y}-{m}-{d}");
            assert_eq!(to_days(y, m, d), z);
        }
    }

    /// Leap flag of every supported year, from `jalaali-js` (including the first year of each 33-year cycle).
    #[test]
    fn leap_years_match_the_library() {
        for line in include_str!("../tests/data/jalali_leap.txt").lines().filter(|l| !l.is_empty()) {
            let (y, l) = line.split_once(' ').unwrap();
            assert_eq!(is_leap(y.parse().unwrap()), l == "1", "{line}");
        }
    }

    /// `jalaali-js` output for every 37th day from 1800 to 2200 (see tests/data/jalali_oracle.txt).
    #[test]
    fn matches_the_jalaali_js_library() {
        let data = include_str!("../tests/data/jalali_oracle.txt");
        let mut n = 0;
        for line in data.lines().filter(|l| !l.trim().is_empty()) {
            let v: Vec<i32> = line.split(' ').map(|x| x.parse().unwrap()).collect();
            let z = days_from_civil(v[0], v[1] as u32, v[2] as u32);
            assert_eq!(from_days(z), (v[3], v[4] as u32, v[5] as u32), "{line}");
            assert_eq!(to_days(v[3], v[4] as u32, v[5] as u32), z, "{line}");
            n += 1;
        }
        assert!(n > 3000, "{n}");
    }
}
