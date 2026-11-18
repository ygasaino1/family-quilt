//! Relationship names. "A is the <label> of B".
//!
//! A [`RelName`] carries a stable `key` plus numbers, so the front end can translate it
//! (section 20); `text` is the English rendering used by the agent API and as a fallback.

use family_store::{LinkKind, Sex};
use serde::Serialize;

use crate::{Dir, Step};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelName {
    /// Stable key, e.g. `cousin`, `half_sibling`, `adoptive_sibling`, `child_in_law`.
    pub key: String,
    /// Cousin degree, or the number of "great"s.
    pub n: u32,
    /// Times removed (cousins only).
    pub removed: u32,
    pub half: bool,
    /// The sex the gendered `text` was written for.
    pub sex: Sex,
    pub text: String,
}

fn ord(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// Neutral noun -> (male, female). The longest matching suffix wins.
const GENDERED: &[(&str, &str, &str)] = &[
    ("parent-in-law", "father-in-law", "mother-in-law"),
    ("child-in-law", "son-in-law", "daughter-in-law"),
    ("sibling-in-law", "brother-in-law", "sister-in-law"),
    ("stepparent", "stepfather", "stepmother"),
    ("stepchild", "stepson", "stepdaughter"),
    ("step-sibling", "stepbrother", "stepsister"),
    ("grandparent", "grandfather", "grandmother"),
    ("grandchild", "grandson", "granddaughter"),
    ("aunt/uncle", "uncle", "aunt"),
    ("niece/nephew", "nephew", "niece"),
    ("parent", "father", "mother"),
    ("child", "son", "daughter"),
    ("sibling", "brother", "sister"),
];

pub(crate) fn gender(label: &str, sex: Sex) -> String {
    let pick = |m: &str, f: &str| if sex == Sex::Male { m.to_string() } else { f.to_string() };
    if sex == Sex::Male || sex == Sex::Female {
        for (neutral, m, f) in GENDERED {
            if let Some(prefix) = label.strip_suffix(neutral) {
                return format!("{prefix}{}", pick(m, f));
            }
        }
    }
    label.to_string()
}

fn greats(n: u32, noun: &str) -> (String, String) {
    // n = generations beyond the first "grand" level: grandparent n=0, great n=1, 2nd great n=2
    match n {
        0 => (format!("grand{noun}"), format!("grand_{noun}")),
        1 => (format!("great-grand{noun}"), format!("great_grand_{noun}")),
        _ => (format!("{} great-grand{noun}", ord(n)), format!("nth_great_grand_{noun}")),
    }
}

fn great_aunt(n: u32, noun: &str, key: &str) -> (String, String) {
    match n {
        0 => (noun.to_string(), key.to_string()),
        1 => (format!("great-{noun}"), format!("great_{key}")),
        _ => (format!("{} great-{noun}", ord(n)), format!("nth_great_{key}")),
    }
}

/// Blood relationship. `da` / `db` = generations from A / B up to the common ancestor.
pub fn blood_label(da: u32, db: u32, half: bool, sex: Sex) -> RelName {
    let mk = |neutral: String, key: String, n: u32, removed: u32| {
        let neutral = if half { format!("half-{neutral}") } else { neutral };
        let key = if half { format!("half_{key}") } else { key };
        RelName { key, n, removed, half, sex, text: gender(&neutral, sex) }
    };
    match (da, db) {
        (0, 0) => RelName { key: "same_person".into(), n: 0, removed: 0, half: false, sex, text: "same person".into() },
        (0, 1) => mk("parent".into(), "parent".into(), 0, 0),
        (0, _) => {
            let (t, k) = greats(db - 2, "parent");
            mk(t, k, db - 2, 0)
        }
        (1, 0) => mk("child".into(), "child".into(), 0, 0),
        (_, 0) => {
            let (t, k) = greats(da - 2, "child");
            mk(t, k, da - 2, 0)
        }
        (1, 1) => mk("sibling".into(), "sibling".into(), 0, 0),
        (1, _) => {
            let (t, k) = great_aunt(db - 2, "aunt/uncle", "aunt_uncle");
            mk(t, k, db - 2, 0)
        }
        (_, 1) => {
            let (t, k) = great_aunt(da - 2, "niece/nephew", "niece_nephew");
            mk(t, k, da - 2, 0)
        }
        _ => {
            let k = da.min(db) - 1;
            let removed = da.abs_diff(db);
            let mut t = format!("{} cousin", ord(k));
            match removed {
                0 => {}
                1 => t.push_str(" once removed"),
                2 => t.push_str(" twice removed"),
                r => t.push_str(&format!(" {r} times removed")),
            }
            if half {
                t = format!("half {t}");
            }
            RelName { key: if half { "half_cousin" } else { "cousin" }.into(), n: k, removed, half, sex, text: t }
        }
    }
}

fn name(key: &str, neutral: &str, sex: Sex) -> RelName {
    RelName { key: key.to_string(), n: 0, removed: 0, half: false, sex, text: gender(neutral, sex) }
}

/// Name for a route that is not a blood relation. Returns `None` when no named pattern fits
/// (the caller then shows the readable chain).
pub fn route_label(steps: &[Step], sex: Sex) -> Option<RelName> {
    use LinkKind::*;
    let dirs: Vec<Dir> = steps.iter().map(|s| s.dir).collect();
    let kind = |i: usize| steps[i].kind.unwrap_or(Birth);

    // One parent-child step with a recorded kind. Read from A: `Up` means B is A's parent,
    // so A is B's child.
    if let [Dir::Up] = dirs[..] {
        return Some(match kind(0) {
            Birth => name("child", "child", sex),
            Adopted => name("adopted_child", "adopted child", sex),
            Foster => name("foster_child", "foster child", sex),
            Step => name("stepchild", "stepchild", sex),
            Guardian => name("ward", "ward", sex),
            Sealing => name("sealed_child", "sealed child", sex),
            Unknown => name("child_unrecorded", "child (nature not recorded)", sex),
        });
    }
    if let [Dir::Down] = dirs[..] {
        return Some(match kind(0) {
            Birth => name("parent", "parent", sex),
            Adopted => name("adoptive_parent", "adoptive parent", sex),
            Foster => name("foster_parent", "foster parent", sex),
            Step => name("stepparent", "stepparent", sex),
            Guardian => name("guardian", "guardian", sex),
            Sealing => name("sealed_parent", "sealed parent", sex),
            Unknown => name("parent_unrecorded", "parent (nature not recorded)", sex),
        });
    }
    // Siblings through a shared parent where at least one link is not birth.
    if let [Dir::Up, Dir::Down] = dirs[..] {
        let (k1, k2) = (kind(0), kind(1));
        let prefix = match (k1, k2) {
            (Birth, Birth) => return Some(name("sibling", "sibling", sex)),
            (Adopted, Adopted | Birth) | (Birth, Adopted) => ("adoptive_sibling", "adoptive sibling"),
            (Foster, Foster | Birth) | (Birth, Foster) => ("foster_sibling", "foster sibling"),
            (Step, Step | Birth) | (Birth, Step) => ("step_sibling", "step-sibling"),
            _ => return None,
        };
        return Some(name(prefix.0, prefix.1, sex));
    }
    // Through a partner: patterns read from A towards B.
    let base = match &dirs[..] {
        [Dir::Partner] => return Some(RelName { key: "partner".into(), n: 0, removed: 0, half: false, sex, text: "spouse/partner".into() }),
        [Dir::Partner, Dir::Up] => ("child_in_law", "child-in-law"),
        [Dir::Down, Dir::Partner] => ("parent_in_law", "parent-in-law"),
        [Dir::Up, Dir::Partner] => ("stepchild", "stepchild"),
        [Dir::Partner, Dir::Down] => ("stepparent", "stepparent"),
        [Dir::Partner, Dir::Up, Dir::Down] | [Dir::Up, Dir::Down, Dir::Partner] => ("sibling_in_law", "sibling-in-law"),
        [Dir::Up, Dir::Partner, Dir::Down] => ("step_sibling", "step-sibling"),
        [Dir::Partner, Dir::Up, Dir::Down, Dir::Partner] => ("co_sibling_in_law", "co-sibling-in-law"),
        _ => return None,
    };
    Some(name(base.0, base.1, sex))
}

/// Readable chain: "A → child of → B → partner of → C".
pub fn describe_chain(names: &[String], steps: &[Step]) -> String {
    let mut out = names.first().cloned().unwrap_or_default();
    for (i, s) in steps.iter().enumerate() {
        let word = match (s.dir, s.kind) {
            (Dir::Up, Some(LinkKind::Adopted)) => "adopted child of",
            (Dir::Up, Some(LinkKind::Foster)) => "foster child of",
            (Dir::Up, Some(LinkKind::Step)) => "stepchild of",
            (Dir::Up, Some(LinkKind::Guardian)) => "ward of",
            (Dir::Up, Some(LinkKind::Sealing)) => "sealed child of",
            (Dir::Up, Some(LinkKind::Unknown)) => "child of (nature not recorded)",
            (Dir::Up, _) => "child of",
            (Dir::Down, Some(LinkKind::Adopted)) => "adoptive parent of",
            (Dir::Down, Some(LinkKind::Foster)) => "foster parent of",
            (Dir::Down, Some(LinkKind::Step)) => "stepparent of",
            (Dir::Down, Some(LinkKind::Guardian)) => "guardian of",
            (Dir::Down, Some(LinkKind::Sealing)) => "sealed parent of",
            (Dir::Down, Some(LinkKind::Unknown)) => "parent of (nature not recorded)",
            (Dir::Down, _) => "parent of",
            (Dir::Partner, _) if s.ended => "former partner of",
            (Dir::Partner, _) => "partner of",
        };
        out.push_str(&format!(" → {word} → {}", names.get(i + 1).map(String::as_str).unwrap_or("?")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blood_names() {
        let t = |da, db, half, sex| blood_label(da, db, half, sex).text;
        assert_eq!(t(1, 1, false, Sex::Female), "sister");
        assert_eq!(t(1, 1, true, Sex::Male), "half-brother");
        assert_eq!(t(0, 1, false, Sex::Unknown), "parent");
        assert_eq!(t(0, 2, false, Sex::Male), "grandfather");
        assert_eq!(t(0, 3, false, Sex::Female), "great-grandmother");
        assert_eq!(t(0, 4, false, Sex::Male), "2nd great-grandfather");
        assert_eq!(t(2, 0, false, Sex::Female), "granddaughter");
        assert_eq!(t(1, 2, false, Sex::Male), "uncle");
        assert_eq!(t(1, 3, false, Sex::Female), "great-aunt");
        assert_eq!(t(2, 1, false, Sex::Female), "niece");
        assert_eq!(t(2, 2, false, Sex::Unknown), "1st cousin");
        assert_eq!(t(2, 3, false, Sex::Unknown), "1st cousin once removed");
        assert_eq!(t(3, 5, false, Sex::Unknown), "2nd cousin twice removed");
        assert_eq!(t(2, 2, true, Sex::Unknown), "half 1st cousin");
        assert_eq!(ord(11), "11th");
        assert_eq!(ord(22), "22nd");
    }
}
