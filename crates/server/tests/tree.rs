//! First run, lock and unlock, passwords, recovery (spec sections 4 and 10), through the real HTTP door.

use family_server::{serve, tree, AppState, Shared};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering;

const TOKEN: &str = "test-token-123";

struct Srv {
    port: u16,
    st: Shared,
    dir: tempfile::TempDir,
    _h: tokio::task::JoinHandle<()>,
}

async fn start() -> Srv {
    let dir = tempfile::tempdir().unwrap();
    let st = AppState::new_closed(TOKEN.into(), dir.path().to_path_buf(), dir.path().join("tree.sqlite"), true);
    let (port, h) = serve(st.clone(), 0).await.unwrap();
    Srv { port, st, dir, _h: h }
}

fn http(port: u16, method: &str, path: &str, extra: &[(&str, &str)], body: Option<&Value>) -> (u16, Value) {
    let mut r = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\nHost: 127.0.0.1:{port}\r\n");
    for (k, v) in extra {
        r.push_str(&format!("{k}: {v}\r\n"));
    }
    match body {
        Some(b) => {
            let b = b.to_string();
            r.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", b.len()));
        }
        None => r.push_str("\r\n"),
    }
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(r.as_bytes()).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (head.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0), serde_json::from_str(body).unwrap_or(Value::Null))
}

fn ui(p: u16, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    http(p, method, path, &[("Sec-Fetch-Site", "same-origin")], body.as_ref())
}

fn agent(p: u16, path: &str) -> (u16, Value) {
    http(p, "GET", path, &[("Authorization", &format!("Bearer {TOKEN}"))], None)
}

fn code(v: &Value) -> &str {
    v["error"]["code"].as_str().unwrap_or("")
}

