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

fn year_of(d: &Option<family_store::FuzzyDate>) -> Option<u16> {
    d.as_ref().map(|d| d.date.year())
}

/// A short card for one person: enough to tell two people with the same name apart.
pub fn card(store: &Store, id: &str) -> Result<Value> {
    let p = store.get_person(&CTX, id)?;
    let (mut parents, mut partners, mut children) = (vec![], vec![], 0usize);
    for r in store.relationships_of(&CTX, id)? {
        let name_of = |o: &str| store.get_person(&CTX, o).map(|p| p.display_name).unwrap_or_default();
        match (r.rel_type, r.person_a == id) {
            (RelType::ParentChild, true) => children += 1,
            (RelType::ParentChild, false) => parents.push(json!({"id": r.person_a, "name": name_of(&r.person_a)})),
            (RelType::Partnership, a_is_me) => {
                let o = if a_is_me { r.person_b.clone() } else { r.person_a.clone() };
                partners.push(json!({"id": o, "name": name_of(&o)}));
            }
        }
    }
    Ok(json!({
        "id": p.id, "name": p.display_name, "display_name": p.display_name, "sex": p.sex, "is_living": p.is_living,
        "birth_year": year_of(&p.birth), "death_year": year_of(&p.death),
        "status": p.status, "proposal_id": p.proposal_id,
        "parents": parents, "partners": partners, "children_count": children,
    }))
}

pub fn search_cards(store: &Store, text: &str, limit: usize, include_pending: bool) -> Result<Vec<Value>> {
    search(store, text, limit, include_pending)?.iter().map(|s| card(store, &s.id)).collect()
}

/// `male`, `female`, `other`, `unknown` or the stored codes `M`, `F`, `X`, `U` (any case) -> the stored value.
pub fn parse_sex(s: &str) -> std::result::Result<family_store::Sex, String> {
    use family_store::Sex;
    match s.trim().to_lowercase().as_str() {
        "m" | "male" => Ok(Sex::Male),
        "f" | "female" => Ok(Sex::Female),
        "x" | "other" => Ok(Sex::Other),
        "u" | "unknown" => Ok(Sex::Unknown),
        _ => Err(format!("'{s}' is not a sex: use one of male, female, other, unknown (or M, F, X, U)")),
    }
}

#[derive(Debug, Default, Clone)]
pub struct ListOpts {
    /// `verified` (default), `pending` or `all`.
    pub status: String,
    pub name: Option<String>,
    pub sex: Option<String>,
    pub no_parents: bool,
    pub no_children: bool,
    pub no_partners: bool,
    pub created_by_proposal: Option<String>,
    /// Cursor from the previous page's `next_cursor`: continue after the last person you received.
    pub after: Option<String>,
    pub limit: usize,
    /// Which card fields to return (default: all). Cheap ones skip the relative lookups.
    pub fields: Option<Vec<String>>,
    /// `columns` returns `{columns, rows}` instead of one object per person.
    pub columnar: bool,
    /// `sex` or `status`: return counts per value instead of people.
    pub group_by: Option<String>,
}

pub const LIST_DEFAULT: usize = 50;
pub const LIST_MAX: usize = 2000;
pub const CARD_FIELDS: &[&str] = &["id", "name", "sex", "birth_year", "death_year", "is_living", "status", "proposal_id", "parents", "partners", "children_count"];

fn bad(field: &str, msg: String) -> crate::ReviewError {
    StoreError::invalid(field, msg).into()
}

