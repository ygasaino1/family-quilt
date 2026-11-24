//! `/api/*`: the browser UI's door. Human edits go straight to the store (every one writes a
//! revision); agent proposals are reviewed here. Guarded by `Sec-Fetch-Site` + `Host` in `lib.rs`.

use crate::{ops, ApiError, Shared};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use family_review::apply::{self, ApproveRequest};
use family_review::fields::{self, apply_name, apply_person, apply_relationship};
use family_review::{query, review};
use family_store::{AccessContext, FuzzyDate, LinkKind, NewName, NewPerson, NewRelationship, Origin, PartialDate, PartnershipState, Qualifier, RelType, Sex, Store, StoreError};
use serde::Deserialize;
use serde_json::{json, Value};

const CTX: AccessContext = AccessContext::Everything;
const HUMAN: &str = "human";

fn lock(st: &Shared) -> std::sync::MutexGuard<'_, Store> {
    st.store.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/status", get(status))
        .route("/quilt", get(crate::quilt::get_quilt))
        .route("/quilt/progress", get(crate::quilt::progress))
        .route("/quilt/cancel", post(crate::quilt::cancel))
        .route("/proposals", get(proposals))
        .route("/proposals/{id}", get(proposal))
        .route("/proposals/{id}/approve", post(approve))
        .route("/proposals/{id}/reject", post(reject))
        .route("/proposals/{id}/edit", post(edit_item))
        .route("/people", get(people).post(create_person))
        .route("/people/{id}", get(person).patch(patch_person).delete(delete_person))
        .route("/names/{id}", patch(patch_name))
        .route("/relationships", post(create_link))
        .route("/relationships/{id}", patch(patch_link).delete(delete_link))
        .route("/relationship", get(relationship))
        .route("/history", get(history))
        .route("/history/{id}/undo", post(undo))
        .route("/backup", get(backup))
        .route("/export/json", get(export_json))
        .route("/export/gedcom", get(export_gedcom))
        .route("/import/gedcom", post(import_gedcom).layer(axum::extract::DefaultBodyLimit::max(130 * 1024 * 1024)))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/settings/agent", get(agent_settings))
        .route("/settings/agent/test", post(agent_test))
        .layer(axum::extract::DefaultBodyLimit::max(2_000_000))
}

async fn status(State(st): State<Shared>) -> Result<Json<Value>, ApiError> {
    let s = lock(&st);
    Ok(Json(json!({"version": crate::VERSION, "protected": s.is_protected(), "pending_proposals": s.count_proposals(&CTX, "pending")?, "data_key": s.data_key(&CTX)?})))
}

#[derive(Deserialize)]
struct ProposalsQ {
    status: Option<String>,
}

async fn proposals(State(st): State<Shared>, Query(q): Query<ProposalsQ>) -> Result<Json<Value>, ApiError> {
    let s = lock(&st);
    let list = s.list_proposals(&CTX, q.status.as_deref(), None, 200)?;
    Ok(Json(json!({"proposals": list, "pending_count": s.count_proposals(&CTX, "pending")?})))
}

async fn proposal(State(st): State<Shared>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let v = review::review(&lock(&st), &id)?;
    Ok(Json(serde_json::to_value(v).unwrap_or_default()))
}

async fn approve(State(st): State<Shared>, Path(id): Path<String>, body: Option<Json<ApproveRequest>>) -> Result<Json<Value>, ApiError> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let r = apply::approve(&mut lock(&st), &id, &req, HUMAN)?;
    Ok(Json(serde_json::to_value(r).unwrap_or_default()))
}

#[derive(Deserialize, Default)]
struct RejectReq {
    /// `wrong` (fix it and send again), `duplicate`, `not_wanted` (never again) or `not_now`.
    reason: Option<String>,
    note: Option<String>,
}

async fn reject(State(st): State<Shared>, Path(id): Path<String>, body: Option<Json<RejectReq>>) -> Result<Json<Value>, ApiError> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let code = req.reason.as_deref().filter(|c| ["wrong", "duplicate", "not_wanted", "not_now"].contains(c));
    let text = req.note.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let note = match (code, text) {
        (Some(c), Some(t)) => Some(format!("{c}: {t}")),
        (Some(c), None) => Some(c.to_string()),
        (None, t) => t.map(str::to_string),
    };
    let r = apply::reject_with_note(&mut lock(&st), &id, HUMAN, note.as_deref())?;
    Ok(Json(json!({"ok": true, "affected_proposals": r.affected_proposals})))
}