#[tokio::test(flavor = "multi_thread")]
async fn first_run_then_lock_unlock_with_a_password() {
    let s = start().await;
    let p = s.port;
    // Nothing exists yet: the wizard's state, and every data route is refused on both doors.
    let (c, v) = ui(p, "GET", "/api/tree", None);
    assert_eq!((c, v["state"].as_str()), (200, Some("missing")));
    assert_eq!(ui(p, "GET", "/api/people", None).0, 423);
    assert_eq!(code(&ui(p, "GET", "/api/status", None).1), "locked");
    assert_eq!(agent(p, "/agent/v1/guide").0, 423);

    let (c, v) = ui(p, "POST", "/api/tree/create", Some(json!({"title": "The Roots", "password": "correct horse"})));
    assert_eq!(c, 200, "{v}");
    let rk = v["recovery_key"].as_str().expect("the recovery key is shown once").to_string();
    assert!(rk.len() > 20);
    let (_, v) = ui(p, "GET", "/api/tree", None);
    assert_eq!((v["state"].as_str(), v["protected"].as_bool(), v["title"].as_str()), (Some("open"), Some(true), Some("The Roots")));
    // The file is not a plain SQLite file.
    let head = std::fs::read(s.dir.path().join("tree.sqlite")).unwrap();
    assert_ne!(&head[..16], b"SQLite format 3\0");
    // A second create is refused.
    assert_eq!(ui(p, "POST", "/api/tree/create", Some(json!({}))).0, 409);

    let (c, made) = ui(p, "POST", "/api/people", Some(json!({"given_name": "Ada", "family_name": "Root", "sex": "F"})));
    assert!(c == 200 || c == 201, "{made}");
    // Lock: both doors refuse, the state hides the title, the quilt cache is gone.
    assert_eq!(ui(p, "POST", "/api/tree/lock", None).0, 200);
    assert_eq!(ui(p, "GET", "/api/people?q=Ada", None).0, 423);
    assert_eq!(agent(p, "/agent/v1/people?q=Ada").0, 423);
    let (_, v) = ui(p, "GET", "/api/tree", None);
    assert_eq!((v["state"].as_str(), v["title"].as_str()), (Some("locked"), Some("")));
    // Wrong password, then right.
    let (c, v) = ui(p, "POST", "/api/tree/unlock", Some(json!({"password": "nope"})));
    assert_eq!((c, code(&v)), (423, "wrong_password"));
    assert_eq!(ui(p, "GET", "/api/people", None).0, 423, "still locked");
    assert_eq!(ui(p, "POST", "/api/tree/unlock", Some(json!({"password": "correct horse"}))).0, 200);
    let (c, v) = ui(p, "GET", "/api/people?q=Ada", None);
    assert_eq!(c, 200);
    assert_eq!(v["people"].as_array().map(|a| a.len()), Some(1), "the data survived the lock");
    assert_eq!(agent(p, "/agent/v1/people?q=Ada").0, 200);

    // Forgot the password: lock, reset with the recovery key, the old one stops working.
    ui(p, "POST", "/api/tree/lock", None);
    let (c, v) = ui(p, "POST", "/api/tree/reset", Some(json!({"recovery_key": "AAAA-BBBB", "new_password": "x"})));
    assert!(c >= 400, "{v}");
    assert_eq!(ui(p, "POST", "/api/tree/reset", Some(json!({"recovery_key": rk, "new_password": "second password"}))).0, 200);
    assert_eq!(ui(p, "GET", "/api/status", None).0, 200, "the reset opens the tree");
    ui(p, "POST", "/api/tree/lock", None);
    assert_eq!(ui(p, "POST", "/api/tree/unlock", Some(json!({"password": "correct horse"}))).0, 423);
    assert_eq!(ui(p, "POST", "/api/tree/unlock", Some(json!({"password": "second password"}))).0, 200);
    // The recovery key itself also unlocks.
    ui(p, "POST", "/api/tree/lock", None);
    assert_eq!(ui(p, "POST", "/api/tree/unlock", Some(json!({"recovery_key": rk}))).0, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unprotected_tree_can_get_a_password_change_it_and_lose_it_again() {
    let s = start().await;
    let p = s.port;
    assert_eq!(ui(p, "POST", "/api/tree/create", Some(json!({"title": "Plain"}))).1["recovery_key"], Value::Null);
    ui(p, "POST", "/api/people", Some(json!({"given_name": "Bo", "family_name": "Plain", "sex": "M"})));
    assert_eq!(tree::open_if_plain(&s.st).unwrap(), family_store::TreeState::Plain);

    let (c, v) = ui(p, "POST", "/api/tree/protect", Some(json!({"new": "first pass"})));
    assert_eq!(c, 200, "{v}");
    assert!(v["recovery_key"].as_str().is_some());
    assert_eq!(ui(p, "GET", "/api/people?q=Bo", None).1["people"].as_array().map(|a| a.len()), Some(1), "still open after the change");
    assert_eq!(ui(p, "GET", "/api/tree", None).1["protected"], json!(true));

    // A wrong current password changes nothing and keeps the tree open.
    let (c, v) = ui(p, "POST", "/api/tree/password", Some(json!({"current": "wrong", "new": "second pass"})));
    assert_eq!((c, code(&v)), (423, "wrong_password"));
    assert_eq!(ui(p, "GET", "/api/status", None).0, 200);
    assert_eq!(ui(p, "POST", "/api/tree/password", Some(json!({"current": "first pass", "new": "second pass"}))).0, 200);
    assert_eq!(ui(p, "GET", "/api/status", None).0, 200);
    ui(p, "POST", "/api/tree/lock", None);
    assert_eq!(ui(p, "POST", "/api/tree/unlock", Some(json!({"password": "first pass"}))).0, 423);
    assert_eq!(ui(p, "POST", "/api/tree/unlock", Some(json!({"password": "second pass"}))).0, 200);

    let (c, _) = ui(p, "POST", "/api/tree/unprotect", Some(json!({"current": "wrong"})));
    assert_eq!(c, 423);
    assert_eq!(ui(p, "POST", "/api/tree/unprotect", Some(json!({"current": "second pass"}))).0, 200);
    assert_eq!(ui(p, "GET", "/api/tree", None).1["protected"], json!(false));
    assert_eq!(ui(p, "GET", "/api/people?q=Bo", None).1["people"].as_array().map(|a| a.len()), Some(1));
    let head = std::fs::read(s.dir.path().join("tree.sqlite")).unwrap();
    assert_eq!(&head[..16], b"SQLite format 3\0");
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_locking_follows_the_setting_and_a_tree_with_no_setting_never_locks() {
    let s = start().await;
    let p = s.port;
    ui(p, "POST", "/api/tree/create", Some(json!({"password": "pw pw pw"})));
    assert!(!tree::lock_if_idle(&s.st));
    assert_eq!(ui(p, "POST", "/api/tree/auto-lock", Some(json!({"minutes": 5}))).0, 200);
    assert_eq!(ui(p, "GET", "/api/tree", None).1["auto_lock_minutes"], json!(5));
    assert!(!tree::lock_if_idle(&s.st), "not idle yet");
    // Polling does not count as activity; real requests do.
    *s.st.last_active.lock().unwrap() = std::time::Instant::now() - std::time::Duration::from_secs(4 * 60);
    ui(p, "GET", "/api/status", None);
    *s.st.last_active.lock().unwrap() = std::time::Instant::now() - std::time::Duration::from_secs(6 * 60);
    assert!(tree::lock_if_idle(&s.st));
    assert!(s.st.locked.load(Ordering::SeqCst));
    assert_eq!(ui(p, "GET", "/api/people", None).0, 423);
    // The setting is in the file: it is back after unlocking.
    ui(p, "POST", "/api/tree/unlock", Some(json!({"password": "pw pw pw"})));
    assert_eq!(ui(p, "GET", "/api/tree", None).1["auto_lock_minutes"], json!(5));
}

/// Send raw headers and return the status and the lower-cased header block.
fn head(port: u16, path: &str, extra: &[(&str, &str)]) -> (u16, String) {
    let mut r = format!("GET {path} HTTP/1.1\r\nConnection: close\r\nHost: 127.0.0.1:{port}\r\n");
    for (k, v) in extra {
        r.push_str(&format!("{k}: {v}\r\n"));
    }
    r.push_str("\r\n");
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(r.as_bytes()).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let text = String::from_utf8_lossy(&buf).to_lowercase();
    let h = text.split("\r\n\r\n").next().unwrap_or("").to_string();
    (h.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0), h)
}

#[tokio::test(flavor = "multi_thread")]
async fn with_a_launch_token_only_the_window_that_was_handed_it_reaches_the_ui_door() {
    let dir = tempfile::tempdir().unwrap();
    let st = AppState::new_closed(TOKEN.into(), dir.path().to_path_buf(), dir.path().join("tree.sqlite"), true);
    st.ui_token.set("launch-secret-123".into()).unwrap();
    let (p, _h) = serve(st.clone(), 0).await.unwrap();
    let same = [("Sec-Fetch-Site", "same-origin")];
    // A program that forges the browser headers still has no cookie.
    let (c, h) = head(p, "/api/tree", &same);
    assert_eq!(c, 403, "{h}");
    let (c, _) = head(p, "/api/tree", &[("Sec-Fetch-Site", "same-origin"), ("Cookie", "fq_ui=wrong")]);
    assert_eq!(c, 403);
    // The page itself is served to anyone (it holds no data); a wrong token hands out nothing.
    let (c, h) = head(p, "/?t=wrong", &[]);
    assert_eq!(c, 200);
    assert!(!h.contains("set-cookie"));
    // The right token hands out the cookie and redirects; with the cookie the door opens.
    let (c, h) = head(p, "/?t=launch-secret-123", &[]);
    assert_eq!(c, 303);
    assert!(h.contains("set-cookie: fq_ui=launch-secret-123") && h.contains("httponly") && h.contains("samesite=strict"), "{h}");
    let (c, _) = head(p, "/api/tree", &[("Sec-Fetch-Site", "same-origin"), ("Cookie", "other=1; fq_ui=launch-secret-123")]);
    assert_eq!(c, 200);
    // The agent door is unaffected (it has its own bearer token).
    assert_eq!(agent(p, "/agent/v1/guide").0, 423, "locked, but reachable with the agent token");
    let _ = (&same, &st);
}

#[test]
fn a_second_copy_is_noticed_through_the_endpoint_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!family_server::token::already_running(dir.path()));
    let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    family_server::token::write_endpoint(dir.path(), l.local_addr().unwrap().port()).unwrap();
    assert!(family_server::token::already_running(dir.path()));
    drop(l);
    assert!(!family_server::token::already_running(dir.path()), "a stale file from a crash does not block the start");
}

