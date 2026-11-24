//! The review view of a proposal: new records, per-field before/after, conflicts, and warnings
//! computed on the state **as it would be after approval** (section 19).

use crate::fields::{current_name_field, current_person_field, current_rel_field, norm};
use crate::Result;
use family_store::{AccessContext, ChangeRow, GraphFilter, LinkKind, Person, ProposalRow, RelType, Relationship, Sex, Store};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

const CTX: AccessContext = AccessContext::Everything;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Warning {
    /// Stable code, e.g. `parent_too_young`.
    pub code: &'static str,
    pub message: String,
    pub people: Vec<String>,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldView {
    pub field: String,
    pub from: Value,
    pub to: Value,
    pub current: Value,
    /// `current` differs from what the agent saw: never applied silently.
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeView {
    #[serde(flatten)]
    pub change: ChangeRow,
    /// Human-readable name of the record (person name, or "A — B" for a link).
    pub target_label: String,
    pub target_missing: bool,
    /// Record changed since the agent read it (`base_updated_at` differs).
    pub stale: bool,
    /// Removing a person is applied only when ticked explicitly.
    pub needs_explicit_approval: bool,
    pub fields: Vec<FieldView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkView {
    #[serde(flatten)]
    pub rel: Relationship,
    pub a_name: String,
    pub b_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewView {
    pub proposal: ProposalRow,
    /// Other proposals that must be approved first (this one links to, or edits, their pending records).
    pub depends_on: Vec<Dependency>,
    pub new_people: Vec<Person>,
    pub new_relationships: Vec<LinkView>,
    pub changes: Vec<ChangeView>,
    pub warnings: Vec<Warning>,
    /// What the reviewer corrected so far: `[{kind, target_id, field, original, current, at}]`.
    pub edits: Vec<Value>,
}

fn name_of(store: &Store, id: &str) -> String {
    store.get_person(&CTX, id).map(|p| p.display_name).unwrap_or_else(|_| "(unknown person)".into())
}

/// Current-vs-proposed view of one change.
pub fn change_view(store: &Store, c: ChangeRow) -> ChangeView {
    let mut view =
        ChangeView { target_label: String::new(), target_missing: false, stale: false, needs_explicit_approval: c.target_type == "person" && c.op == "remove", fields: vec![], change: c.clone() };
    let get: Option<(String, crate::fields::FieldReader)> = match c.target_type.as_str() {
        "person" => store
            .get_person(&CTX, &c.target_id)
            .ok()
            .map(|p| {
                (
                    format!("{} ({})", p.display_name, ""),
                    Box::new({
                        let p2 = p.clone();
                        move |f: &str| current_person_field(&p2, f)
                    }) as Box<dyn Fn(&str) -> Value>,
                )
            })
            .map(|(l, f)| (l.replace(" ()", ""), f)),
        "person_name" => store
            .find_name(&CTX, &c.target_id)
            .ok()
            .map(|(pid, n)| (format!("{} (name of {})", n.full_name, name_of(store, &pid)), Box::new(move |f: &str| current_name_field(&n, f)) as Box<dyn Fn(&str) -> Value>)),
        _ => store
            .get_relationship(&CTX, &c.target_id)
            .ok()
            .map(|r| (format!("{} — {}", name_of(store, &r.person_a), name_of(store, &r.person_b)), Box::new(move |f: &str| current_rel_field(&r, f)) as Box<dyn Fn(&str) -> Value>)),
    };
    let Some((label, cur)) = get else {
        view.target_missing = true;
        view.target_label = "(record no longer exists)".into();
        return view;
    };
    view.target_label = label;
    view.stale = match (c.target_type.as_str(), &c.base_updated_at) {
        (_, None) => false,
        ("person", Some(b)) => store.get_person(&CTX, &c.target_id).map(|p| &p.updated_at != b).unwrap_or(true),
        ("relationship", Some(b)) => store.get_relationship(&CTX, &c.target_id).map(|r| &r.updated_at != b).unwrap_or(true),
        _ => false,
    };
    if let Some(o) = c.patch.as_object() {
        for (f, ft) in o {
            let from = norm(&ft["from"]);
            let current = norm(&cur(f));
            view.fields.push(FieldView { field: f.clone(), conflict: from != current, from, to: norm(&ft["to"]), current });
        }
    }
    view
}

pub fn review(store: &Store, proposal_id: &str) -> Result<ReviewView> {
    let proposal = store.get_proposal(&CTX, proposal_id)?;
    let recs = store.proposal_records(&CTX, proposal_id)?;
    let mut new_people = vec![];
    for id in &recs.people {
        new_people.push(store.get_person(&CTX, id)?);
    }
    let mut new_relationships = vec![];
    for id in &recs.relationships {
        let rel = store.get_relationship(&CTX, id)?;
        new_relationships.push(LinkView { a_name: name_of(store, &rel.person_a), b_name: name_of(store, &rel.person_b), rel });
    }
    let changes = store.changes_of(&CTX, proposal_id)?.into_iter().map(|c| change_view(store, c)).collect();
    let warnings = proposal_warnings(store, proposal_id)?;
    let depends_on = dependencies(store, proposal_id)?;
    let edits = store.reviewer_edits(proposal_id)?;
    Ok(ReviewView { proposal, depends_on, new_people, new_relationships, changes, warnings, edits })
}

// ───────────────────────────── warnings ─────────────────────────────

const MIN_PARENT_YEARS: f64 = 12.0;
const MAX_MOTHER_YEARS: f64 = 55.0;
const MAX_FATHER_YEARS: f64 = 80.0;

fn is_birthlike(k: Option<LinkKind>) -> bool {
    matches!(k.unwrap_or_default(), LinkKind::Birth | LinkKind::Sealing)
}

/// Blood ancestors of `start` over birth links; `override_kind` replaces the kind of one link.
fn blood_ancestors(parents: &HashMap<String, Vec<(String, String, LinkKind)>>, start: &str, over: Option<(&str, LinkKind)>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut q = VecDeque::from([start.to_string()]);
    while let Some(c) = q.pop_front() {
        for (link_id, parent, kind) in parents.get(&c).into_iter().flatten() {
            let k = match over {
                Some((id, nk)) if id == link_id => nk,
                _ => *kind,
            };
            if k == LinkKind::Birth && seen.insert(parent.clone()) {
                q.push_back(parent.clone());
            }
        }
    }
    seen
}

/// Warnings for what the proposal touches, judged on the resulting state (all changes accepted).
pub fn proposal_warnings(store: &Store, proposal_id: &str) -> Result<Vec<Warning>> {
    let recs = store.proposal_records(&CTX, proposal_id)?;
    let changes = store.changes_of(&CTX, proposal_id)?;
    let mut out: Vec<Warning> = vec![];

    // Overlays from the proposed changes.
    let mut person_over: HashMap<String, HashMap<String, Value>> = HashMap::new();
    let mut kind_over: HashMap<String, LinkKind> = HashMap::new();
    let mut removed: HashSet<String> = HashSet::new();
    let mut removed_people: HashSet<String> = HashSet::new();
    let mut touched: BTreeSet<String> = recs.people.iter().cloned().collect();
    for c in &changes {
        match (c.target_type.as_str(), c.op.as_str()) {
            ("person", "update") => {
                touched.insert(c.target_id.clone());
                if let Some(o) = c.patch.as_object() {
                    for (f, ft) in o {
                        person_over.entry(c.target_id.clone()).or_default().insert(f.clone(), norm(&ft["to"]));
                    }
                }
            }
            ("person", "remove") => {
                removed_people.insert(c.target_id.clone());
            }
            ("person_name", _) => {
                if let Ok((pid, _)) = store.find_name(&CTX, &c.target_id) {
                    touched.insert(pid);
                }
            }
            ("relationship", op) => {
                if let Ok(r) = store.get_relationship(&CTX, &c.target_id) {
                    touched.insert(r.person_a.clone());
                    touched.insert(r.person_b.clone());
                }
                if op == "remove" {
                    removed.insert(c.target_id.clone());
                } else if let Some(k) = c.patch.get("kind").and_then(|f| f["to"].as_str()).and_then(|s| LinkKind::parse(s).ok()) {
                    kind_over.insert(c.target_id.clone(), k);
                }
            }
            _ => {}
        }
    }
    for id in &recs.relationships {
        if let Ok(r) = store.get_relationship(&CTX, id) {
            touched.insert(r.person_a);
            touched.insert(r.person_b);
        }
    }

    // People (with overlay) and links around the touched set.
    let mut cache: HashMap<String, Option<Person>> = HashMap::new();
    let load = |id: &str, cache: &mut HashMap<String, Option<Person>>| -> Option<Person> { cache.entry(id.to_string()).or_insert_with(|| store.get_person(&CTX, id).ok()).clone() };
    let date_of = |p: &Person, which: &str, over: Option<&HashMap<String, Value>>| -> Option<family_store::FuzzyDate> {
        let (df, qf, cur) = if which == "birth" { ("birth_date", "birth_qualifier", p.birth) } else { ("death_date", "death_qualifier", p.death) };
        let Some(o) = over else { return cur };
        if !o.contains_key(df) && !o.contains_key(qf) {
            return cur;
        }
        let d = o.get(df).cloned().unwrap_or_else(|| current_person_field(p, df));
        let q = o.get(qf).cloned().unwrap_or_else(|| current_person_field(p, qf));
        let q = q.as_str().and_then(|s| family_store::Qualifier::parse(s).ok()).unwrap_or_default();
        d.as_str().and_then(|s| family_store::FuzzyDate::parse(s, q).ok())
    };
    let years = |days: i32| days as f64 / 365.25;

    for id in removed_people.iter() {
        touched.remove(id);
    }
    for id in touched.iter() {
        let Some(p) = load(id, &mut cache) else { continue };
        let ov = person_over.get(id);
        if let (Some(b), Some(d)) = (date_of(&p, "birth", ov), date_of(&p, "death", ov)) {
            if d.latest_day() < b.earliest_day() {
                out.push(Warning { code: "death_before_birth", message: format!("{} would have died before being born.", p.display_name), people: vec![id.clone()], params: json!({}) });
            }
        }
    }
    let mut seen_links = HashSet::new();
    for id in touched.iter() {
        for r in store.relationships_of(&CTX, id)? {
            if removed.contains(&r.id) || removed_people.contains(&r.person_a) || removed_people.contains(&r.person_b) || !seen_links.insert(r.id.clone()) || r.rel_type != RelType::ParentChild {
                continue;
            }
            let kind = kind_over.get(&r.id).copied().or(r.kind);
            if !is_birthlike(kind) {
                continue;
            }
            let (Some(parent), Some(child)) = (load(&r.person_a, &mut cache), load(&r.person_b, &mut cache)) else { continue };
            let cb = date_of(&child, "birth", person_over.get(&child.id));
            let pb = date_of(&parent, "birth", person_over.get(&parent.id));
            let pd = date_of(&parent, "death", person_over.get(&parent.id));
            let both = vec![parent.id.clone(), child.id.clone()];
            if let (Some(cb), Some(pb)) = (cb, pb) {
                let age = years(cb.earliest_day() - pb.latest_day());
                let max = if parent.sex == Sex::Female { MAX_MOTHER_YEARS } else { MAX_FATHER_YEARS };
                if age < MIN_PARENT_YEARS {
                    out.push(Warning {
                        code: "parent_too_young",
                        message: format!("{} would have been about {:.0} when {} was born.", parent.display_name, age.max(0.0), child.display_name),
                        people: both.clone(),
                        params: json!({"age": age.round()}),
                    });
                } else if years(cb.latest_day() - pb.earliest_day()) > max {
                    out.push(Warning {
                        code: "parent_too_old",
                        message: format!("{} would have been about {:.0} when {} was born.", parent.display_name, years(cb.latest_day() - pb.earliest_day()), child.display_name),
                        people: both.clone(),
                        params: json!({"age": years(cb.latest_day() - pb.earliest_day()).round()}),
                    });
                }
            }
            if let (Some(cb), Some(pd)) = (cb, pd) {
                // A father may die up to ~9 months before the birth; a mother may not.
                let slack = if parent.sex == Sex::Male { 280 } else { 0 };
                if pd.latest_day() + slack < cb.earliest_day() {
                    out.push(Warning {
                        code: "parent_died_before_birth",
                        message: format!("{} would have died before {} was born.", parent.display_name, child.display_name),
                        people: both,
                        params: json!({}),
                    });
                }
            }
        }
    }
    // Impact of removing people: links that go with them, and who is left without a recorded parent.
    for pid in &removed_people {
        let Some(p) = load(pid, &mut cache) else { continue };
        let links = store.relationships_of(&CTX, pid)?;
        let mut orphaned: Vec<String> = vec![];
        for l in links.iter().filter(|l| l.rel_type == RelType::ParentChild && &l.person_a == pid) {
            let others = store
                .relationships_of(&CTX, &l.person_b)?
                .iter()
                .filter(|x| x.rel_type == RelType::ParentChild && x.person_b == l.person_b && &x.person_a != pid && !removed_people.contains(&x.person_a))
                .count();
            if others == 0 {
                orphaned.push(l.person_b.clone());
            }
        }
        let names: Vec<String> = orphaned.iter().filter_map(|i| load(i, &mut cache)).map(|c| c.display_name).collect();
        let tail = if names.is_empty() { String::new() } else { format!(", and {} would be left without a recorded parent ({})", names.len(), names.join(", ")) };
        let mut people = vec![pid.clone()];
        people.extend(orphaned);
        out.push(Warning {
            code: "person_removal_impact",
            message: format!("Removing {} also removes {} link{}{tail}. It can be undone from History.", p.display_name, links.len(), if links.len() == 1 { "" } else { "s" }),
            people,
            params: json!({"links": links.len()}),
        });
    }
    // Possible duplicates of new people.
    for id in &recs.people {
        let Some(p) = load(id, &mut cache) else { continue };
        let hits = store.search_people(&CTX, &family_store::SearchQuery { text: p.display_name.clone(), status: Some(family_store::Status::Verified), limit: 5 })?;
        let same: Vec<String> = hits.iter().filter(|h| h.display_name.eq_ignore_ascii_case(&p.display_name)).map(|h| h.id.clone()).collect();
        if !same.is_empty() {
            let mut people = vec![id.clone()];
            people.extend(same.iter().cloned());
            out.push(Warning {
                code: "possible_duplicate",
                message: format!("A person named {} already exists in the tree; check this is not the same person.", p.display_name),
                people,
                params: json!({"existing": same}),
            });
        }
    }
    // Impact of link-kind edits.
    if !kind_over.is_empty() {
        let g = store.load_graph(&CTX, GraphFilter::default())?;
        let mut parents: HashMap<String, Vec<(String, String, LinkKind)>> = HashMap::new();
        let mut kids: HashMap<String, Vec<String>> = HashMap::new();
        for l in g.links.iter().filter(|l| l.rel_type == RelType::ParentChild) {
            let (a, b) = (g.person(l.a).id.clone(), g.person(l.b).id.clone());
            parents.entry(b.clone()).or_default().push((l.id.clone(), a.clone(), l.kind.unwrap_or_default()));
            kids.entry(a).or_default().push(b);
        }
        for (link_id, new_kind) in &kind_over {
            let Ok(r) = store.get_relationship(&CTX, link_id) else { continue };
            let child = &r.person_b;
            let before = blood_ancestors(&parents, child, None);
            let after = blood_ancestors(&parents, child, Some((link_id, *new_kind)));
            let lost: Vec<_> = before.difference(&after).cloned().collect();
            let gained: Vec<_> = after.difference(&before).cloned().collect();
            if lost.is_empty() && gained.is_empty() {
                continue;
            }
            // Blood descendants of the child (and the child) are affected as well.
            let mut desc = BTreeSet::from([child.clone()]);
            let mut q = VecDeque::from([child.clone()]);
            while let Some(c) = q.pop_front() {
                for k in kids.get(&c).into_iter().flatten() {
                    let birth = parents.get(k).map(|v| v.iter().any(|(id, p, kind)| p == &c && *kind == LinkKind::Birth && id != link_id)).unwrap_or(false);
                    if birth && desc.insert(k.clone()) {
                        q.push_back(k.clone());
                    }
                }
            }
            let n = lost.len() + gained.len();
            let answers = n * desc.len();
            let (verb, count) = if gained.is_empty() { ("removes", lost.len()) } else { ("adds", gained.len()) };
            let msg = format!(
                "Changing this link to {} {verb} {count} {} {} {}'s blood lineage and changes up to {answers} stored relationship answer{}.",
                new_kind.as_str(),
                if count == 1 { "person" } else { "people" },
                if gained.is_empty() { "from" } else { "to" },
                name_of(store, child),
                if answers == 1 { "" } else { "s" }
            );
            out.push(Warning {
                code: "link_kind_impact",
                message: msg,
                people: std::iter::once(child.clone()).chain(lost).chain(gained).collect(),
                params: json!({"link": link_id, "kind": new_kind.as_str(), "affected_pairs": answers}),
            });
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct Dependency {
    pub proposal_id: String,
    pub summary: Option<String>,
    pub status: String,
    pub created_by: String,
}

/// Proposals whose still-pending records this proposal links to or edits.
pub fn dependencies(store: &Store, proposal_id: &str) -> Result<Vec<Dependency>> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let mut note = |owner: Option<String>, status: family_store::Status| {
        if let (Some(o), family_store::Status::Pending) = (owner, status) {
            if o != proposal_id {
                ids.insert(o);
            }
        }
    };
    let recs = store.proposal_records(&CTX, proposal_id)?;
    for rid in &recs.relationships {
        let r = store.get_relationship(&CTX, rid)?;
        for pid in [&r.person_a, &r.person_b] {
            if let Ok(p) = store.get_person(&CTX, pid) {
                note(p.proposal_id.clone(), p.status);
            }
        }
    }
    for c in store.changes_of(&CTX, proposal_id)? {
        match c.target_type.as_str() {
            "person" => {
                if let Ok(p) = store.get_person(&CTX, &c.target_id) {
                    note(p.proposal_id.clone(), p.status);
                }
            }
            "person_name" => {
                if let Ok((pid, _)) = store.find_name(&CTX, &c.target_id) {
                    if let Ok(p) = store.get_person(&CTX, &pid) {
                        note(p.proposal_id.clone(), p.status);
                    }
                }
            }
            _ => {
                if let Ok(r) = store.get_relationship(&CTX, &c.target_id) {
                    note(r.proposal_id.clone(), r.status);
                }
            }
        }
    }
    let mut out = vec![];
    for id in ids {
        if let Ok(p) = store.get_proposal(&CTX, &id) {
            out.push(Dependency { proposal_id: id, summary: p.summary, status: p.status, created_by: p.created_by });
        }
    }
    Ok(out)
}
