//! GEDCOM date strings <-> fuzzy dates.

use family_store::{FuzzyDate, PartialDate, Qualifier};

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedDate {
    /// Stored as-is. `note` is set if information was lost (ranges become "about" the start).
    Date {
        date: FuzzyDate,
        note: Option<String>,
    },
    /// Could not be stored (other calendar, phrase, BC...). The original text should be kept in a note.
    Unsupported(String),
    Empty,
}

const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

fn month_num(s: &str) -> Option<u8> {
    MONTHS.iter().position(|m| *m == s).map(|i| i as u8 + 1)
}

/// `[day] [month] year` -> PartialDate.
fn core(tokens: &[&str]) -> Result<PartialDate, String> {
    let (d, m, y) = match tokens {
        [y] => (None, None, *y),
        [m, y] => (None, Some(*m), *y),
        [d, m, y] => (Some(*d), Some(*m), *y),
        _ => return Err("unrecognised date layout".into()),
    };
    let year_digits = y.split('/').next().unwrap_or(y); // dual dating 1699/00 -> 1699
    if !year_digits.bytes().all(|b| b.is_ascii_digit()) || year_digits.is_empty() || year_digits.len() > 4 {
        return Err(format!("year '{y}' is not supported"));
    }
    let year: u16 = year_digits.parse().map_err(|_| "bad year".to_string())?;
    let month = match m {
        Some(m) => Some(month_num(m).ok_or_else(|| format!("unknown month '{m}'"))?),
        None => None,
    };
    let day = match d {
        Some(d) => Some(d.parse::<u8>().map_err(|_| format!("bad day '{d}'"))?),
        None => None,
    };
    PartialDate::new(year, month, day).map_err(|e| e.to_string())
}

pub fn parse_date(raw: &str) -> ParsedDate {
    let up = raw.trim().to_uppercase();
    if up.is_empty() {
        return ParsedDate::Empty;
    }
    if up.starts_with('(') {
        return ParsedDate::Unsupported(raw.trim().to_string());
    }
    let mut toks: Vec<&str> = up.split_whitespace().collect();
    if let Some(first) = toks.first() {
        if first.starts_with("@#") {
            let cal = toks.remove(0);
            if cal != "@#DGREGORIAN@" {
                return ParsedDate::Unsupported(raw.trim().to_string());
            }
        }
    }
    let mut qualifier = Qualifier::Exact;
    let mut note = None;
    let take = |toks: &mut Vec<&str>, n: usize| {
        toks.drain(..n);
    };
    match toks.first().copied() {
        Some("ABT" | "ABOUT" | "CAL" | "EST" | "CIRCA") => {
            qualifier = Qualifier::About;
            take(&mut toks, 1)
        }
        Some("BEF" | "BEFORE" | "TO") => {
            qualifier = Qualifier::Before;
            take(&mut toks, 1)
        }
        Some("AFT" | "AFTER") => {
            qualifier = Qualifier::After;
            take(&mut toks, 1)
        }
        Some("INT") => {
            qualifier = Qualifier::About;
            take(&mut toks, 1);
            note = Some("interpreted date".to_string());
        }
        Some("BET" | "FROM") => {
            qualifier = Qualifier::About;
            let end = toks.iter().position(|t| *t == "AND" || *t == "TO");
            toks.remove(0);
            if let Some(e) = end {
                let e = e - 1;
                toks.truncate(e);
            }
            note = Some(format!("original range: {}", raw.trim()));
        }
        _ => {}
    }
    // drop a trailing "(phrase)"
    if let Some(p) = toks.iter().position(|t| t.starts_with('(')) {
        toks.truncate(p);
    }
    if toks.last().is_some_and(|t| t.starts_with("B.C") || *t == "BC" || *t == "BCE") {
        return ParsedDate::Unsupported(raw.trim().to_string());
    }
    match core(&toks) {
        Ok(date) => ParsedDate::Date { date: FuzzyDate::new(date, qualifier), note },
        Err(_) => ParsedDate::Unsupported(raw.trim().to_string()),
    }
}

/// `1887-04-12` -> `12 APR 1887`.
pub fn format_partial(d: &PartialDate) -> String {
    match (d.month(), d.day()) {
        (None, _) => format!("{:04}", d.year()),
        (Some(m), None) => format!("{} {:04}", MONTHS[m as usize - 1], d.year()),
        (Some(m), Some(day)) => format!("{day} {} {:04}", MONTHS[m as usize - 1], d.year()),
    }
}

pub fn format_fuzzy(d: &FuzzyDate) -> String {
    let core = format_partial(&d.date);
    match d.qualifier {
        Qualifier::Exact => core,
        Qualifier::About => format!("ABT {core}"),
        Qualifier::Before => format!("BEF {core}"),
        Qualifier::After => format!("AFT {core}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> String {
        match parse_date(s) {
            ParsedDate::Date { date, .. } => date.to_string(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn common_forms() {
        assert_eq!(ok("12 APR 1887"), "1887-04-12");
        assert_eq!(ok("apr 1887"), "1887-04");
        assert_eq!(ok("1887"), "1887");
        assert_eq!(ok("ABT 1887"), "about 1887");
        assert_eq!(ok("BEF 3 MAY 1900"), "before 1900-05-03");
        assert_eq!(ok("AFT 1900"), "after 1900");
        assert_eq!(ok("EST 1850"), "about 1850");
        assert_eq!(ok("1699/00"), "1699");
        assert_eq!(ok("@#DGREGORIAN@ 5 JUN 1950"), "1950-06-05");
        assert_eq!(ok("TO 1950"), "before 1950");
    }

    #[test]
    fn ranges_keep_a_note() {
        match parse_date("BET 1880 AND 1890") {
            ParsedDate::Date { date, note } => {
                assert_eq!(date.to_string(), "about 1880");
                assert!(note.unwrap().contains("BET 1880 AND 1890"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(ok("FROM 1 JAN 1900 TO 1 JAN 1910"), "about 1900-01-01");
    }

    #[test]
    fn unsupported_forms_are_reported_not_guessed() {
        for s in ["@#DJULIAN@ 1 JAN 1700", "(unknown)", "44 B.C.", "31 FEB 1900", "XYZ 1900", "1 2 3 4", "12345", "@#DHEBREW@ 5 TSH 5700"] {
            assert!(matches!(parse_date(s), ParsedDate::Unsupported(_)), "{s}");
        }
        assert_eq!(parse_date("  "), ParsedDate::Empty);
    }

    #[test]
    fn format_round_trip() {
        for s in ["12 APR 1887", "APR 1887", "1887", "ABT 1887", "BEF 5 MAY 1900", "AFT 1900"] {
            match parse_date(s) {
                ParsedDate::Date { date, .. } => assert_eq!(format_fuzzy(&date), s),
                other => panic!("{other:?}"),
            }
        }
    }
}
