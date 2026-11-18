//! GEDCOM -> import plan -> store.
//!
//! What is read: INDI (NAME with GIVN/SURN/TYPE, SEX, BIRT/DEAT with DATE and PLAC, NOTE, FAMC with
//! PEDI, ADOP/FAMC, FAMS), FAM (HUSB, WIFE, CHIL, MARR, DIV and separation markers). A child's
//! parents come from FAM.CHIL **and** INDI.FAMC; a child with several FAMC gets several parent sets.
//! The original GeneaQuilts reader kept only the last FAMC and ignored PEDI, MARR and DIV.

use crate::date::{parse_date, ParsedDate};
use crate::parse::{pointer, read_records, Limits, Node};
use crate::GedError;
use family_store::{AccessContext, LinkKind, NameType, NewName, NewPerson, NewRelationship, Origin, PartialDate, PartnershipState, RelType, Sex, Store, StoreError};
use std::collections::{HashMap, HashSet};
use std::io::BufRead;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportWarning {
    pub line: usize,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct PlannedPerson {
    pub xref: String,
    pub line: usize,
    pub person: NewPerson,
}

#[derive(Debug, Clone)]
pub struct PlannedLink {
    pub rel_type: RelType,
    /// Indexes into `ImportPlan::people`.
    pub a: usize,
    pub b: usize,
    pub kind: Option<LinkKind>,
    pub subtype: Option<PartnershipState>,
    pub start_date: Option<PartialDate>,
    pub end_date: Option<PartialDate>,
    pub place: Option<String>,
    pub note: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ImportPlan {
    pub people: Vec<PlannedPerson>,
    pub links: Vec<PlannedLink>,
    pub warnings: Vec<ImportWarning>,
    pub gedcom_version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApplyReport {
    /// Store ids, in the order of `ImportPlan::people`.
    pub person_ids: Vec<String>,
    pub links_created: usize,
}

struct Raw {
    indis: Vec<Node>,
    fams: Vec<Node>,
    head: Option<Node>,
    saw_trlr: bool,
}

/// Read a GEDCOM stream into a plan. Fails on syntax problems (with the line number); softer
/// problems become warnings in the plan.
pub fn import_gedcom(r: impl BufRead, limits: &Limits) -> Result<ImportPlan, GedError> {
    let mut raw = Raw { indis: Vec::new(), fams: Vec::new(), head: None, saw_trlr: false };
    let mut first = true;
    read_records(r, limits, |n| {
        if first {
            first = false;
            if n.tag != "HEAD" {
                return Err(GedError::NotGedcom);
            }
        }
        match n.tag.as_str() {
            "HEAD" => raw.head = Some(n),
            "INDI" => raw.indis.push(n),
            "FAM" => raw.fams.push(n),
            "TRLR" => raw.saw_trlr = true,
            _ => {}
        }
        Ok(())
    })?;
    if first {
        return Err(GedError::NotGedcom);
    }
    Ok(build_plan(raw))
}

#[derive(Default)]
struct FamcInfo {
    fam: String,
    pedi: Option<String>,
    phrase: Option<String>,
    kind_note: Option<LinkKind>,
    kinds_note: Vec<(String, LinkKind)>,
}

fn script_of(s: &str) -> Option<String> {
    let arabic = s.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c) || ('\u{0750}'..='\u{077F}').contains(&c) || ('\u{FB50}'..='\u{FEFF}').contains(&c));
    if arabic {
        Some("Arab".into())
    } else if s.chars().any(char::is_alphabetic) && s.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c)) {
        Some("Latn".into())
    } else {
        None
    }
}

fn parse_kind_word(w: &str) -> Option<LinkKind> {
    LinkKind::parse(w.trim()).ok()
}