#[derive(Deserialize)]
struct PeopleQ {
    q: Option<String>,
    limit: Option<usize>,
}

async fn people(State(st): State<Shared>, Query(q): Query<PeopleQ>) -> Result<Json<Value>, ApiError> {
    ops::search_people(&st, q.q.as_deref().unwrap_or(""), q.limit.unwrap_or(50), true).map(Json)
}

async fn person(State(st): State<Shared>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    ops::get_person(&st, &id).map(Json)
}

fn s(b: &Value, k: &str) -> Result<Option<String>, ApiError> {
    match b.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(x)) => Ok(Some(x.clone())),
        Some(_) => Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "wrong_type", &format!("{k} must be text"))),
    }
}

fn fuzzy(b: &Value, d: &str, q: &str) -> Result<Option<FuzzyDate>, ApiError> {
    let Some(date) = s(b, d)?.filter(|x| !x.trim().is_empty()) else { return Ok(None) };
    let qual = s(b, q)?.map(|x| Qualifier::parse(&x)).transpose()?.unwrap_or_default();
    Ok(Some(FuzzyDate::parse(date.trim(), qual)?))
}

async fn create_person(State(st): State<Shared>, Json(b): Json<Value>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let name = NewName { given: s(&b, "given_name")?, family: s(&b, "family_name")?, full_name: s(&b, "full_name")?, ..Default::default() };
    let new = NewPerson {
        sex: s(&b, "sex")?.map(|x| Sex::parse(&x)).transpose()?.unwrap_or_default(),
        is_living: b.get("is_living").and_then(Value::as_bool),
        birth: fuzzy(&b, "birth_date", "birth_qualifier")?,
        birth_place: s(&b, "birth_place")?,
        death: fuzzy(&b, "death_date", "death_qualifier")?,
        death_place: s(&b, "death_place")?,
        note: s(&b, "note")?,
        names: vec![name],
    };
    let p = lock(&st).create_person(&CTX, new, &Origin::human())?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(p).unwrap_or_default())))
}

/// Body: `{"field": value, ...}` using the same field names as agent changes.
fn field_pairs<'a>(b: &'a Value, allowed: &[&str], rel: Option<RelType>) -> Result<Vec<(&'a str, &'a Value)>, ApiError> {
    let o = b.as_object().ok_or_else(|| ApiError::bad_request("send an object of fields"))?;
    let mut out = vec![];
    let mut errs = vec![];
    for (k, v) in o {
        if !allowed.contains(&k.as_str()) {
            errs.push(family_review::FieldError::new(k.clone(), "field_not_editable", format!("'{k}' cannot be edited; editable: {}", allowed.join(", "))));
        } else if let Err(m) = fields::check_to(k, rel, v) {
            errs.push(family_review::FieldError::new(k.clone(), "invalid_value", m));
        } else {
            out.push((k.as_str(), v));
        }
    }
    if errs.is_empty() {
        Ok(out)
    } else {
        Err(family_review::ReviewError::Invalid(errs).into())
    }
}

async fn patch_person(State(st): State<Shared>, Path(id): Path<String>, Json(b): Json<Value>) -> Result<Json<Value>, ApiError> {
    let pairs = field_pairs(&b, fields::PERSON_FIELDS, None)?;
    let mut s = lock(&st);
    apply_person(&mut s, &CTX, &id, &pairs, HUMAN)?;
    Ok(Json(serde_json::to_value(s.get_person(&CTX, &id)?).unwrap_or_default()))
}

async fn delete_person(State(st): State<Shared>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let n = lock(&st).soft_delete_person(&CTX, &id, HUMAN)?;
    Ok(Json(json!({"ok": true, "links_removed": n})))
}

async fn create_link(State(st): State<Shared>, Json(b): Json<Value>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let t = RelType::parse(&s(&b, "type")?.unwrap_or_default())?;
    let (a, c) = (s(&b, "a")?.unwrap_or_default(), s(&b, "b")?.unwrap_or_default());
    let date = |k: &str| -> Result<Option<PartialDate>, ApiError> { Ok(s(&b, k)?.filter(|x| !x.trim().is_empty()).map(|x| PartialDate::parse(x.trim())).transpose()?) };
    let new = NewRelationship {
        rel_type: t,
        person_a: a,
        person_b: c,
        kind: s(&b, "kind")?.map(|x| LinkKind::parse(&x)).transpose()?,
        subtype: s(&b, "subtype")?.map(|x| PartnershipState::parse(&x)).transpose()?,
        start_date: date("start_date")?,
        end_date: date("end_date")?,
        place: s(&b, "place")?,
        note: s(&b, "note")?,
    };
    let r = lock(&st).add_relationship(&CTX, new, &Origin::human())?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(r).unwrap_or_default())))
}

