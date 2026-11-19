//! Read side shared by the agent API, MCP and the UI. Verified data only unless `include_pending`,
//! in which case open proposals' **changes** are applied on top and results say so.

use crate::Result;
use family_relate::{FindOptions, RelGraph};
use family_store::{AccessContext, ChangeRow, Graph, GraphFilter, LinkKind, PartnershipState, Person, PersonSummary, RelType, Relationship, SearchQuery, Status, Store, StoreError};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet, VecDeque};

const CTX: AccessContext = AccessContext::Everything;

pub fn search(store: &Store, text: &str, limit: usize, include_pending: bool) -> Result<Vec<PersonSummary>> {
    let status = if include_pending { None } else { Some(Status::Verified) };
    Ok(store.search_people(&CTX, &SearchQuery { text: text.to_string(), status, limit })?)
}

#[derive(Debug, Serialize)]
pub struct LinkedPerson {
    pub link_id: String,
    /// What the other person is to this one: `parent`, `child` or `partner`.
    pub role: &'static str,
    pub kind: Option<LinkKind>,
    pub subtype: Option<PartnershipState>,
    pub status: Status,
    pub other_id: String,
    pub other_name: String,
}

#[derive(Debug, Serialize)]
pub struct PersonDetail {
    pub person: Person,
    pub links: Vec<LinkedPerson>,
    /// Edits other agents (or this one) have proposed and the user has not reviewed yet.
    pub suggested_changes: Vec<ChangeRow>,
}

pub fn person_detail(store: &Store, id: &str) -> Result<PersonDetail> {
    let person = store.get_person(&CTX, id)?;
    let mut links = vec![];
    for r in store.relationships_of(&CTX, id)? {
        let (role, other) = match (r.rel_type, r.person_a == id) {
            (RelType::ParentChild, true) => ("child", r.person_b.clone()),
            (RelType::ParentChild, false) => ("parent", r.person_a.clone()),
            (RelType::Partnership, a_is_me) => ("partner", if a_is_me { r.person_b.clone() } else { r.person_a.clone() }),
        };
        let other_name = store.get_person(&CTX, &other).map(|p| p.display_name).unwrap_or_default();
        links.push(LinkedPerson { link_id: r.id, role, kind: r.kind, subtype: r.subtype, status: r.status, other_id: other, other_name });
    }
    let mut suggested_changes = store.open_changes_for(&CTX, "person", id)?;
    for n in &person.names {
        suggested_changes.extend(store.open_changes_for(&CTX, "person_name", &n.id)?);
    }
    for l in &links {
        suggested_changes.extend(store.open_changes_for(&CTX, "relationship", &l.link_id)?);
    }
    Ok(PersonDetail { person, links, suggested_changes })
}

