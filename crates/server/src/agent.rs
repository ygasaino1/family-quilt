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

#[derive(Deserialize)]
struct PeopleQ {
    q: Option<String>,
    limit: Option<usize>,
    pending: Option<u8>,
}

async fn people(State(st): State<Shared>, Extension(a): Extension<AgentName>, Query(q): Query<PeopleQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "searched people");
    ops::search_people(&st, q.q.as_deref().unwrap_or(""), q.limit.unwrap_or(50), q.pending == Some(1)).map(Json)
}

#[derive(Deserialize)]
struct DirQ {
    status: Option<String>,
    name: Option<String>,
    sex: Option<String>,
    no_parents: Option<bool>,
    no_children: Option<bool>,
    no_partners: Option<bool>,
    created_by_proposal: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
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
        offset: q.offset.unwrap_or(0),
        limit: q.limit.unwrap_or(0),
    };
    ops::list_people(&st, &o).map(Json)
}

async fn proposal(State(st): State<Shared>, Extension(a): Extension<AgentName>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "read a proposal");
    ops::get_proposal(&st, name(&a), &id).map(Json)
}

#[derive(Deserialize)]
struct HistQ {
    history: Option<String>,
}

async fn person(State(st): State<Shared>, Extension(a): Extension<AgentName>, Path(id): Path<String>, Query(q): Query<HistQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "read a person");
    ops::get_person_history(&st, &id, q.history.as_deref() == Some("all")).map(Json)
}

#[derive(Deserialize)]
struct RelQ {
    a: Option<String>,
    b: Option<String>,
    pending: Option<u8>,
}

async fn relationship(State(st): State<Shared>, Extension(ag): Extension<AgentName>, Query(q): Query<RelQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&ag), "asked for a relationship");
    let (Some(a), Some(b)) = (q.a, q.b) else { return Err(ApiError::bad_request("`a` and `b` (person ids) are required")) };
    ops::find_relationship(&st, &a, &b, q.pending == Some(1)).map(Json)
}

#[derive(Deserialize)]
struct GraphQ {
    focus: Option<String>,
    up: Option<u32>,
    down: Option<u32>,
    pending: Option<u8>,
}

async fn graph(State(st): State<Shared>, Extension(a): Extension<AgentName>, Query(q): Query<GraphQ>) -> Result<Json<Value>, ApiError> {
    st.log_activity(name(&a), "read a graph");
    let Some(focus) = q.focus else { return Err(ApiError::bad_request("`focus` (a person id) is required")) };
    ops::get_graph(&st, &focus, q.up.unwrap_or(2), q.down.unwrap_or(2), q.pending == Some(1)).map(Json)
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

async fn list(State(st): State<Shared>, Extension(a): Extension<AgentName>) -> Result<Json<Value>, ApiError> {
    ops::list_my_proposals(&st, name(&a)).map(Json)
}