async fn patch_link(State(st): State<Shared>, Path(id): Path<String>, Json(b): Json<Value>) -> Result<Json<Value>, ApiError> {
    let mut s = lock(&st);
    let cur = s.get_relationship(&CTX, &id)?;
    let pairs = field_pairs(&b, fields::allowed_fields("relationship", Some(cur.rel_type)), Some(cur.rel_type))?;
    apply_relationship(&mut s, &CTX, &id, &pairs, HUMAN)?;
    Ok(Json(serde_json::to_value(s.get_relationship(&CTX, &id)?).unwrap_or_default()))
}

async fn delete_link(State(st): State<Shared>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    lock(&st).remove_relationship(&CTX, &id, HUMAN)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct RelQ {
    a: Option<String>,
    b: Option<String>,
    pending: Option<u8>,
}

async fn relationship(State(st): State<Shared>, Query(q): Query<RelQ>) -> Result<Json<Value>, ApiError> {
    let (Some(a), Some(b)) = (q.a, q.b) else { return Err(ApiError::bad_request("`a` and `b` are required")) };
    ops::find_relationship(&st, &a, &b, q.pending == Some(1)).map(Json)
}

#[derive(Deserialize)]
struct HistQ {
    target_type: Option<String>,
    target_id: Option<String>,
    limit: Option<usize>,
}

async fn history(State(st): State<Shared>, Query(q): Query<HistQ>) -> Result<Json<Value>, ApiError> {
    let target = q.target_type.as_deref().zip(q.target_id.as_deref());
    let revs = lock(&st).recent_revisions(&CTX, target, q.limit.unwrap_or(50))?;
    Ok(Json(json!({ "revisions": revs })))
}

async fn undo(State(st): State<Shared>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    lock(&st).undo_revision(&CTX, &id, HUMAN)?;
    Ok(Json(json!({"ok": true})))
}

/// A consistent copy of the whole tree as one file (`VACUUM INTO`).
async fn backup(State(st): State<Shared>) -> Result<Response, ApiError> {
    let tmp = std::env::temp_dir().join(format!("family-backup-{}.tmp", family_store::ids::new_id()));
    let bytes = {
        let s = lock(&st);
        s.backup_to(&tmp)?;
        let b = std::fs::read(&tmp).map_err(StoreError::from);
        let _ = std::fs::remove_file(&tmp);
        b?
    };
    let name = format!("family-tree-backup-{}.sqlite", &family_store::ids::now_iso()[..10]);
    Ok(([(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], bytes).into_response())
}

/// Settings → "AI assistant (advanced)": ready-to-copy snippets and the last agent activity.
async fn agent_settings(State(st): State<Shared>) -> Json<Value> {
    let port = st.port.load(std::sync::atomic::Ordering::Relaxed);
    let url = format!("http://127.0.0.1:{port}/agent/mcp");
    let exe = st.exe.to_string_lossy().to_string();
    let desktop = json!({"mcpServers": {"family-tree": {"command": exe, "args": ["--mcp-stdio"]}}});
    let activity: Vec<_> = st.activity.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
    Json(json!({
        "token": st.token,
        "api_base": format!("http://127.0.0.1:{port}/agent/v1"),
        "mcp_url": url,
        "claude_desktop_config": serde_json::to_string_pretty(&desktop).unwrap_or_default(),
        "claude_code_http": format!("claude mcp add --transport http family-tree {url} --header \"Authorization: Bearer {}\"", st.token),
        "claude_code_stdio": format!("claude mcp add --transport stdio family-tree -- \"{exe}\" --mcp-stdio"),
        "note": "The app must be open for agents to reach the tree.",
        "last_activity": activity,
    }))
}

/// "Test connection": call the MCP endpoint the same way a client would and list the tools.
async fn agent_test(State(st): State<Shared>) -> Json<Value> {
    let tools = crate::mcp::tool_names();
    st.log_activity("settings", "connection test");
    let n = query::search(&lock(&st), "a", 1, false).map(|v| v.len()).is_ok();
    Json(json!({"ok": n, "tools": tools}))
}

/// Settings kept in the file itself (the rest, such as theme and text size, stays in the browser).
const FILE_SETTINGS: &[&str] = &["tree_title"];

async fn get_settings(State(st): State<Shared>) -> Result<Json<Value>, ApiError> {
    let s = lock(&st);
    let mut o = serde_json::Map::new();
    for k in FILE_SETTINGS {
        o.insert((*k).into(), json!(s.meta_get(&CTX, &format!("setting:{k}"))?.unwrap_or_default()));
    }
    Ok(Json(Value::Object(o)))
}

async fn put_settings(State(st): State<Shared>, Json(b): Json<Value>) -> Result<Json<Value>, ApiError> {
    let o = b.as_object().ok_or_else(|| ApiError::bad_request("send an object"))?;
    let mut s = lock(&st);
    for (k, v) in o {
        if !FILE_SETTINGS.contains(&k.as_str()) {
            return Err(ApiError::bad_request(&format!("unknown setting '{k}'")));
        }
        let text = v.as_str().ok_or_else(|| ApiError::bad_request("settings are text"))?;
        if text.chars().count() > 200 {
            return Err(ApiError::bad_request("too long (200 characters at most)"));
        }
        s.meta_set(&CTX, &format!("setting:{k}"), text)?;
    }
    Ok(Json(json!({"ok": true})))
}

/// Everything verified, as one JSON file people can keep or process elsewhere.
async fn export_json(State(st): State<Shared>) -> Result<Response, ApiError> {
    let (people, links, title) = {
        let s = lock(&st);
        (s.all_people(&CTX, false)?, s.all_relationships(&CTX, false)?, s.meta_get(&CTX, "setting:tree_title")?.unwrap_or_default())
    };
    let body = serde_json::to_vec_pretty(&json!({"format": "family-tree-export", "version": 1, "title": title, "exported_at": family_store::ids::now_iso(), "people": people, "relationships": links}))
        .unwrap_or_default();
    let name = format!("family-tree-{}.json", &family_store::ids::now_iso()[..10]);
    Ok(([(header::CONTENT_TYPE, "application/json".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], body).into_response())
}

/// GEDCOM text in the request body; people and links are added as verified records in one transaction.
async fn import_gedcom(State(st): State<Shared>, body: axum::body::Bytes) -> Result<Json<Value>, ApiError> {
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        let plan =
            family_gedcom::import_gedcom(std::io::Cursor::new(&body[..]), &family_gedcom::Limits::default()).map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.code(), &e.to_string()))?;
        let report = plan.apply(&mut lock(&st2), &CTX, &Origin::human())?;
        let warnings: Vec<Value> = plan.warnings.iter().take(50).map(|w| json!({"line": w.line, "code": w.code, "message": w.message})).collect();
        Ok(Json(json!({"people": report.person_ids.len(), "links": report.links_created, "warnings": warnings, "warning_count": plan.warnings.len()})))
    })
    .await
    .map_err(|_| ApiError::bad_request("import task failed"))?
}

