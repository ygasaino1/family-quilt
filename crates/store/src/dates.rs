//! Fuzzy genealogy dates.
//!
//! Stored as text `1887`, `1887-04` or `1887-04-12` plus a qualifier
//! (`exact | about | before | after`). Comparing and sorting always uses the
//! **earliest possible day** of the stated precision (so `1887` sorts as
//! 1887-01-01 and `1887-04` as 1887-04-01); the qualifier is a label for people
//! and does not move the sort position. All date *formatting* for display lives
//! elsewhere (section 20): this module only parses, validates and compares.

use crate::error::StoreError;
use std::cmp::Ordering;
use std::fmt;

/// Days since 1970-01-01 (negative before). Plain integer so it is cheap to sort and subtract.
pub type Day = i32;

/// Howard Hinnant's `days_from_civil` (proleptic Gregorian).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> Day {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u32;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i32 - 719_468
}

/// Inverse of [`days_from_civil`]: `(year, month, day)`.
pub fn civil_from_days(z: Day) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PartialDate {
    year: u16,
    month: Option<u8>,
    day: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Precision {
    Year,
    Month,
    Day,
}

impl PartialDate {
    /// Validated constructor. A day without a month is not allowed.
    pub fn new(year: u16, month: Option<u8>, day: Option<u8>) -> Result<Self, StoreError> {
        let bad = |why: &str| StoreError::invalid("date", format!("{why} ({})", Self::raw(year, month, day)));
        if !(1..=9999).contains(&year) {
            return Err(bad("year must be between 0001 and 9999"));
        }
        match (month, day) {
            (None, Some(_)) => return Err(bad("a day needs a month")),
            (Some(m), _) if !(1..=12).contains(&m) => return Err(bad("month must be 01-12")),
            (Some(m), Some(d)) if d == 0 || u32::from(d) > days_in_month(i32::from(year), u32::from(m)) => return Err(bad("no such day in that month")),
            _ => {}
        }
        Ok(Self { year, month, day })
    }

    fn raw(y: u16, m: Option<u8>, d: Option<u8>) -> String {
        match (m, d) {
            (None, _) => format!("{y:04}"),
            (Some(m), None) => format!("{y:04}-{m:02}"),
            (Some(m), Some(d)) => format!("{y:04}-{m:02}-{d:02}"),
        }
    }

    /// Parse `YYYY`, `YYYY-MM` or `YYYY-MM-DD` (digits only, zero padded).
    pub fn parse(s: &str) -> Result<Self, StoreError> {
        let bad = || StoreError::invalid("date", format!("'{s}' is not YYYY, YYYY-MM or YYYY-MM-DD"));
        if !s.is_ascii() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
            return Err(bad());
        }
        let parts: Vec<&str> = s.split('-').collect();
        let num = |p: &str, len: usize| -> Result<u16, StoreError> {
            if p.len() != len {
                return Err(bad());
            }
            p.parse::<u16>().map_err(|_| bad())
        };
        match parts.as_slice() {
            [y] => Self::new(num(y, 4)?, None, None),
            [y, m] => Self::new(num(y, 4)?, Some(num(m, 2)? as u8), None),
            [y, m, d] => Self::new(num(y, 4)?, Some(num(m, 2)? as u8), Some(num(d, 2)? as u8)),
            _ => Err(bad()),
        }
    }

    pub fn year(&self) -> u16 {
        self.year
    }
    pub fn month(&self) -> Option<u8> {
        self.month
    }
    pub fn day(&self) -> Option<u8> {
        self.day
    }

    pub fn precision(&self) -> Precision {
        match (self.month, self.day) {
            (None, _) => Precision::Year,
            (Some(_), None) => Precision::Month,
            (Some(_), Some(_)) => Precision::Day,
        }
    }

    /// First day this date could mean. Used for every sort and comparison.
    pub fn earliest_day(&self) -> Day {
        days_from_civil(i32::from(self.year), u32::from(self.month.unwrap_or(1)), u32::from(self.day.unwrap_or(1)))
    }

    /// Last day this date could mean (end of the year / month / the day itself).
    pub fn latest_day(&self) -> Day {
        let y = i32::from(self.year);
        let m = u32::from(self.month.unwrap_or(12));
        let d = u32::from(self.day.unwrap_or_else(|| days_in_month(y, m) as u8));
        days_from_civil(y, m, d)
    }
}

impl fmt::Display for PartialDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&Self::raw(self.year, self.month, self.day))
    }
}

/// Order by earliest possible day; at equal days the less precise date first
/// (`1887` before `1887-01` before `1887-01-01`). Consistent with `Eq`.
impl Ord for PartialDate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.earliest_day().cmp(&other.earliest_day()).then_with(|| (self.precision() as u8).cmp(&(other.precision() as u8)))
    }
}
impl PartialOrd for PartialDate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Qualifier {
    #[default]
    Exact,
    About,
    Before,
    After,
}