/// Page through people. Filters combine with AND. Returns cards plus paging info, the filters that were
/// applied (so an empty result is never ambiguous), and `tree_version`. Bad filter values are errors.
pub fn list_people(store: &Store, o: &ListOpts) -> Result<Value> {
    let status = if o.status.is_empty() { "verified" } else { o.status.as_str() };
    if !["verified", "pending", "all"].contains(&status) {
        return Err(bad("status", format!("'{status}' is not a status: use one of verified, pending, all")));
    }
    let sex_filter = match o.sex.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => Some(parse_sex(s).map_err(|m| bad("sex", m))?),
        None => None,
    };
    let fields: Option<Vec<&str>> = match &o.fields {
        Some(f) if !f.is_empty() => {
            for x in f {
                if !CARD_FIELDS.contains(&x.as_str()) {
                    return Err(bad("fields", format!("'{x}' is not a field: use any of {}", CARD_FIELDS.join(", "))));
                }
            }
            Some(f.iter().map(String::as_str).collect())
        }
        _ => None,
    };
    if let Some(g) = o.group_by.as_deref() {
        if !["sex", "status"].contains(&g) {
            return Err(bad("group_by", format!("'{g}' cannot be grouped: use sex or status")));
        }
    }
    let g = store.load_graph(&CTX, GraphFilter { include_pending: status != "verified" })?;
    let n = g.people.len();
    let (mut has_parent, mut has_child, mut has_partner) = (vec![false; n], vec![false; n], vec![false; n]);
    for l in &g.links {
        let (a, b) = (l.a as usize, l.b as usize);
        match l.rel_type {
            RelType::ParentChild => {
                has_child[a] = true;
                has_parent[b] = true;
            }
            RelType::Partnership => {
                has_partner[a] = true;
                has_partner[b] = true;
            }
        }
    }
    let of_proposal: Option<HashSet<String>> = match &o.created_by_proposal {
        Some(p) => Some(store.people_of_proposal(&CTX, p)?.into_iter().collect()),
        None => None,
    };
    let needle = o.name.as_deref().map(family_store::names::search_key).filter(|k| !k.is_empty());
    let mut hits: Vec<usize> = vec![];
    for (i, p) in g.people.iter().enumerate() {
        if status == "pending" && p.status != Status::Pending {
            continue;
        }
        if o.no_parents && has_parent[i] || o.no_children && has_child[i] || o.no_partners && has_partner[i] {
            continue;
        }
        if sex_filter.is_some_and(|s| p.sex != s) {
            continue;
        }
        if of_proposal.as_ref().is_some_and(|set| !set.contains(&p.id)) {
            continue;
        }
        if needle.as_ref().is_some_and(|k| !family_store::names::search_key(&p.display_name).contains(k.as_str())) {
            continue;
        }
        hits.push(i);
    }
    hits.sort_by(|&a, &b| g.people[a].id.cmp(&g.people[b].id));
    let total = hits.len();
    let applied = json!({
        "status": status, "name": o.name, "sex": sex_filter.map(|s| s.as_str()),
        "no_parents": o.no_parents, "no_children": o.no_children, "no_partners": o.no_partners,
        "created_by_proposal": o.created_by_proposal,
    });
    let version = tree_version(store)?;
    if let Some(by) = o.group_by.as_deref() {
        let mut groups = std::collections::BTreeMap::<String, usize>::new();
        for &i in &hits {
            let key = if by == "sex" { g.people[i].sex.as_str().to_string() } else { g.people[i].status.as_str().to_string() };
            *groups.entry(key).or_default() += 1;
        }
        return Ok(json!({"group_by": by, "groups": groups, "total": total, "filters": applied, "tree_version": version}));
    }
    let limit = if o.limit == 0 { LIST_DEFAULT } else { o.limit.min(LIST_MAX) };
    // A cursor is `<tree_version when the first page was read>~<last id received>`; people are sorted by id.
    let (start_version, after_id) = match o.after.as_deref().filter(|c| !c.is_empty()) {
        None => (None, None),
        Some(c) => match c.split_once('~') {
            Some((v, id)) if !id.is_empty() => (Some(v.to_string()), Some(id.to_string())),
            _ => return Err(bad("after", "not a cursor: pass the `next_cursor` value from the previous page unchanged".into())),
        },
    };
    let from = after_id.as_ref().map_or(0, |id| hits.partition_point(|&i| g.people[i].id.as_str() <= id.as_str()));
    let window = &hits[from..];
    let slice = &window[..window.len().min(limit)];
    let wanted: Vec<&str> = fields.clone().unwrap_or_else(|| CARD_FIELDS.to_vec());
    let needs_card = wanted.iter().any(|f| ["proposal_id", "parents", "partners", "children_count"].contains(f));
    let mut page: Vec<Value> = vec![];
    for &i in slice {
        let p = &g.people[i];
        let full = if needs_card {
            card(store, &p.id)?
        } else {
            json!({"id": p.id, "name": p.display_name, "sex": p.sex, "is_living": p.is_living, "birth_year": year_of(&p.birth), "death_year": year_of(&p.death), "status": p.status})
        };
        page.push(full);
    }
    let first_version = start_version.clone().unwrap_or_else(|| version.clone());
    let next_cursor = match slice.last() {
        Some(&last) if slice.len() < window.len() => json!(format!("{first_version}~{}", g.people[last].id)),
        _ => Value::Null,
    };
    let mut out = json!({"total": total, "remaining_after_this_page": window.len() - slice.len(), "next_cursor": next_cursor, "filters": applied, "tree_version": version});
    if let Some(v) = start_version {
        // The tree changed since the first page was read: people may have been added or removed in the part already read.
        out["changed_since_start"] = json!(v != version);
    }
    if o.columnar {
        let rows: Vec<Value> = page.iter().map(|c| Value::Array(wanted.iter().map(|f| c[*f].clone()).collect())).collect();
        out["columns"] = json!(wanted);
        out["rows"] = Value::Array(rows);
    } else {
        let projected: Vec<Value> = page.iter().map(|c| if fields.is_some() { Value::Object(wanted.iter().map(|f| (f.to_string(), c[*f].clone())).collect()) } else { c.clone() }).collect();
        out["people"] = Value::Array(projected);
    }
    Ok(out)
}

/// Changes whenever anything in the tree changes (the id of the newest revision), so an agent can tell a cached read is stale.
pub fn tree_version(store: &Store) -> Result<String> {
    Ok(store.recent_revisions(&CTX, None, 1)?.first().map(|r| r.id.clone()).unwrap_or_default())
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
            json!({"id": p.id, "name": p.display_name, "display_name": p.display_name, "sex": p.sex, "birth": p.birth, "death": p.death, "is_living": p.is_living, "status": p.status})
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