/// Graph snapshot. With `include_pending`, pending people/links are included and the changes of
/// open proposals are applied on top (kind, subtype, dates, removals).
pub fn load_graph(store: &Store, include_pending: bool) -> Result<Graph> {
    let mut g = store.load_graph(&CTX, GraphFilter { include_pending })?;
    if !include_pending {
        return Ok(g);
    }
    let open: Vec<_> = store.list_proposals(&CTX, Some("pending"), None, 1000)?;
    let mut removed: HashSet<String> = HashSet::new();
    for p in open {
        for c in store.changes_of(&CTX, &p.id)? {
            let patch = |f: &str| c.patch.get(f).and_then(|x| x["to"].as_str()).map(str::to_string);
            match (c.target_type.as_str(), c.op.as_str()) {
                ("relationship", "remove") => {
                    removed.insert(c.target_id.clone());
                }
                ("relationship", _) => {
                    if let Some(l) = g.links.iter_mut().find(|l| l.id == c.target_id) {
                        if let Some(k) = patch("kind").and_then(|s| LinkKind::parse(&s).ok()) {
                            l.kind = Some(k);
                        }
                        if let Some(s) = patch("subtype").and_then(|s| PartnershipState::parse(&s).ok()) {
                            l.subtype = Some(s);
                        }
                        if c.patch.get("end_date").is_some() {
                            l.end_date = patch("end_date");
                        }
                    }
                }
                ("person", _) => {
                    if let Some(i) = g.index_of(&c.target_id) {
                        let q = |f: &str| patch(f).and_then(|s| family_store::Qualifier::parse(&s).ok());
                        let person = &mut g.people[i as usize];
                        if let Some(s) = patch("sex").and_then(|s| family_store::Sex::parse(&s).ok()) {
                            person.sex = s;
                        }
                        for (df, qf, is_birth) in [("birth_date", "birth_qualifier", true), ("death_date", "death_qualifier", false)] {
                            if c.patch.get(df).is_some() || c.patch.get(qf).is_some() {
                                let cur = if is_birth { person.birth } else { person.death };
                                let d = if c.patch.get(df).is_some() { patch(df) } else { cur.map(|x| x.date.to_string()) };
                                let qual = q(qf).or(cur.map(|x| x.qualifier)).unwrap_or_default();
                                let new = d.and_then(|s| family_store::FuzzyDate::parse(&s, qual).ok());
                                if is_birth {
                                    person.birth = new;
                                } else {
                                    person.death = new;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if !removed.is_empty() {
        g.links.retain(|l| !removed.contains(&l.id));
    }
    Ok(g)
}

#[derive(Debug, Serialize)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct RelationAnswer {
    pub a: Endpoint,
    pub b: Endpoint,
    /// `same`, `blood`, `connection` or `none`.
    pub kind: family_relate::RelationKind,
    pub by_blood: bool,
    /// "a is the <label> of b"
    pub a_is: Option<String>,
    pub b_is: Option<String>,
    pub label_key: Option<String>,
    pub chain: Option<String>,
    pub generations: Option<(u32, u32)>,
    pub common_ancestors: Vec<Endpoint>,
    pub has_non_birth_link: bool,
    pub includes_pending: bool,
    pub waiting_for_review: bool,
}

pub fn relationship(store: &Store, a: &str, b: &str, include_pending: bool, all: bool) -> Result<RelationAnswer> {
    let g = load_graph(store, include_pending)?;
    let ia = g.index_of(a).ok_or_else(|| StoreError::NotFound { kind: "person", id: a.to_string() })?;
    let ib = g.index_of(b).ok_or_else(|| StoreError::NotFound { kind: "person", id: b.to_string() })?;
    let rg = RelGraph::new(&g, include_pending);
    let rel = rg.find(ia, ib, &FindOptions { all, max_cost: 40.0, include_pending });
    let ep = |i: u32| Endpoint { id: g.person(i).id.clone(), name: g.person(i).display_name.clone() };
    Ok(RelationAnswer {
        a: ep(ia),
        b: ep(ib),
        kind: rel.kind,
        by_blood: rel.by_blood,
        a_is: rel.label.as_ref().map(|l| l.text.clone()),
        b_is: rel.reverse_label.as_ref().map(|l| l.text.clone()),
        label_key: rel.label.as_ref().map(|l| l.key.clone()),
        chain: rel.chain.clone(),
        generations: rel.generations,
        common_ancestors: rel.common_ancestors.iter().map(|i| ep(*i)).collect(),
        has_non_birth_link: rel.has_non_birth,
        includes_pending: rel.includes_pending,
        waiting_for_review: rel.waiting_for_review,
    })
}

const MAX_GRAPH_NODES: usize = 500;

/// `up` generations of parents, `down` generations of children around `focus`, plus partners of everyone included.
pub fn neighbourhood(store: &Store, focus: &str, up: u32, down: u32, include_pending: bool) -> Result<Value> {
    let g = load_graph(store, include_pending)?;
    let f = g.index_of(focus).ok_or_else(|| StoreError::NotFound { kind: "person", id: focus.to_string() })?;
    let (up, down) = (up.min(10), down.min(10));
    let mut inc: BTreeSet<u32> = BTreeSet::from([f]);
    let mut truncated = false;
    for (dir_up, depth) in [(true, up), (false, down)] {
        let mut frontier = vec![f];
        for _ in 0..depth {
            let mut next = vec![];
            for p in &frontier {
                for l in g.links.iter().filter(|l| l.rel_type == RelType::ParentChild) {
                    let other = if dir_up && l.b == *p {
                        Some(l.a)
                    } else if !dir_up && l.a == *p {
                        Some(l.b)
                    } else {
                        None
                    };
                    if let Some(o) = other {
                        if inc.len() >= MAX_GRAPH_NODES {
                            truncated = true;
                        } else if inc.insert(o) {
                            next.push(o);
                        }
                    }
                }
            }
            frontier = next;
        }
    }
    let core: Vec<u32> = inc.iter().copied().collect();
    for l in g.links.iter().filter(|l| l.rel_type == RelType::Partnership) {
        for (x, y) in [(l.a, l.b), (l.b, l.a)] {
            if core.contains(&x) && inc.len() < MAX_GRAPH_NODES {
                inc.insert(y);
            }
        }
    }
    let people: Vec<Value> = inc
        .iter()
        .map(|i| {
            let p = g.person(*i);
            json!({"id": p.id, "name": p.display_name, "sex": p.sex, "birth": p.birth, "death": p.death, "is_living": p.is_living, "status": p.status})
        })
        .collect();
    let links: Vec<Value> = g
        .links
        .iter()
        .filter(|l| inc.contains(&l.a) && inc.contains(&l.b))
        .map(|l| json!({"id": l.id, "type": l.rel_type, "a": g.person(l.a).id, "b": g.person(l.b).id, "kind": l.kind, "subtype": l.subtype, "pending": l.pending}))
        .collect();
    let _: VecDeque<u8> = VecDeque::new();
    let _: Option<Relationship> = None;
    Ok(json!({"focus": focus, "people": people, "links": links, "truncated": truncated, "includes_pending": include_pending}))
}
