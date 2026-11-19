//! `POST /agent/v1/proposals` and the MCP `create_proposal` tool.
//!
//! Strict: unknown fields, bad values, unknown ids and stale `from` values all come back as
//! field-level errors the agent can act on. Nothing is written unless everything is valid, and
//! everything written is **pending** (new people/links) or **stored as a change** (edits).
//! Verified data is never modified here.

use crate::fields::{allowed_fields, check_to, current_name_field, current_person_field, current_rel_field, norm};
use crate::review::{proposal_warnings, Warning};
use crate::{clean_agent_name, FieldError, Result, ReviewError};
use family_store::{AccessContext, FuzzyDate, LinkKind, NameType, NewName, NewPerson, NewRelationship, Origin, PartialDate, PartnershipState, Qualifier, RelType, Sex, Status, Store, StoreError};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

const CTX: AccessContext = AccessContext::Everything;
pub const MAX_PEOPLE: usize = 200;
pub const MAX_RELATIONSHIPS: usize = 400;
pub const MAX_CHANGES: usize = 200;
pub const MAX_OPEN_PROPOSALS_PER_AGENT: usize = 50;

#[derive(Debug, Serialize)]
pub struct SubmitResult {
    pub proposal_id: String,
    pub status: &'static str,
    pub people: Vec<RefId>,
    pub relationships: Vec<IndexId>,
    pub changes: Vec<IndexId>,
    pub warnings: Vec<Warning>,
    pub note: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RefId {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct IndexId {
    pub index: usize,
    pub id: String,
}

struct Errs(Vec<FieldError>);

impl Errs {
    fn add(&mut self, path: impl Into<String>, code: &'static str, msg: impl Into<String>) {
        self.0.push(FieldError::new(path, code, msg));
    }
}

fn as_obj<'a>(v: &'a Value, path: &str, errs: &mut Errs) -> Option<&'a Map<String, Value>> {
    match v.as_object() {
        Some(o) => Some(o),
        None => {
            errs.add(path, "wrong_type", "must be an object");
            None
        }
    }
}

fn unknown_keys(o: &Map<String, Value>, path: &str, allowed: &[&str], errs: &mut Errs) {
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) {
            errs.add(format!("{path}.{k}"), "unknown_field", format!("'{k}' is not a field of this object; allowed: {}", allowed.join(", ")));
        }
    }
}

/// Optional string field, trimmed; `""` and null count as absent.
fn opt_str(o: &Map<String, Value>, key: &str, path: &str, max: usize, errs: &mut Errs) -> Option<String> {
    match o.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.chars().count() > max {
                errs.add(format!("{path}.{key}"), "too_long", format!("longer than {max} characters"));
                return None;
            }
            (!t.is_empty()).then(|| t.to_string())
        }
        Some(_) => {
            errs.add(format!("{path}.{key}"), "wrong_type", "must be a string");
            None
        }
    }
}

fn date_pair(o: &Map<String, Value>, date_key: &str, qual_key: &str, path: &str, errs: &mut Errs) -> Option<FuzzyDate> {
    let d = opt_str(o, date_key, path, 10, errs);
    let q = opt_str(o, qual_key, path, 10, errs);
    let q = match q {
        Some(q) => match Qualifier::parse(&q) {
            Ok(q) => q,
            Err(e) => {
                errs.add(format!("{path}.{qual_key}"), "invalid_value", e.to_string());
                Qualifier::Exact
            }
        },
        None => Qualifier::Exact,
    };
    let d = d?;
    match FuzzyDate::parse(&d, q) {
        Ok(f) => Some(f),
        Err(e) => {
            errs.add(format!("{path}.{date_key}"), "invalid_date", format!("{e} (use YYYY, YYYY-MM or YYYY-MM-DD)"));
            None
        }
    }
}

struct PersonPlan {
    ref_: String,
    new: NewPerson,
    note: Option<String>,
}

struct RelPlan {
    new: NewRelationship,
    a: Endpoint,
    b: Endpoint,
    note: Option<String>,
}

#[derive(Clone)]
enum Endpoint {
    Ref(String),
    Id(String),
}

