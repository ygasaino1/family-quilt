//! Store -> GEDCOM 5.5.1 or 7.0 (UTF-8), including `PEDI`.
//!
//! Families come from `derive_families`, so a child with a birth family and an adoptive family
//! is written as two FAM records with one FAMC each. What GEDCOM cannot say is carried in
//! private tags and notes that our importer reads back, so import -> export -> import is stable:
//!   * `step`, `guardian`, `unknown`: 5.5.1 gets `NOTE family-quilt:kind=...`; 7.0 gets `PEDI OTHER` + `PHRASE`
//!   * a column whose parents have different kinds for the child: `NOTE family-quilt:kinds=I3:birth,I7:step`
//!   * partnership state `unmarried`/`ended` (and dates/place of non-marriages): `_FQSTATE`
//!   * a third parent: `_FQPARENT`
//!
//! Pending people and links are never exported unless asked for. Notes on parent-child links are not exported.

use crate::date::{format_fuzzy, format_partial};
use family_families::derive_families;
use family_store::{AccessContext, GraphFilter, LinkKind, NameType, PartnershipState, Person, PersonName, Sex, Store, StoreError};
use std::collections::HashMap;
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GedcomVersion {
    V551,
    V70,
}

fn esc(v: &str) -> String {
    v.replace('@', "@@")
}

/// One logical value, split into CONT lines at newlines.
fn put(out: &mut String, level: usize, tag: &str, value: &str) {
    let mut parts = value.split('\n');
    let first = parts.next().unwrap_or("");
    let _ = if first.is_empty() { writeln!(out, "{level} {tag}") } else { writeln!(out, "{level} {tag} {}", esc(first)) };
    for p in parts {
        let _ = if p.is_empty() { writeln!(out, "{} CONT", level + 1) } else { writeln!(out, "{} CONT {}", level + 1, esc(p)) };
    }
}

fn name_line(n: &PersonName) -> String {
    let clean = |s: &str| s.replace('/', " ");
    match (&n.given, &n.family) {
        (None, None) => clean(&n.full_name),
        (g, f) => format!("{} /{}/", g.as_deref().map(clean).unwrap_or_default(), f.as_deref().map(clean).unwrap_or_default()).trim_start().to_string(),
    }
}

fn kind_word(k: LinkKind) -> &'static str {
    k.as_str()
}