#[tokio::test(flavor = "multi_thread")]
async fn first_run_can_choose_another_folder_and_remembers_it() {
    let s = start().await;
    let p = s.port;
    let other = tempfile::tempdir().unwrap();
    let folder = other.path().join("my family");
    // Not absolute: refused.
    let (c, v) = ui(p, "POST", "/api/tree/location", Some(json!({"folder": "relative/dir"})));
    assert_eq!(c, 400, "{v}");
    // Absolute: created, remembered, used.
    let (c, v) = ui(p, "POST", "/api/tree/location", Some(json!({"folder": folder.to_str().unwrap()})));
    assert_eq!(c, 200, "{v}");
    assert!(folder.is_dir());
    let (_, st) = ui(p, "GET", "/api/tree", None);
    assert_eq!(st["state"], "missing");
    assert!(st["path"].as_str().unwrap().contains("my family"), "{st}");
    let (c, _) = ui(p, "POST", "/api/tree/create", Some(json!({"title": "T", "password": null})));
    assert_eq!(c, 200);
    assert!(folder.join("tree.sqlite").exists());
    assert!(!s.dir.path().join("tree.sqlite").exists());
    // The next start finds it through the remembered choice.
    assert_eq!(tree::saved_tree_path(s.dir.path()), folder.join("tree.sqlite"));
    // Once a tree is open the place cannot be changed.
    let (c, v) = ui(p, "POST", "/api/tree/location", Some(json!({"folder": other.path().to_str().unwrap()})));
    assert_eq!(c, 409, "{v}");
    // A remembered folder that vanished falls back to the default place.
    tree::lock_now(&s.st); // Windows cannot delete a file that is still open
    drop(other);
    assert_eq!(tree::saved_tree_path(s.dir.path()), s.dir.path().join("tree.sqlite"));
    // Without the window feature the dialog is not offered.
    let (c, v) = ui(p, "POST", "/api/tree/pick-folder", Some(json!({})));
    assert_eq!(c, 501, "{v}");
}
