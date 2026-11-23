//! `family-server`: the localhost HTTP server.
//!
//! Two doors (spec section 4), one process, one SQLite file:
//! * `/api/*`  the browser UI. Needs `Sec-Fetch-Site: same-origin|none` and a localhost `Host`.
//! * `/agent/v1/*` and `/agent/mcp`  agents. Bearer token; read, search, relationship, graph,
//!   **create proposal**, list own proposals. No route exists for direct edit, delete, approve or reject.
//!
//! Every request, on every route, must carry a localhost `Host` (and, if present, a localhost
//! `Origin`): this blocks DNS-rebinding from other websites.

pub mod agent;
pub mod bridge;
pub mod error;
pub mod mcp;
pub mod ops;
pub mod quilt;
pub mod token;
pub mod tree;
pub mod ui;
pub mod web;
#[cfg(feature = "window")]
pub mod window;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use family_store::Store;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use error::ApiError;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, serde::Serialize)]
pub struct Activity {
    pub at: String,
    pub agent: String,
    pub what: String,
}

pub struct AppState {
    pub store: Mutex<Store>,
    pub token: String,
    pub data_dir: PathBuf,
    /// Path of this executable, for the bridge config snippet.
    pub exe: PathBuf,
    pub port: std::sync::atomic::AtomicU16,
    /// MCP sessions (2025 protocol revisions): session id -> client name.
    pub sessions: Mutex<HashMap<String, String>>,
    pub activity: Mutex<VecDeque<Activity>>,
    pub quilt: Mutex<quilt::QuiltState>,
    /// The tree file. While `locked` the `store` is an empty placeholder (see `tree`).
    pub db_path: PathBuf,
    pub locked: std::sync::atomic::AtomicBool,
    pub last_active: Mutex<std::time::Instant>,
    pub auto_lock_minutes: std::sync::atomic::AtomicU32,
    /// Key-derivation cost for new passwords (tests use cheap settings).
    pub kdf: family_store::KdfParams,
    /// Pause after a wrong password (tests set zero).
    pub wrong_password_delay: std::time::Duration,
    /// Set when the UI is shown in a window or browser: `/api/*` then also needs the cookie that `?t=` hands out, so
    /// another program on this computer cannot reach the UI door by forging a header. Unset for headless runs and tests.
    pub ui_token: std::sync::OnceLock<String>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    /// An already open tree (tests, tools).
    pub fn new(store: Store, token: String, data_dir: PathBuf) -> Shared {
        Self::build_state(store, token, data_dir.clone(), data_dir.join("tree.sqlite"), false)
    }

    /// No tree open yet: first run or locked. `tree::open_if_plain` opens an unprotected one.
    /// `fast_for_tests`: cheap key derivation and no pause after a wrong password. Never for real data.
    pub fn new_closed(token: String, data_dir: PathBuf, db_path: PathBuf, fast_for_tests: bool) -> Shared {
        let placeholder = Store::open_in_memory().expect("an in-memory store always opens");
        let mut st = Self::build_state(placeholder, token, data_dir, db_path, true);
        if fast_for_tests {
            let s = Arc::get_mut(&mut st).expect("not shared yet");
            s.kdf = family_store::KdfParams::FAST_FOR_TESTS;
            s.wrong_password_delay = std::time::Duration::ZERO;
        }
        st
    }

    fn build_state(store: Store, token: String, data_dir: PathBuf, db_path: PathBuf, locked: bool) -> Shared {
        Arc::new(AppState {
            db_path,
            locked: locked.into(),
            last_active: Mutex::new(std::time::Instant::now()),
            auto_lock_minutes: Default::default(),
            kdf: family_store::KdfParams::STANDARD,
            wrong_password_delay: std::time::Duration::from_millis(1000),
            ui_token: Default::default(),
            store: Mutex::new(store),
            token,
            data_dir,
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("family-app")),
            port: Default::default(),
            sessions: Default::default(),
            activity: Default::default(),
            quilt: Default::default(),
        })
    }

    pub fn log_activity(&self, agent: &str, what: &str) {
        let mut a = self.activity.lock().unwrap_or_else(|e| e.into_inner());
        a.push_front(Activity { at: family_store::ids::now_iso(), agent: agent.to_string(), what: what.to_string() });
        a.truncate(20);
    }
}