async fn export_gedcom(State(st): State<Shared>) -> Result<Response, ApiError> {
    let out = family_gedcom::export_gedcom(&lock(&st), &CTX, family_gedcom::GedcomVersion::V551, false)?;
    let name = format!("family-tree-{}.ged", &family_store::ids::now_iso()[..10]);
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], out).into_response())
}

async fn patch_name(State(st): State<Shared>, Path(id): Path<String>, Json(b): Json<Value>) -> Result<Json<Value>, ApiError> {
    let pairs = field_pairs(&b, fields::NAME_FIELDS, None)?;
    let mut s = lock(&st);
    let (person_id, _) = s.find_name(&CTX, &id)?;
    apply_name(&mut s, &CTX, &id, &pairs, HUMAN)?;
    Ok(Json(serde_json::to_value(s.get_person(&CTX, &person_id)?).unwrap_or_default()))
}

/// Correct a value in a pending proposal before approving it.
async fn edit_item(State(st): State<Shared>, Path(id): Path<String>, Json(req): Json<family_review::edit::EditRequest>) -> Result<Json<Value>, ApiError> {
    let edits = family_review::edit::edit(&mut lock(&st), &id, &req, HUMAN)?;
    let view = review::review(&lock(&st), &id)?;
    Ok(Json(json!({"edits": edits, "review": view})))
}