fn build_plan(raw: Raw) -> ImportPlan {
    let mut plan = ImportPlan::default();
    let warn = |plan: &mut ImportPlan, line, code, message: String| plan.warnings.push(ImportWarning { line, code, message });

    if let Some(h) = &raw.head {
        plan.gedcom_version = h.child("GEDC").and_then(|g| g.text("VERS")).map(str::to_string);
        if let Some(cs) = h.text("CHAR") {
            if !matches!(cs.to_uppercase().as_str(), "UTF-8" | "UTF8" | "UNICODE" | "ASCII" | "ANSI") {
                warn(&mut plan, h.line, "charset", format!("the file says its character set is {cs}; it was read as UTF-8"));
            }
        }
    }
    if !raw.saw_trlr {
        warn(&mut plan, 0, "missing_trlr", "the file has no 0 TRLR line at the end (it may be cut off)".into());
    }

    // ---------- people ----------
    let mut xref_to_idx: HashMap<String, usize> = HashMap::new();
    let mut famcs: Vec<Vec<FamcInfo>> = Vec::new();
    let mut adop: Vec<HashSet<String>> = Vec::new();
    for n in &raw.indis {
        let Some(x) = &n.xref else {
            warn(&mut plan, n.line, "no_xref", "an INDI record has no id and was skipped".into());
            continue;
        };
        if xref_to_idx.contains_key(x) {
            warn(&mut plan, n.line, "duplicate_id", format!("person id {x} appears twice; the second one was skipped"));
            continue;
        }
        let mut notes: Vec<String> = Vec::new();
        let mut p = NewPerson::default();

        for (i, nm) in n.all("NAME").enumerate() {
            let value = nm.value.trim();
            let (mut given, mut family, mut suffix) = (None::<String>, None::<String>, None::<String>);
            if let Some((before, rest)) = value.split_once('/') {
                let (sur, after) = rest.split_once('/').unwrap_or((rest, ""));
                given = Some(before.trim().to_string()).filter(|s| !s.is_empty());
                family = Some(sur.trim().to_string()).filter(|s| !s.is_empty());
                suffix = Some(after.trim().to_string()).filter(|s| !s.is_empty());
            }
            let has_parts = given.is_some() || family.is_some() || nm.text("GIVN").is_some() || nm.text("SURN").is_some();
            if let Some(g) = nm.text("GIVN") {
                given = Some(g.to_string());
            }
            if let Some(s) = nm.text("SURN") {
                family = Some(s.to_string());
            }
            let composed = [given.as_deref(), family.as_deref(), suffix.as_deref()].iter().flatten().copied().collect::<Vec<_>>().join(" ");
            let full = if composed.is_empty() { value.replace('/', " ").split_whitespace().collect::<Vec<_>>().join(" ") } else { composed };
            if full.is_empty() {
                warn(&mut plan, nm.line, "empty_name", "an empty NAME was skipped".into());
                continue;
            }
            let name_type = match nm.text("TYPE").map(|t| t.to_lowercase()).as_deref() {
                None | Some("birth") => NameType::Birth,
                Some("married") => NameType::Married,
                Some("aka") | Some("alias") | Some("nickname") => NameType::Alias,
                Some("religious") => NameType::Religious,
                Some(_) => NameType::Other,
            };
            let script = script_of(&full);
            p.names.push(NewName { name_type: Some(name_type), given: if has_parts { given } else { None }, family: if has_parts { family } else { None }, full_name: Some(full), script });
            let _ = i;
        }
        if p.names.is_empty() {
            warn(&mut plan, n.line, "no_name", format!("person {x} has no name; it was imported as \"(unnamed)\""));
            p.names.push(NewName::full("(unnamed)"));
        }

        p.sex = match n.text("SEX").map(|s| s.to_uppercase()).as_deref() {
            None | Some("U") => Sex::Unknown,
            Some("M") => Sex::Male,
            Some("F") => Sex::Female,
            Some("X") => Sex::Other,
            Some(other) => {
                warn(&mut plan, n.line, "bad_sex", format!("unknown sex value '{other}' for {x}; set to unknown"));
                Sex::Unknown
            }
        };

        for (label, tag) in [("Birth", "BIRT"), ("Death", "DEAT")] {
            let Some(ev) = n.child(tag) else { continue };
            let place = ev.text("PLAC").map(str::to_string);
            let date = match ev.text("DATE").map(parse_date) {
                Some(ParsedDate::Date { date, note }) => {
                    if let Some(nt) = note {
                        notes.push(format!("{label} date: {nt}"));
                    }
                    Some(date)
                }
                Some(ParsedDate::Unsupported(orig)) => {
                    warn(&mut plan, ev.line, "unsupported_date", format!("{label} date '{orig}' of {x} cannot be stored; the original text was kept in the note"));
                    notes.push(format!("{label} date (as in the file): {orig}"));
                    None
                }
                _ => None,
            };
            if tag == "BIRT" {
                p.birth = date;
                p.birth_place = place;
            } else {
                p.death = date;
                p.death_place = place;
                p.is_living = Some(false);
            }
        }
        if p.is_living.is_none() && (n.child("BURI").is_some() || n.child("CREM").is_some()) {
            p.is_living = Some(false);
        }
        for nt in n.all("NOTE") {
            if pointer(&nt.value).is_none() && !nt.value.trim().is_empty() {
                notes.insert(0, nt.value.trim().to_string());
            }
        }
        if !notes.is_empty() {
            p.note = Some(notes.join("\n"));
        }

        let mut fc = Vec::new();
        for f in n.all("FAMC") {
            let Some(fam) = pointer(&f.value) else { continue };
            let mut info = FamcInfo { fam: fam.to_string(), ..Default::default() };
            if let Some(pd) = f.child("PEDI") {
                info.pedi = Some(pd.value.trim().to_lowercase());
                info.phrase = pd.text("PHRASE").map(|s| s.to_lowercase());
            }
            for nt in f.all("NOTE") {
                if let Some(v) = nt.value.trim().strip_prefix("family-quilt:kind=") {
                    info.kind_note = parse_kind_word(v);
                } else if let Some(v) = nt.value.trim().strip_prefix("family-quilt:kinds=") {
                    info.kinds_note = v.split(',').filter_map(|pair| pair.split_once(':')).filter_map(|(x, k)| parse_kind_word(k).map(|k| (x.trim().to_string(), k))).collect();
                }
            }
            fc.push(info);
        }
        let mut ad = HashSet::new();
        for a in n.all("ADOP") {
            for f in a.all("FAMC") {
                if let Some(fam) = pointer(&f.value) {
                    ad.insert(fam.to_string());
                }
            }
        }
        xref_to_idx.insert(x.clone(), plan.people.len());
        plan.people.push(PlannedPerson { xref: x.clone(), line: n.line, person: p });
        famcs.push(fc);
        adop.push(ad);
    }

    // ---------- families ----------
    // family xref -> children found on the INDI side
    let mut kids_by_fam: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, fc) in famcs.iter().enumerate() {
        for f in fc {
            kids_by_fam.entry(f.fam.as_str()).or_default().push(i);
        }
    }

    let mut parents_of: Vec<Vec<usize>> = vec![Vec::new(); plan.people.len()];
    let mut link_pairs: HashSet<(usize, usize)> = HashSet::new();
    let mut partner_pairs: HashSet<(usize, usize)> = HashSet::new();
    let mut seen_fams: HashSet<&str> = HashSet::new();

    for f in &raw.fams {
        let Some(fx) = f.xref.as_deref() else {
            warn(&mut plan, f.line, "no_xref", "a FAM record has no id and was skipped".into());
            continue;
        };
        if !seen_fams.insert(fx) {
            warn(&mut plan, f.line, "duplicate_id", format!("family id {fx} appears twice; the second one was skipped"));
            continue;
        }
        let resolve = |plan: &mut ImportPlan, ptr: &str, line: usize, role: &str| -> Option<usize> {
            match pointer(ptr).and_then(|x| xref_to_idx.get(x)) {
                Some(i) => Some(*i),
                None => {
                    warn(plan, line, "unknown_reference", format!("family {fx}: {role} {ptr} does not exist; ignored"));
                    None
                }
            }
        };
        let mut parents: Vec<usize> = Vec::new();
        for (tag, role) in [("HUSB", "husband"), ("WIFE", "wife"), ("_FQPARENT", "parent")] {
            for c in f.all(tag) {
                if let Some(i) = resolve(&mut plan, &c.value, c.line, role) {
                    if !parents.contains(&i) {
                        parents.push(i);
                    }
                }
            }
        }
        let mut kids: Vec<usize> = Vec::new();
        for c in f.all("CHIL") {
            if let Some(i) = resolve(&mut plan, &c.value, c.line, "child") {
                if !kids.contains(&i) {
                    kids.push(i);
                }
            }
        }
        for i in kids_by_fam.get(fx).cloned().unwrap_or_default() {
            if !kids.contains(&i) {
                kids.push(i);
            }
        }

        // partnership (two parents)
        if parents.len() == 2 {
            let (a, b) = (parents[0].min(parents[1]), parents[0].max(parents[1]));
            if !partner_pairs.insert((a, b)) {
                warn(&mut plan, f.line, "duplicate_partnership", format!("family {fx}: the same two people are already partners in another family; the extra partnership was not repeated"));
            } else {
                let fq = f.child("_FQSTATE");
                let marr = f.child("MARR");
                let div = f.child("DIV").or_else(|| f.child("DIVF"));
                let state = match fq.map(|n| n.value.trim().to_lowercase()).as_deref() {
                    Some(s) if PartnershipState::parse(s).is_ok() => PartnershipState::parse(s).unwrap(),
                    _ if div.is_some() => PartnershipState::Divorced,
                    _ if ["ANUL", "SEPR", "_SEPR", "_SEPARATED"].iter().any(|t| f.child(t).is_some()) => PartnershipState::Ended,
                    _ if marr.is_some() => PartnershipState::Married,
                    _ => PartnershipState::Unknown,
                };
                let date = |plan: &mut ImportPlan, n: Option<&Node>| -> Option<PartialDate> {
                    let n = n?;
                    match n.text("DATE").map(parse_date) {
                        Some(ParsedDate::Date { date, .. }) => Some(date.date),
                        Some(ParsedDate::Unsupported(o)) => {
                            warn(plan, n.line, "unsupported_date", format!("family {fx}: date '{o}' cannot be stored"));
                            None
                        }
                        _ => None,
                    }
                };
                let (start, end, place) = if let Some(q) = fq.filter(|_| marr.is_none()) {
                    (
                        date(&mut plan, Some(q)),
                        q.child("_FQEND").and_then(|e| match parse_date(&e.value) {
                            ParsedDate::Date { date, .. } => Some(date.date),
                            _ => None,
                        }),
                        q.text("PLAC").map(str::to_string),
                    )
                } else {
                    (date(&mut plan, marr), date(&mut plan, div), marr.and_then(|m| m.text("PLAC")).map(str::to_string))
                };
                let note = f.all("NOTE").find(|n| pointer(&n.value).is_none() && !n.value.trim().is_empty()).map(|n| n.value.trim().to_string());
                plan.links.push(PlannedLink { rel_type: RelType::Partnership, a, b, kind: None, subtype: Some(state), start_date: start, end_date: end, place, note, line: f.line });
            }
        } else if parents.len() > 2 {
            warn(&mut plan, f.line, "many_parents", format!("family {fx} lists {} parents", parents.len()));
        }

        if parents.is_empty() && !kids.is_empty() {
            warn(&mut plan, f.line, "family_without_parents", format!("family {fx} has children but no parents; the children were not linked"));
            continue;
        }

        // parent-child links
        for c in kids {
            let info = famcs[c].iter().find(|i| i.fam == fx);
            let default_kind = match info.and_then(|i| i.pedi.as_deref()) {
                None | Some("birth") => {
                    if adop[c].contains(fx) && info.and_then(|i| i.pedi.as_deref()).is_none() {
                        LinkKind::Adopted
                    } else {
                        LinkKind::Birth
                    }
                }
                Some("adopted") => LinkKind::Adopted,
                Some("foster") => LinkKind::Foster,
                Some("sealing") => LinkKind::Sealing,
                Some("other") => match info.and_then(|i| i.phrase.as_deref()) {
                    Some("step") => LinkKind::Step,
                    Some("unknown") => LinkKind::Unknown,
                    _ => LinkKind::Guardian,
                },
                Some(other) => {
                    let (cl, cx) = (plan.people[c].line, plan.people[c].xref.clone());
                    warn(&mut plan, cl, "unknown_pedi", format!("{cx}: unknown PEDI value '{other}' in family {fx}; the link was imported with kind \"unknown\""));
                    LinkKind::Unknown
                }
            };
            let default_kind = info.and_then(|i| i.kind_note).unwrap_or(default_kind);
            for &p in &parents {
                let kind = info.and_then(|i| i.kinds_note.iter().find(|(x, _)| plan.people.get(p).is_some_and(|pp| pp.xref == *x)).map(|(_, k)| *k)).unwrap_or(default_kind);
                if p == c {
                    warn(&mut plan, f.line, "self_parent", format!("family {fx}: a person is listed as their own parent; link skipped"));
                    continue;
                }
                if !link_pairs.insert((p, c)) {
                    let (px, cx) = (plan.people[p].xref.clone(), plan.people[c].xref.clone());
                    warn(&mut plan, f.line, "duplicate_link", format!("family {fx}: {px} is already a parent of {cx} through another family; the second link was skipped"));
                    continue;
                }
                // would this make c an ancestor of p?
                let mut stack = vec![p];
                let mut seen = HashSet::new();
                let mut cycle = false;
                while let Some(x) = stack.pop() {
                    if x == c {
                        cycle = true;
                        break;
                    }
                    if seen.insert(x) {
                        stack.extend(parents_of[x].iter().copied());
                    }
                }
                if cycle {
                    let (px, cx) = (plan.people[p].xref.clone(), plan.people[c].xref.clone());
                    warn(&mut plan, f.line, "cycle_skipped", format!("family {fx}: {px} as parent of {cx} would make a person their own ancestor; link skipped"));
                    link_pairs.remove(&(p, c));
                    continue;
                }
                parents_of[c].push(p);
                plan.links.push(PlannedLink { rel_type: RelType::ParentChild, a: p, b: c, kind: Some(kind), subtype: None, start_date: None, end_date: None, place: None, note: None, line: f.line });
            }
        }
    }
    plan
}

impl ImportPlan {
    /// Create everything in one all-or-nothing batch. `origin` decides verified (human) or pending (agent proposal).
    pub fn apply(&self, store: &mut Store, ctx: &AccessContext, origin: &Origin) -> Result<ApplyReport, StoreError> {
        store.atomic(|s| {
            let mut ids = Vec::with_capacity(self.people.len());
            for p in &self.people {
                ids.push(s.create_person(ctx, p.person.clone(), origin)?.id);
            }
            for l in &self.links {
                let mut nr = match l.rel_type {
                    RelType::ParentChild => NewRelationship::parent_child(&ids[l.a], &ids[l.b], l.kind.unwrap_or_default()),
                    RelType::Partnership => NewRelationship::partnership(&ids[l.a], &ids[l.b], l.subtype.unwrap_or_default()),
                };
                nr.start_date = l.start_date;
                nr.end_date = l.end_date;
                nr.place = l.place.clone();
                nr.note = l.note.clone();
                s.add_relationship(ctx, nr, origin)?;
            }
            Ok(ApplyReport { person_ids: ids, links_created: self.links.len() })
        })
    }
}
