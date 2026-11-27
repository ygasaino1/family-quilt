//! `/agent/v1/*`. Exactly these routes and no others: guide, people, person, relationship, graph,
//! create proposal, list own proposals.

use crate::{ops, AgentName, ApiError, Shared};
use axum::extract::{Extension, Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/guide", get(guide))
        .route("/people", get(people))
        .route("/people/{id}", get(person))
        .route("/directory", get(directory))
        .route("/guide/reading", get(guide_reading))
        .route("/proposals/{id}", get(proposal))
        .route("/relationship", get(relationship))
        .route("/graph", get(graph))
        .route("/proposals", get(list).post(create))
        .route("/proposals/{id}/withdraw", post(withdraw))
        .layer(axum::extract::DefaultBodyLimit::max(1_000_000))
}

fn name(e: &AgentName) -> &str {
    if e.0.is_empty() {
        "unnamed"
    } else {
        &e.0
    }
}

async fn guide(State(st): State<Shared>, Extension(a): Extension<AgentName>) -> Response {
    st.log_activity(name(&a), "read the guide");
    ([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], family_review::guide::GUIDE).into_response()
}

async fn guide_reading() -> Response {
    ([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], family_review::guide::GUIDE_READING).into_response()
}

/// `status` (verified, pending, all); none means verified.
fn scope(status: Option<String>) -> Result<family_review::query::Scope, ApiError> {
    Ok(family_review::query::Scope::parse(status.as_deref().unwrap_or(""))?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeopleQ {
    q: Option<String>,
    limit: Option<usize>,
    after: Option<String>,
    status: Option<String>,
}

async fn people(State(st): State<Shared>, Extension(a): Extension<AgentName>, Query(q): Query<PeopleQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "searched people");
    ops::search_people(&st, q.q.as_deref().unwrap_or(""), q.limit.unwrap_or(50), scope(q.status)?, q.after.as_deref()).map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirQ {
    status: Option<String>,
    name: Option<String>,
    sex: Option<String>,
    no_parents: Option<bool>,
    no_children: Option<bool>,
    no_partners: Option<bool>,
    created_by_proposal: Option<String>,
    after: Option<String>,
    limit: Option<usize>,
    fields: Option<String>,
    format: Option<String>,
    group_by: Option<String>,
}

async fn directory(State(st): State<Shared>, Extension(a): Extension<AgentName>, Query(q): Query<DirQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "listed people");
    let o = family_review::query::ListOpts {
        status: q.status.unwrap_or_default(),
        name: q.name,
        sex: q.sex,
        no_parents: q.no_parents.unwrap_or(false),
        no_children: q.no_children.unwrap_or(false),
        no_partners: q.no_partners.unwrap_or(false),
        created_by_proposal: q.created_by_proposal,
        after: q.after,
        limit: q.limit.unwrap_or(0),
        fields: q.fields.map(|f| f.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()),
        columnar: match q.format.as_deref() {
            None | Some("json") => false,
            Some("columns") => true,
            Some(o) => return Err(ApiError::bad_request(&format!("format '{o}' is not supported: use json or columns"))),
        },
        group_by: q.group_by,
    };
    let r = ops::list_people(&st, &o)?;
    if r["total"] == 0 {
        st.log_activity(name(&a), &format!("list_people matched nobody ({})", r["filters"]));
    }
    Ok(Json(r))
}

async fn proposal(State(st): State<Shared>, Extension(a): Extension<AgentName>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "read a proposal");
    ops::proposals(&st, name(&a), Some(&id), "", 0, None).map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistQ {
    history: Option<String>,
}

async fn person(State(st): State<Shared>, Extension(a): Extension<AgentName>, Path(id): Path<String>, Query(q): Query<HistQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "read a person");
    ops::get_person_history(&st, &id, q.history.as_deref() == Some("all")).map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelQ {
    a: Option<String>,
    b: Option<String>,
    status: Option<String>,
}

async fn relationship(State(st): State<Shared>, Extension(ag): Extension<AgentName>, Query(q): Query<RelQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&ag), "asked for a relationship");
    let (Some(a), Some(b)) = (q.a, q.b) else { return Err(ApiError::bad_request("`a` and `b` (person ids) are required")) };
    ops::find_relationship(&st, &a, &b, scope(q.status)?).map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQ {
    focus: Option<String>,
    up: Option<u32>,
    down: Option<u32>,
    status: Option<String>,
}

async fn graph(State(st): State<Shared>, Extension(a): Extension<AgentName>, Query(q): Query<GraphQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "read a graph");
    let Some(focus) = q.focus else { return Err(ApiError::bad_request("`focus` (a person id) is required")) };
    ops::get_graph(&st, &focus, q.up.unwrap_or(2), q.down.unwrap_or(2), scope(q.status)?).map(Json)
}

async fn create(State(st): State<Shared>, Extension(a): Extension<AgentName>, Json(body): Json<Value>) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    st.log_activity(name(&a), "sent a proposal");
    ops::create_proposal(&st, name(&a), &body).map(|v| (axum::http::StatusCode::CREATED, Json(v)))
}

#[derive(Deserialize, Default)]
struct WithdrawReq {
    reason: Option<String>,
}

async fn withdraw(State(st): State<Shared>, Extension(a): Extension<AgentName>, Path(id): Path<String>, body: Option<Json<WithdrawReq>>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "withdrew a proposal");
    let reason = body.and_then(|b| b.0.reason);
    ops::withdraw_proposal(&st, name(&a), &id, reason.as_deref()).map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalsQ {
    status: Option<String>,
    limit: Option<usize>,
    after: Option<String>,
}

async fn list(State(st): State<Shared>, Extension(a): Extension<AgentName>, Query(q): Query<ProposalsQ>) -> Result<Json<Value>, ApiError> {
    ops::proposals(&st, name(&a), None, q.status.as_deref().unwrap_or(""), q.limit.unwrap_or(0), q.after.as_deref()).map(Json)
}