/// `127.0.0.1`, `localhost` or `[::1]`, optionally with a numeric port.
fn host_is_local(h: &str) -> bool {
    let (name, port) = match h.rsplit_once(':') {
        Some((n, p)) if !h.ends_with(']') && p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (n, true),
        _ => (h, false),
    };
    let _ = port;
    matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

fn origin_is_local(o: &str) -> bool {
    match o.strip_prefix("http://") {
        Some(rest) => host_is_local(rest.trim_end_matches('/')),
        None => false,
    }
}

fn deny(status: StatusCode, code: &str, msg: &str) -> Response {
    ApiError::new(status, code, msg).into_response_inner()
}

/// Applied to everything: localhost `Host`, and a localhost `Origin` when one is sent.
async fn host_guard(req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !host_is_local(host) {
        return deny(StatusCode::FORBIDDEN, "bad_host", "this server only answers requests addressed to localhost");
    }
    if let Some(o) = req.headers().get(header::ORIGIN) {
        if !o.to_str().map(origin_is_local).unwrap_or(false) {
            return deny(StatusCode::FORBIDDEN, "bad_origin", "requests from other websites are not allowed");
        }
    }
    next.run(req).await
}

/// `/api/*`: same-origin browser requests only. Missing header = not a browser request = refused.
async fn ui_guard(State(st): State<Shared>, req: Request, next: Next) -> Response {
    let site = req.headers().get("sec-fetch-site").and_then(|v| v.to_str().ok()).unwrap_or("");
    if site != "same-origin" && site != "none" {
        return deny(StatusCode::FORBIDDEN, "not_same_origin", "this address is only for the app's own window");
    }
    if let Some(tok) = st.ui_token.get() {
        if !web::has_ui_cookie(req.headers(), tok) {
            return deny(StatusCode::FORBIDDEN, "no_ui_token", "this address is only for the app's own window");
        }
    }
    next.run(req).await
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Who is calling the agent door (from `X-Agent-Name`), stored as a request extension.
#[derive(Clone)]
pub struct AgentName(pub String);

pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "))
}

async fn agent_guard(State(st): State<Shared>, mut req: Request, next: Next) -> Response {
    match bearer(req.headers()) {
        Some(t) if ct_eq(t.as_bytes(), st.token.as_bytes()) => {}
        _ => return deny(StatusCode::UNAUTHORIZED, "bad_token", "missing or wrong agent token (Authorization: Bearer <token>)"),
    }
    let name = req.headers().get("x-agent-name").and_then(|v| v.to_str().ok()).map(family_review::clean_agent_name);
    req.extensions_mut().insert(AgentName(name.unwrap_or_default()));
    next.run(req).await
}

pub fn router(state: Shared) -> Router {
    let agent = Router::new()
        .nest("/v1", agent::routes())
        .merge(mcp::routes())
        .layer(middleware::from_fn_with_state(state.clone(), tree::agent_lock_guard))
        .layer(middleware::from_fn_with_state(state.clone(), agent_guard));
    let data = ui::routes().layer(middleware::from_fn_with_state(state.clone(), tree::lock_guard));
    let api = data.merge(tree::routes()).layer(middleware::from_fn_with_state(state.clone(), ui_guard));
    Router::new().nest("/agent", agent).nest("/api", api).fallback(web::serve_asset).layer(middleware::from_fn(host_guard)).with_state(state)
}

/// Bind to a free localhost port, serve, and return the port. Used by the app and by tests.
pub async fn serve(state: Shared, port: u16) -> std::io::Result<(u16, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let port = listener.local_addr()?.port();
    state.port.store(port, std::sync::atomic::Ordering::Relaxed);
    let app = router(state.clone());
    let timer = tree::spawn_idle_timer(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
        timer.abort();
    });
    Ok((port, handle))
}