struct ChangePlan {
    target_type: String,
    target_id: String,
    op: &'static str,
    patch: Value,
    base_updated_at: String,
    note: Option<String>,
}

fn parse_name(o: &Map<String, Value>, path: &str, errs: &mut Errs) -> Option<NewName> {
    let given = opt_str(o, "given_name", path, 300, errs);
    let family = opt_str(o, "family_name", path, 300, errs);
    let full = opt_str(o, "full_name", path, 300, errs);
    let script = opt_str(o, "script", path, 40, errs);
    let name_type = match opt_str(o, "name_type", path, 20, errs) {
        Some(t) => match NameType::parse(&t) {
            Ok(t) => Some(t),
            Err(e) => {
                errs.add(format!("{path}.name_type"), "invalid_value", e.to_string());
                None
            }
        },
        None => None,
    };
    if given.is_none() && family.is_none() && full.is_none() {
        errs.add(path, "missing_name", "give full_name, or given_name and/or family_name");
        return None;
    }
    Some(NewName { name_type, given, family, full_name: full, script })
}

fn existing_person(store: &Store, id: &str, path: &str, errs: &mut Errs) {
    match store.get_person(&CTX, id) {
        Ok(p) if p.status == Status::Verified => {}
        Ok(_) => errs.add(path, "pending_target", "this person is still waiting for review (it belongs to an open proposal); use a `ref` in the same proposal or wait for approval"),
        Err(StoreError::NotFound { .. }) => errs.add(path, "unknown_id", format!("no person with id '{id}'; search first, or use a `ref` of a person in this proposal")),
        Err(e) => errs.add(path, "lookup_failed", e.to_string()),
    }
}