pub fn export_gedcom(store: &Store, ctx: &AccessContext, version: GedcomVersion, include_pending: bool) -> Result<String, StoreError> {
    let people = store.all_people(ctx, include_pending)?;
    let rels = store.all_relationships(ctx, include_pending)?;
    let graph = store.load_graph(ctx, GraphFilter { include_pending })?;
    debug_assert_eq!(people.len(), graph.people.len());
    let derived = derive_families(&graph);
    let rel_by_id: HashMap<&str, &family_store::Relationship> = rels.iter().map(|r| (r.id.as_str(), r)).collect();
    let ix = |i: u32| format!("I{}", i + 1);

    // per person: FAMC / FAMS entries
    let mut famc: Vec<Vec<(usize, usize)>> = vec![Vec::new(); people.len()]; // (family index, child cell index)
    let mut fams: Vec<Vec<usize>> = vec![Vec::new(); people.len()];
    for (fi, f) in derived.families.iter().enumerate() {
        for p in &f.parents {
            fams[*p as usize].push(fi);
        }
        for (ci, c) in f.children.iter().enumerate() {
            famc[c.child as usize].push((fi, ci));
        }
    }

    let mut out = String::new();
    let v = if version == GedcomVersion::V551 { "5.5.1" } else { "7.0" };
    out.push_str("0 HEAD\n1 SOUR family-quilt\n2 NAME family-quilt\n");
    let _ = writeln!(out, "2 VERS {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "1 GEDC\n2 VERS {v}");
    if version == GedcomVersion::V551 {
        out.push_str("2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n");
    }

    for (i, p) in people.iter().enumerate() {
        write_indi(&mut out, version, p, &ix(i as u32), &famc[i], &fams[i], &derived, &ix);
    }

    for (fi, f) in derived.families.iter().enumerate() {
        let _ = writeln!(out, "0 @F{}@ FAM", fi + 1);
        // HUSB / WIFE slots: males first, then females, then any remaining parents in order
        let sex = |i: u32| people[i as usize].sex;
        let mut rest: Vec<u32> = f.parents.clone();
        let take = |rest: &mut Vec<u32>, want: Sex| rest.iter().position(|p| sex(*p) == want).map(|i| rest.remove(i));
        let husb = take(&mut rest, Sex::Male).or_else(|| if rest.is_empty() { None } else { Some(rest.remove(0)) });
        let wife = take(&mut rest, Sex::Female).or_else(|| if rest.is_empty() { None } else { Some(rest.remove(0)) });
        if let Some(h) = husb {
            let _ = writeln!(out, "1 HUSB @{}@", ix(h));
        }
        if let Some(w) = wife {
            let _ = writeln!(out, "1 WIFE @{}@", ix(w));
        }
        for extra in rest {
            let _ = writeln!(out, "1 _FQPARENT @{}@", ix(extra));
        }
        for c in &f.children {
            let _ = writeln!(out, "1 CHIL @{}@", ix(c.child));
        }
        if let Some(pt) = &f.partnership {
            let rel = rel_by_id.get(pt.link_id.as_str());
            let place = rel.and_then(|r| r.place.as_ref()).map(|p| p.name.clone());
            let date = |d: &Option<String>| d.as_deref().and_then(|s| family_store::PartialDate::parse(s).ok()).map(|d| format_partial(&d));
            match pt.state {
                PartnershipState::Married | PartnershipState::Divorced => {
                    out.push_str("1 MARR\n");
                    if let Some(d) = date(&pt.start_date) {
                        let _ = writeln!(out, "2 DATE {d}");
                    }
                    if let Some(pl) = &place {
                        put(&mut out, 2, "PLAC", pl);
                    }
                    if pt.state == PartnershipState::Divorced {
                        out.push_str("1 DIV\n");
                        if let Some(d) = date(&pt.end_date) {
                            let _ = writeln!(out, "2 DATE {d}");
                        }
                    }
                }
                st => {
                    if st != PartnershipState::Unknown || pt.start_date.is_some() || pt.end_date.is_some() || place.is_some() {
                        let _ = writeln!(out, "1 _FQSTATE {}", st.as_str());
                        if let Some(d) = date(&pt.start_date) {
                            let _ = writeln!(out, "2 DATE {d}");
                        }
                        if let Some(d) = date(&pt.end_date) {
                            let _ = writeln!(out, "2 _FQEND {d}");
                        }
                        if let Some(pl) = &place {
                            put(&mut out, 2, "PLAC", pl);
                        }
                    }
                }
            }
            if let Some(n) = rel.and_then(|r| r.note.as_deref()) {
                put(&mut out, 1, "NOTE", n);
            }
        }
    }
    out.push_str("0 TRLR\n");
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn write_indi(out: &mut String, version: GedcomVersion, p: &Person, xref: &str, famc: &[(usize, usize)], fams: &[usize], derived: &family_families::Derived, ix: &dyn Fn(u32) -> String) {
    let _ = writeln!(out, "0 @{xref}@ INDI");
    for n in &p.names {
        put(out, 1, "NAME", &name_line(n));
        if let Some(g) = &n.given {
            put(out, 2, "GIVN", g);
        }
        if let Some(f) = &n.family {
            put(out, 2, "SURN", f);
        }
        let ty = match n.name_type {
            NameType::Birth if n.is_primary => None,
            NameType::Birth => Some("birth"),
            NameType::Married => Some("married"),
            NameType::Alias => Some("aka"),
            NameType::Religious => Some("religious"),
            NameType::Other => Some("other"),
        };
        if let Some(t) = ty {
            let _ = writeln!(out, "2 TYPE {t}");
        }
    }
    match p.sex {
        Sex::Male => out.push_str("1 SEX M\n"),
        Sex::Female => out.push_str("1 SEX F\n"),
        Sex::Other => out.push_str("1 SEX X\n"),
        Sex::Unknown => {}
    }
    if p.birth.is_some() || p.birth_place.is_some() {
        out.push_str("1 BIRT\n");
        if let Some(d) = &p.birth {
            let _ = writeln!(out, "2 DATE {}", format_fuzzy(d));
        }
        if let Some(pl) = &p.birth_place {
            put(out, 2, "PLAC", &pl.name);
        }
    }
    if p.is_deceased() {
        if p.death.is_none() && p.death_place.is_none() {
            out.push_str("1 DEAT Y\n");
        } else {
            out.push_str("1 DEAT\n");
            if let Some(d) = &p.death {
                let _ = writeln!(out, "2 DATE {}", format_fuzzy(d));
            }
            if let Some(pl) = &p.death_place {
                put(out, 2, "PLAC", &pl.name);
            }
        }
    }
    if let Some(n) = &p.note {
        put(out, 1, "NOTE", n);
    }
    for &(fi, ci) in famc {
        let cell = &derived.families[fi].children[ci];
        let _ = writeln!(out, "1 FAMC @F{}@", fi + 1);
        let kinds: Vec<LinkKind> = cell.links.iter().map(|l| l.kind).collect();
        let uniform = kinds.windows(2).all(|w| w[0] == w[1]);
        let main = kinds.iter().copied().find(|k| *k != LinkKind::Birth).unwrap_or(LinkKind::Birth);
        match (version, main) {
            (_, LinkKind::Birth) => {}
            (_, LinkKind::Adopted | LinkKind::Foster | LinkKind::Sealing) => {
                let _ = writeln!(out, "2 PEDI {}", kind_word(main));
            }
            (GedcomVersion::V70, k) => {
                let _ = writeln!(out, "2 PEDI OTHER\n3 PHRASE {}", kind_word(k));
            }
            (GedcomVersion::V551, k) => {
                if uniform {
                    let _ = writeln!(out, "2 NOTE family-quilt:kind={}", kind_word(k));
                }
            }
        }
        if !uniform {
            let list: Vec<String> = cell.links.iter().map(|l| format!("{}:{}", ix(l.parent), kind_word(l.kind))).collect();
            let _ = writeln!(out, "2 NOTE family-quilt:kinds={}", list.join(","));
        }
    }
    for &fi in fams {
        let _ = writeln!(out, "1 FAMS @F{}@", fi + 1);
    }
}