impl Qualifier {
    pub fn as_str(self) -> &'static str {
        match self {
            Qualifier::Exact => "exact",
            Qualifier::About => "about",
            Qualifier::Before => "before",
            Qualifier::After => "after",
        }
    }
    pub fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "exact" => Ok(Qualifier::Exact),
            "about" => Ok(Qualifier::About),
            "before" => Ok(Qualifier::Before),
            "after" => Ok(Qualifier::After),
            _ => Err(StoreError::invalid("qualifier", format!("'{s}' is not one of exact, about, before, after"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuzzyDate {
    pub date: PartialDate,
    pub qualifier: Qualifier,
}

impl FuzzyDate {
    pub fn new(date: PartialDate, qualifier: Qualifier) -> Self {
        Self { date, qualifier }
    }
    pub fn parse(s: &str, qualifier: Qualifier) -> Result<Self, StoreError> {
        Ok(Self { date: PartialDate::parse(s)?, qualifier })
    }
    pub fn exact(s: &str) -> Result<Self, StoreError> {
        Self::parse(s, Qualifier::Exact)
    }
    /// Sort key (see module docs).
    pub fn earliest_day(&self) -> Day {
        self.date.earliest_day()
    }
    pub fn latest_day(&self) -> Day {
        self.date.latest_day()
    }
}

impl serde::Serialize for FuzzyDate {
    /// `{"date":"1887-04","qualifier":"about"}`
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("FuzzyDate", 2)?;
        st.serialize_field("date", &self.date.to_string())?;
        st.serialize_field("qualifier", self.qualifier.as_str())?;
        st.end()
    }
}

impl fmt::Display for FuzzyDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.qualifier {
            Qualifier::Exact => write!(f, "{}", self.date),
            q => write!(f, "{} {}", q.as_str(), self.date),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_forms() {
        let cases = [("1887", None, None), ("1887-04", Some(4), None), ("1887-04-12", Some(4), Some(12)), ("0001-01-01", Some(1), Some(1)), ("2024-02-29", Some(2), Some(29))];
        for (s, m, d) in cases {
            let p = PartialDate::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!((p.month(), p.day()), (m, d), "{s}");
            assert_eq!(p.to_string(), s);
        }
    }

    #[test]
    fn rejects_invalid_forms() {
        for s in [
            "",
            "87",
            "18870",
            "1887-4",
            "1887-04-1",
            "1887-13",
            "1887-00",
            "1887-04-31",
            "1900-02-29",
            "2023-02-29",
            "1887/04/12",
            "1887-04-12-01",
            "-1887",
            "18a7",
            "۱۸۸۷",
            "0000",
            "1887-04-00",
            " 1887",
        ] {
            assert!(PartialDate::parse(s).is_err(), "'{s}' should be rejected");
        }
        assert!(PartialDate::new(1887, None, Some(3)).is_err());
    }

    #[test]
    fn leap_years() {
        assert!(PartialDate::parse("2000-02-29").is_ok());
        assert!(PartialDate::parse("1900-02-29").is_err());
        assert!(PartialDate::parse("2024-02-29").is_ok());
    }

    #[test]
    fn earliest_and_latest_day() {
        let d = |s: &str| PartialDate::parse(s).unwrap();
        assert_eq!(d("1970-01-01").earliest_day(), 0);
        assert_eq!(d("1970-01-02").earliest_day(), 1);
        assert_eq!(d("1969-12-31").earliest_day(), -1);
        assert_eq!(d("1887").earliest_day(), d("1887-01-01").earliest_day());
        assert_eq!(d("1887-04").earliest_day(), d("1887-04-01").earliest_day());
        assert_eq!(d("1887").latest_day(), d("1887-12-31").earliest_day());
        assert_eq!(d("1887-04").latest_day(), d("1887-04-30").earliest_day());
        assert_eq!(d("2024-02").latest_day(), d("2024-02-29").earliest_day());
        assert_eq!(d("1887-04-12").latest_day(), d("1887-04-12").earliest_day());
    }

    #[test]
    fn civil_round_trip() {
        for z in (-800_000..800_000).step_by(997) {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn ordering_uses_earliest_day_then_precision() {
        let mut v: Vec<PartialDate> = ["1887-04-12", "1887", "1886-12-31", "1887-04", "1887-01-01", "1887-01"].iter().map(|s| PartialDate::parse(s).unwrap()).collect();
        v.sort();
        let s: Vec<String> = v.iter().map(|d| d.to_string()).collect();
        assert_eq!(s, ["1886-12-31", "1887", "1887-01", "1887-01-01", "1887-04", "1887-04-12"]);
    }

    #[test]
    fn qualifier_does_not_move_sort_position() {
        let a = FuzzyDate::parse("1887", Qualifier::Before).unwrap();
        let b = FuzzyDate::parse("1887", Qualifier::After).unwrap();
        assert_eq!(a.earliest_day(), b.earliest_day());
        assert_eq!(a.to_string(), "before 1887");
        assert_eq!(FuzzyDate::exact("1887-04").unwrap().to_string(), "1887-04");
        assert!(Qualifier::parse("between").is_err());
    }
}
