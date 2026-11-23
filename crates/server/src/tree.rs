//! The tree's life cycle as the window sees it: first run, locked, open (spec sections 4 and 10).
//!
//! While no tree is open (first run, or locked) the shared `Store` is an **empty in-memory placeholder** and the
//! lock guard refuses every data route on both doors with `423 locked`. Locking drops the real store, which closes
//! the file and zeroes the key; the placeholder holds nothing, so even a missed guard would leak nothing.

use crate::{ApiError, Shared};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use family_store::{change_password, protect_tree, reset_password_with_recovery_key, tree_state, unprotect_tree, AccessContext, Credentials, OpenOptions, Protection, Store, TreeState};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const CTX: AccessContext = AccessContext::Everything;
const AUTO_LOCK_KEY: &str = "setting:auto_lock_minutes";

fn placeholder() -> Store {
    Store::open_in_memory().expect("an in-memory store always opens")
}

fn locked_error() -> ApiError {
    ApiError::new(StatusCode::LOCKED, "locked", "The tree is locked. Unlock it in the app window first.")
}

fn opts(st: &Shared) -> OpenOptions {
    OpenOptions { wrong_password_delay: st.wrong_password_delay }
}

/// Make `store` the open tree.
fn install(st: &Shared, store: Store) {
    let minutes = store.meta_get(&CTX, AUTO_LOCK_KEY).ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0);
    st.auto_lock_minutes.store(minutes, Ordering::Relaxed);
    *st.last_active.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    *st.store.lock().unwrap_or_else(|e| e.into_inner()) = store;
    st.locked.store(false, Ordering::SeqCst);
}

/// Take the open tree out (closing it) and put the placeholder in. Returns the real store.
fn take(st: &Shared) -> Store {
    st.locked.store(true, Ordering::SeqCst);
    let old = std::mem::replace(&mut *st.store.lock().unwrap_or_else(|e| e.into_inner()), placeholder());
    // Cached pictures of the tree hold names: drop them with the key.
    *st.quilt.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
    old
}

/// Lock now (also used by the idle timer). Harmless if already locked.
pub fn lock_now(st: &Shared) {
    if st.locked.swap(true, Ordering::SeqCst) {
        return;
    }
    let old = take(st);
    let _ = old.close();
}

/// Open an unprotected tree without asking anything; a protected or missing one stays closed. Returns the new state.
pub fn open_if_plain(st: &Shared) -> Result<TreeState, ApiError> {
    let s = tree_state(&st.db_path)?;
    if s == TreeState::Plain && st.locked.load(Ordering::SeqCst) {
        install(st, Store::open_tree(&st.db_path, Credentials::None, &opts(st))?);
    }
    Ok(s)
}

/// Refuses everything while no tree is open; counts human activity for the idle timer.
pub async fn lock_guard(State(st): State<Shared>, req: Request, next: Next) -> Response {
    if st.locked.load(Ordering::SeqCst) {
        return locked_error().into_response_inner();
    }
    let p = req.uri().path();
    // The window polls these on a timer: they must not keep the tree unlocked.
    if p != "/status" && p != "/quilt/progress" {
        *st.last_active.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }
    next.run(req).await
}

/// Same refusal for the agent door (no activity counting: an agent must not hold the tree open).
pub async fn agent_lock_guard(State(st): State<Shared>, req: Request, next: Next) -> Response {
    if st.locked.load(Ordering::SeqCst) {
        return locked_error().into_response_inner();
    }
    next.run(req).await
}

/// Background timer: lock after the configured idle time.
pub fn spawn_idle_timer(st: Shared) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            lock_if_idle(&st);
        }
    })
}