/// Validate and store a proposal. `agent` is the (already clean) agent name.
pub fn submit(store: &mut Store, agent: &str, body: &Value) -> Result<SubmitResult> {
    let agent = clean_agent_name(agent);
    let created_by = format!("agent:{agent}");
    let mut errs = Errs(vec![]);
    let Some(top) = as_obj(body, "$", &mut errs) else {
        return Err(ReviewError::Invalid(errs.0));
    };
    unknown_keys(top, "$", &["summary", "source_text", "people", "relationships", "changes"], &mut errs);
    let summary = opt_str(top, "summary", "$", 300, &mut errs);
    if summary.is_none() {
        errs.add("$.summary", "required", "a short summary of the request is required (what the user asked for)");
    }
    let source_text = opt_str(top, "source_text", "$", 5000, &mut errs).unwrap_or_default();

    let arr = |key: &str, max: usize, errs: &mut Errs| -> Vec<Value> {
        match top.get(key) {
            None | Some(Value::Null) => vec![],
            Some(Value::Array(a)) => {
                if a.len() > max {
                    errs.add(format!("$.{key}"), "too_many", format!("at most {max} items per proposal"));
                    vec![]
                } else {
                    a.clone()
                }
            }
            Some(_) => {
                errs.add(format!("$.{key}"), "wrong_type", "must be an array");
                vec![]
            }
        }
    };
    let people_in = arr("people", MAX_PEOPLE, &mut errs);
    let rels_in = arr("relationships", MAX_RELATIONSHIPS, &mut errs);
    let changes_in = arr("changes", MAX_CHANGES, &mut errs);
    if people_in.is_empty() && rels_in.is_empty() && changes_in.is_empty() && errs.0.is_empty() {
        errs.add("$", "empty", "a proposal needs at least one person, relationship or change");
    }

    // Do not let one agent bury the review screen.
    match store.list_proposals(&CTX, Some("pending"), Some(&created_by), 1000) {
        Ok(open) if open.len() >= MAX_OPEN_PROPOSALS_PER_AGENT => {
            errs.add("$", "too_many_open_proposals", format!("{} proposals from this agent are still waiting for review; wait for the user to review them", open.len()))
        }
        Ok(_) => {}
        Err(e) => return Err(e.into()),
    }

    // ---- people ----
    let mut people: Vec<PersonPlan> = vec![];
    let mut refs: HashSet<String> = HashSet::new();
    for (i, pv) in people_in.iter().enumerate() {
        let path = format!("$.people[{i}]");
        let Some(o) = as_obj(pv, &path, &mut errs) else { continue };
        unknown_keys(
            o,
            &path,
            &[
                "ref",
                "given_name",
                "family_name",
                "full_name",
                "sex",
                "birth_date",
                "birth_qualifier",
                "birth_place",
                "death_date",
                "death_qualifier",
                "death_place",
                "is_living",
                "note",
                "agent_note",
                "other_names",
            ],
            &mut errs,
        );
        let ref_ = opt_str(o, "ref", &path, 40, &mut errs);
        match &ref_ {
            None => errs.add(format!("{path}.ref"), "required", "every new person needs a `ref` (any short label) so links can point to it"),
            Some(r) if !refs.insert(r.clone()) => errs.add(format!("{path}.ref"), "duplicate_ref", format!("ref '{r}' is used twice")),
            _ => {}
        }
        let sex = match opt_str(o, "sex", &path, 1, &mut errs) {
            Some(s) => match Sex::parse(&s) {
                Ok(s) => s,
                Err(e) => {
                    errs.add(format!("{path}.sex"), "invalid_value", e.to_string());
                    Sex::Unknown
                }
            },
            None => Sex::Unknown,
        };
        let is_living = match o.get("is_living") {
            None | Some(Value::Null) => None,
            Some(Value::Bool(b)) => Some(*b),
            Some(_) => {
                errs.add(format!("{path}.is_living"), "wrong_type", "must be true or false");
                None
            }
        };
        let birth = date_pair(o, "birth_date", "birth_qualifier", &path, &mut errs);
        let death = date_pair(o, "death_date", "death_qualifier", &path, &mut errs);
        if is_living == Some(true) && death.is_some() {
            errs.add(format!("{path}.is_living"), "conflicting_fields", "a person with a death date cannot be marked as living");
        }
        let mut names = vec![];
        if let Some(n) = parse_name(o, &path, &mut errs) {
            names.push(n);
        }
        match o.get("other_names") {
            None | Some(Value::Null) => {}
            Some(Value::Array(a)) => {
                for (j, nv) in a.iter().enumerate() {
                    let np = format!("{path}.other_names[{j}]");
                    if let Some(no) = as_obj(nv, &np, &mut errs) {
                        unknown_keys(no, &np, &["given_name", "family_name", "full_name", "name_type", "script"], &mut errs);
                        if let Some(n) = parse_name(no, &np, &mut errs) {
                            names.push(n);
                        }
                    }
                }
            }
            Some(_) => errs.add(format!("{path}.other_names"), "wrong_type", "must be an array"),
        }
        let new = NewPerson {
            sex,
            is_living,
            birth,
            birth_place: opt_str(o, "birth_place", &path, 300, &mut errs),
            death,
            death_place: opt_str(o, "death_place", &path, 300, &mut errs),
            note: opt_str(o, "note", &path, 20_000, &mut errs),
            names,
        };
        let note = opt_str(o, "agent_note", &path, 1000, &mut errs);
        people.push(PersonPlan { ref_: ref_.unwrap_or_default(), new, note });
    }

    // ---- relationships ----
    let mut rels: Vec<RelPlan> = vec![];
    let mut seen_pairs: HashSet<(String, String, String)> = HashSet::new();
    for (i, rv) in rels_in.iter().enumerate() {
        let path = format!("$.relationships[{i}]");
        let Some(o) = as_obj(rv, &path, &mut errs) else { continue };
        unknown_keys(o, &path, &["type", "a", "b", "kind", "subtype", "start_date", "end_date", "place", "note", "agent_note"], &mut errs);
        let rel_type = match opt_str(o, "type", &path, 20, &mut errs) {
            Some(t) => match RelType::parse(&t) {
                Ok(t) => Some(t),
                Err(e) => {
                    errs.add(format!("{path}.type"), "invalid_value", e.to_string());
                    None
                }
            },
            None => {
                errs.add(format!("{path}.type"), "required", "type must be parent_child or partnership");
                None
            }
        };
        let endpoint = |key: &str, errs: &mut Errs| -> Option<Endpoint> {
            let v = opt_str(o, key, &path, 80, errs);
            let Some(v) = v else {
                errs.add(format!("{path}.{key}"), "required", "an existing person id or the `ref` of a person in this proposal");
                return None;
            };
            if refs.contains(&v) {
                Some(Endpoint::Ref(v))
            } else {
                existing_person(store, &v, &format!("{path}.{key}"), errs);
                Some(Endpoint::Id(v))
            }
        };
        let a = endpoint("a", &mut errs);
        let b = endpoint("b", &mut errs);
        let kind = match opt_str(o, "kind", &path, 20, &mut errs) {
            Some(k) => match LinkKind::parse(&k) {
                Ok(k) => Some(k),
                Err(e) => {
                    errs.add(format!("{path}.kind"), "invalid_value", e.to_string());
                    None
                }
            },
            None => None,
        };
        let subtype = match opt_str(o, "subtype", &path, 20, &mut errs) {
            Some(k) => match PartnershipState::parse(&k) {
                Ok(k) => Some(k),
                Err(e) => {
                    errs.add(format!("{path}.subtype"), "invalid_value", e.to_string());
                    None
                }
            },
            None => None,
        };
        if rel_type == Some(RelType::ParentChild) && subtype.is_some() {
            errs.add(format!("{path}.subtype"), "not_applicable", "only partnerships have a subtype");
        }
        if rel_type == Some(RelType::Partnership) && kind.is_some() {
            errs.add(format!("{path}.kind"), "not_applicable", "only parent-child links have a kind");
        }
        let date = |key: &str, errs: &mut Errs| -> Option<PartialDate> {
            let t = opt_str(o, key, &path, 10, errs)?;
            match PartialDate::parse(&t) {
                Ok(d) => Some(d),
                Err(e) => {
                    errs.add(format!("{path}.{key}"), "invalid_date", format!("{e} (use YYYY, YYYY-MM or YYYY-MM-DD)"));
                    None
                }
            }
        };
        let start_date = date("start_date", &mut errs);
        let end_date = date("end_date", &mut errs);
        let (Some(t), Some(a), Some(b)) = (rel_type, a, b) else {
            continue;
        };
        let key = |e: &Endpoint| match e {
            Endpoint::Ref(r) => format!("ref:{r}"),
            Endpoint::Id(r) => format!("id:{r}"),
        };
        if key(&a) == key(&b) {
            errs.add(path.clone(), "self_link", "a person cannot be linked to themselves");
            continue;
        }
        let pair = if t == RelType::Partnership && key(&a) > key(&b) { (key(&b), key(&a)) } else { (key(&a), key(&b)) };
        if !seen_pairs.insert((t.as_str().to_string(), pair.0, pair.1)) {
            errs.add(path.clone(), "duplicate", "the same link appears twice in this proposal");
            continue;
        }
        let note = opt_str(o, "agent_note", &path, 1000, &mut errs);
        let mut new = NewRelationship {
            rel_type: t,
            person_a: String::new(),
            person_b: String::new(),
            kind,
            subtype,
            start_date,
            end_date,
            place: opt_str(o, "place", &path, 300, &mut errs),
            note: opt_str(o, "note", &path, 20_000, &mut errs),
        };
        new.person_a.clear();
        rels.push(RelPlan { new, a, b, note });
    }

    // ---- changes to existing records ----
    let mut changes: Vec<ChangePlan> = vec![];
    let mut touched: HashSet<(String, String, String)> = HashSet::new();
    for (i, cv) in changes_in.iter().enumerate() {
        let path = format!("$.changes[{i}]");
        let Some(o) = as_obj(cv, &path, &mut errs) else { continue };
        unknown_keys(o, &path, &["target_type", "target_id", "op", "fields", "base_updated_at", "agent_note"], &mut errs);
        let target_type = opt_str(o, "target_type", &path, 20, &mut errs);
        let target_id = opt_str(o, "target_id", &path, 80, &mut errs);
        let op = opt_str(o, "op", &path, 10, &mut errs);
        let (Some(target_type), Some(target_id), Some(op)) = (target_type, target_id, op) else {
            errs.add(path.clone(), "required", "target_type, target_id and op are required");
            continue;
        };
        if !["person", "person_name", "relationship"].contains(&target_type.as_str()) {
            errs.add(format!("{path}.target_type"), "invalid_value", "must be person, person_name or relationship");
            continue;
        }
        let op: &'static str = match op.as_str() {
            "update" => "update",
            "remove" => "remove",
            "delete" | "merge" => {
                errs.add(format!("{path}.op"), "not_allowed", "deleting or merging people is a human action; only `update`, and `remove` of a relationship, can be proposed");
                continue;
            }
            _ => {
                errs.add(format!("{path}.op"), "invalid_value", "must be update or remove");
                continue;
            }
        };
        if op == "remove" && target_type != "relationship" {
            errs.add(format!("{path}.op"), "not_allowed", if target_type == "person" { "deleting a person is a human action" } else { "only relationships can be removed" });
            continue;
        }
        // Current state of the target.
        let (updated_at, status, rel_type, current): (String, Status, Option<RelType>, crate::fields::FieldReader) = match target_type.as_str() {
            "person" => match store.get_person(&CTX, &target_id) {
                Ok(p) => (p.updated_at.clone(), p.status, None, Box::new(move |f| current_person_field(&p, f))),
                Err(e) => {
                    errs.add(format!("{path}.target_id"), "unknown_id", e.to_string());
                    continue;
                }
            },
            "person_name" => match store.find_name(&CTX, &target_id) {
                Ok((pid, n)) => match store.get_person(&CTX, &pid) {
                    Ok(p) => (p.updated_at.clone(), p.status, None, Box::new(move |f| current_name_field(&n, f))),
                    Err(e) => {
                        errs.add(format!("{path}.target_id"), "unknown_id", e.to_string());
                        continue;
                    }
                },
                Err(e) => {
                    errs.add(format!("{path}.target_id"), "unknown_id", e.to_string());
                    continue;
                }
            },
            _ => match store.get_relationship(&CTX, &target_id) {
                Ok(r) => (r.updated_at.clone(), r.status, Some(r.rel_type), Box::new(move |f| current_rel_field(&r, f))),
                Err(e) => {
                    errs.add(format!("{path}.target_id"), "unknown_id", e.to_string());
                    continue;
                }
            },
        };
        if status == Status::Pending {
            errs.add(format!("{path}.target_id"), "pending_target", "this record is still waiting for review; submit a new proposal after it is approved");
            continue;
        }
        if let Some(b) = opt_str(o, "base_updated_at", &path, 40, &mut errs) {
            if b != updated_at {
                errs.add(format!("{path}.base_updated_at"), "stale", format!("the record changed after you read it (updated_at is now {updated_at}); read it again"));
                continue;
            }
        }
        let note = opt_str(o, "agent_note", &path, 1000, &mut errs);
        if op == "remove" {
            if o.contains_key("fields") {
                errs.add(format!("{path}.fields"), "not_applicable", "a remove has no fields");
            }
            if !touched.insert((target_type.clone(), target_id.clone(), "*".into())) {
                errs.add(path.clone(), "duplicate", "the same record is changed twice in this proposal");
                continue;
            }
            changes.push(ChangePlan { target_type, target_id, op, patch: Value::Null, base_updated_at: updated_at, note });
            continue;
        }
        let Some(fields) = o.get("fields").and_then(Value::as_object) else {
            errs.add(format!("{path}.fields"), "required", "an update needs `fields`: {\"<field>\": {\"from\": <value you saw>, \"to\": <new value>}}");
            continue;
        };
        if fields.is_empty() {
            errs.add(format!("{path}.fields"), "required", "no fields to change");
            continue;
        }
        let allowed = allowed_fields(&target_type, rel_type);
        let mut patch = Map::new();
        for (f, ft) in fields {
            let fp = format!("{path}.fields.{f}");
            if !allowed.contains(&f.as_str()) {
                errs.add(&fp, "field_not_editable", format!("'{f}' cannot be changed through a proposal; editable here: {}", allowed.join(", ")));
                continue;
            }
            let Some(ft) = ft.as_object() else {
                errs.add(&fp, "wrong_type", "must be {\"from\": ..., \"to\": ...}");
                continue;
            };
            if ft.keys().any(|k| k != "from" && k != "to") || !ft.contains_key("from") || !ft.contains_key("to") {
                errs.add(&fp, "wrong_type", "must be exactly {\"from\": <value you saw>, \"to\": <new value>}");
                continue;
            }
            let (from, to) = (norm(&ft["from"]), norm(&ft["to"]));
            let cur = norm(&current(f));
            if from != cur {
                errs.add(format!("{fp}.from"), "stale_from", format!("the current value is {cur}, not {from}; read the record again before proposing"));
                continue;
            }
            if let Err(m) = check_to(f, rel_type, &to) {
                errs.add(format!("{fp}.to"), "invalid_value", m);
                continue;
            }
            if from == to {
                errs.add(&fp, "no_change", "`to` equals the current value");
                continue;
            }
            if !touched.insert((target_type.clone(), target_id.clone(), f.clone())) {
                errs.add(&fp, "duplicate", "the same field is changed twice in this proposal");
                continue;
            }
            patch.insert(f.clone(), json!({ "from": from, "to": to }));
        }
        if !patch.is_empty() {
            changes.push(ChangePlan { target_type, target_id, op, patch: Value::Object(patch), base_updated_at: updated_at, note });
        }
    }

    if !errs.0.is_empty() {
        return Err(ReviewError::Invalid(errs.0));
    }

    // ---- write everything or nothing ----
    let mut fail: Option<(String, &'static str, String)> = None;
    let out = store.atomic(|s| {
        let pid = s.insert_proposal(&CTX, summary.as_deref().unwrap_or(""), &source_text, &created_by)?;
        let mut ids: HashMap<String, String> = HashMap::new();
        let mut people_out = vec![];
        for (i, p) in people.iter().enumerate() {
            let origin = Origin::agent(&agent, &pid, p.note.as_deref());
            match s.create_person(&CTX, p.new.clone(), &origin) {
                Ok(created) => {
                    ids.insert(p.ref_.clone(), created.id.clone());
                    people_out.push(RefId { ref_: p.ref_.clone(), id: created.id });
                }
                Err(e) => {
                    fail = Some((format!("$.people[{i}]"), e.code(), e.to_string()));
                    return Err(e);
                }
            }
        }
        let mut rels_out = vec![];
        for (i, r) in rels.iter().enumerate() {
            let resolve = |e: &Endpoint| match e {
                Endpoint::Ref(r) => ids.get(r).cloned().unwrap_or_default(),
                Endpoint::Id(i) => i.clone(),
            };
            let mut new = r.new.clone();
            new.person_a = resolve(&r.a);
            new.person_b = resolve(&r.b);
            let origin = Origin::agent(&agent, &pid, r.note.as_deref());
            match s.add_relationship(&CTX, new, &origin) {
                Ok(created) => rels_out.push(IndexId { index: i, id: created.id }),
                Err(e) => {
                    fail = Some((format!("$.relationships[{i}]"), e.code(), e.to_string()));
                    return Err(e);
                }
            }
        }
        let mut changes_out = vec![];
        for (i, c) in changes.iter().enumerate() {
            match s.insert_change(&CTX, &pid, &c.target_type, &c.target_id, c.op, &c.patch, Some(&c.base_updated_at), c.note.as_deref()) {
                Ok(id) => changes_out.push(IndexId { index: i, id }),
                Err(e) => {
                    fail = Some((format!("$.changes[{i}]"), e.code(), e.to_string()));
                    return Err(e);
                }
            }
        }
        Ok((pid, people_out, rels_out, changes_out))
    });
    match out {
        Ok((proposal_id, people, relationships, changes)) => {
            let warnings = proposal_warnings(store, &proposal_id).unwrap_or_default();
            Ok(SubmitResult {
                proposal_id,
                status: "pending",
                people,
                relationships,
                changes,
                warnings,
                note: "Nothing is final until the user approves this proposal in the app. Do not assume any of it was applied.",
            })
        }
        Err(e) => match fail {
            Some((path, code, msg)) => Err(ReviewError::Invalid(vec![FieldError::new(path, code, msg)])),
            None => Err(e.into()),
        },
    }
}