/// Lock if the configured idle time has passed. Returns whether it locked.
pub fn lock_if_idle(st: &Shared) -> bool {
    let minutes = st.auto_lock_minutes.load(Ordering::Relaxed);
    if minutes == 0 || st.locked.load(Ordering::SeqCst) {
        return false;
    }
    let idle = st.last_active.lock().unwrap_or_else(|e| e.into_inner()).elapsed();
    if idle >= Duration::from_secs(u64::from(minutes) * 60) {
        lock_now(st);
        return true;
    }
    false
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/tree", get(state))
        .route("/tree/create", post(create))
        .route("/tree/unlock", post(unlock))
        .route("/tree/lock", post(lock))
        .route("/tree/reset", post(reset))
        .route("/tree/protect", post(protect))
        .route("/tree/unprotect", post(unprotect))
        .route("/tree/password", post(password))
        .route("/tree/auto-lock", post(auto_lock))
}

fn blocking_err(_: tokio::task::JoinError) -> ApiError {
    ApiError::bad_request("the task failed")
}

async fn state(State(st): State<Shared>) -> Result<Json<Value>, ApiError> {
    let on_disk = tree_state(&st.db_path)?;
    let open = !st.locked.load(Ordering::SeqCst);
    let (title, minutes) = if open {
        let s = st.store.lock().unwrap_or_else(|e| e.into_inner());
        (s.meta_get(&CTX, "setting:tree_title")?.unwrap_or_default(), st.auto_lock_minutes.load(Ordering::Relaxed))
    } else {
        (String::new(), 0)
    };
    let name = match (open, on_disk) {
        (true, _) => "open",
        (false, TreeState::Missing) => "missing",
        (false, _) => "locked",
    };
    Ok(Json(json!({"state": name, "protected": on_disk == TreeState::Protected, "title": title, "auto_lock_minutes": minutes, "path": st.db_path.display().to_string()})))
}

#[derive(Deserialize)]
struct CreateReq {
    #[serde(default)]
    title: String,
    /// Empty or missing: an unprotected tree.
    password: Option<String>,
}

/// First run: make the tree (protected or not), open it, and return the recovery key once.
async fn create(State(st): State<Shared>, Json(b): Json<CreateReq>) -> Result<Json<Value>, ApiError> {
    if tree_state(&st.db_path)? != TreeState::Missing {
        return Err(ApiError::new(StatusCode::CONFLICT, "exists", "A tree already exists here."));
    }
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        let pw = b.password.filter(|p| !p.is_empty());
        let (mut store, rk) = match &pw {
            Some(p) => Store::create_tree(&st2.db_path, Protection::Password { password: p, kdf: st2.kdf })?,
            None => Store::create_tree(&st2.db_path, Protection::None)?,
        };
        let title = b.title.trim().chars().take(200).collect::<String>();
        if !title.is_empty() {
            store.meta_set(&CTX, "setting:tree_title", &title)?;
        }
        install(&st2, store);
        Ok(Json(json!({"ok": true, "recovery_key": rk.as_ref().map(|r| r.as_str())})))
    })
    .await
    .map_err(blocking_err)?
}

#[derive(Deserialize)]
struct UnlockReq {
    password: Option<String>,
    recovery_key: Option<String>,
}

async fn unlock(State(st): State<Shared>, Json(b): Json<UnlockReq>) -> Result<Json<Value>, ApiError> {
    if !st.locked.load(Ordering::SeqCst) {
        return Ok(Json(json!({"ok": true})));
    }
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        let creds = match (&b.password, &b.recovery_key) {
            (Some(p), _) => Credentials::Password(p),
            (None, Some(r)) => Credentials::RecoveryKey(r),
            _ => Credentials::None,
        };
        let store = Store::open_tree(&st2.db_path, creds, &opts(&st2))?;
        install(&st2, store);
        Ok(Json(json!({"ok": true})))
    })
    .await
    .map_err(blocking_err)?
}

async fn lock(State(st): State<Shared>) -> Json<Value> {
    let st2 = st.clone();
    let _ = tokio::task::spawn_blocking(move || lock_now(&st2)).await;
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct ResetReq {
    recovery_key: String,
    new_password: String,
}

/// Forgot the password: the recovery key sets a new one, then the tree opens.
async fn reset(State(st): State<Shared>, Json(b): Json<ResetReq>) -> Result<Json<Value>, ApiError> {
    if !st.locked.load(Ordering::SeqCst) {
        return Err(ApiError::bad_request("the tree is already open"));
    }
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        reset_password_with_recovery_key(&st2.db_path, &b.recovery_key, &b.new_password, &opts(&st2), None)?;
        install(&st2, Store::open_tree(&st2.db_path, Credentials::Password(&b.new_password), &opts(&st2))?);
        Ok(Json(json!({"ok": true})))
    })
    .await
    .map_err(blocking_err)?
}

/// Run `f` on the file with our store closed, then reopen it with the password that is valid afterwards (`None` =
/// unprotected): `after_ok` if `f` worked, `after_err` (what was valid before) if it did not. The key never leaves
/// the process.
fn with_store_closed<T>(st: &Shared, after_ok: Option<&str>, after_err: Option<&str>, f: impl FnOnce() -> Result<T, family_store::StoreError>) -> Result<T, ApiError> {
    if st.locked.load(Ordering::SeqCst) {
        return Err(locked_error());
    }
    let old = take(st);
    let _ = old.close();
    let out = f();
    let pw = if out.is_ok() { after_ok } else { after_err };
    let creds = pw.map_or(Credentials::None, Credentials::Password);
    // If this fails the tree simply stays locked and the person unlocks it with the password they have.
    if let Ok(s) = Store::open_tree(&st.db_path, creds, &opts(st)) {
        install(st, s);
    }
    Ok(out?)
}

/// Check a password against the file while the tree is open (a wrong one is refused before anything is closed).
fn verify_password(st: &Shared, password: &str) -> Result<(), ApiError> {
    if st.locked.load(Ordering::SeqCst) {
        return Err(locked_error());
    }
    Store::open_tree(&st.db_path, Credentials::Password(password), &opts(st))?.close()?;
    Ok(())
}

#[derive(Deserialize)]
struct PasswordReq {
    #[serde(default)]
    current: String,
    #[serde(default)]
    new: String,
}

/// Add a password to an unprotected tree. Returns the recovery key once.
async fn protect(State(st): State<Shared>, Json(b): Json<PasswordReq>) -> Result<Json<Value>, ApiError> {
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        let rk = with_store_closed(&st2, Some(&b.new), None, || protect_tree(&st2.db_path, &b.new, st2.kdf, None))?;
        Ok(Json(json!({"ok": true, "recovery_key": rk.as_str()})))
    })
    .await
    .map_err(blocking_err)?
}

async fn unprotect(State(st): State<Shared>, Json(b): Json<PasswordReq>) -> Result<Json<Value>, ApiError> {
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        verify_password(&st2, &b.current)?;
        with_store_closed(&st2, None, Some(&b.current), || unprotect_tree(&st2.db_path, Credentials::Password(&b.current), &opts(&st2), None))?;
        Ok(Json(json!({"ok": true})))
    })
    .await
    .map_err(blocking_err)?
}

async fn password(State(st): State<Shared>, Json(b): Json<PasswordReq>) -> Result<Json<Value>, ApiError> {
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        verify_password(&st2, &b.current)?;
        with_store_closed(&st2, Some(&b.new), Some(&b.current), || change_password(&st2.db_path, &b.current, &b.new, &opts(&st2), None))?;
        Ok(Json(json!({"ok": true})))
    })
    .await
    .map_err(blocking_err)?
}

#[derive(Deserialize)]
struct AutoLockReq {
    minutes: u32,
}

async fn auto_lock(State(st): State<Shared>, Json(b): Json<AutoLockReq>) -> Result<Json<Value>, ApiError> {
    if b.minutes > 24 * 60 {
        return Err(ApiError::bad_request("at most 24 hours"));
    }
    if st.locked.load(Ordering::SeqCst) {
        return Err(locked_error());
    }
    st.store.lock().unwrap_or_else(|e| e.into_inner()).meta_set(&CTX, AUTO_LOCK_KEY, &b.minutes.to_string())?;
    st.auto_lock_minutes.store(b.minutes, Ordering::Relaxed);
    Ok(Json(json!({"ok": true})))
}
