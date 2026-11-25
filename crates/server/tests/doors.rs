//! Raw-HTTP tests (clients like reqwest set `Host` themselves, so we write the bytes).

use family_server::{bridge, serve, token, AppState, Shared};
use family_store::*;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;

const TOKEN: &str = "test-token-123";
const CTX: AccessContext = AccessContext::Everything;

struct Srv {
    port: u16,
    st: Shared,
    dir: tempfile::TempDir,
    _h: tokio::task::JoinHandle<()>,
}

async fn start() -> Srv {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in_memory().unwrap();
    for (n, s) in [("Gus Tester", Sex::Male), ("Dan Tester", Sex::Male)] {
        store.create_person(&CTX, NewPerson { sex: s, names: vec![NewName::full(n)], ..Default::default() }, &Origin::human()).unwrap();
    }
    let st = AppState::new(store, TOKEN.into(), dir.path().to_path_buf());
    let (port, h) = serve(st.clone(), 0).await.unwrap();
    Srv { port, st, dir, _h: h }
}

/// Send exact bytes; return (status, lowercase header block, body).
fn raw(port: u16, req: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
    (status, head.to_lowercase(), body.to_string())
}

fn req(method: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> String {
    let mut r = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        r.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        r.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", b.len()));
    } else {
        r.push_str("\r\n");
    }
    r
}

fn ui(p: u16, method: &str, path: &str, body: Option<&str>) -> (u16, Value) {
    let (s, _, b) = raw(p, &req(method, path, &[("Host", &format!("127.0.0.1:{p}")), ("Sec-Fetch-Site", "same-origin")], body));
    (s, serde_json::from_str(&b).unwrap_or(Value::Null))
}

fn agent(p: u16, method: &str, path: &str, name: &str, body: Option<&str>) -> (u16, Value) {
    let auth = format!("Bearer {TOKEN}");
    let (s, _, b) = raw(p, &req(method, path, &[("Host", &format!("127.0.0.1:{p}")), ("Authorization", &auth), ("X-Agent-Name", name)], body));
    (s, serde_json::from_str(&b).unwrap_or(Value::Null))
}

fn first_person_id(st: &Shared) -> String {
    st.store.lock().unwrap().all_people(&CTX, false).unwrap()[0].id.clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn api_door_rejects_forged_host_and_missing_or_foreign_fetch_site() {
    let s = start().await;
    let p = s.port;
    let host = format!("127.0.0.1:{p}");
    // No Sec-Fetch-Site (a script or curl): refused.
    assert_eq!(raw(p, &req("GET", "/api/status", &[("Host", &host)], None)).0, 403);
    // Other sites in a browser: refused.
    for site in ["cross-site", "same-site", ""] {
        assert_eq!(raw(p, &req("GET", "/api/status", &[("Host", &host), ("Sec-Fetch-Site", site)], None)).0, 403, "{site}");
    }
    // Forged Host (DNS rebinding) is refused even with a good Sec-Fetch-Site.
    for bad in ["evil.example.com", "evil.example.com:80", "127.0.0.1.evil.com", "localhost.evil.com:9", ""] {
        assert_eq!(raw(p, &req("GET", "/api/status", &[("Host", bad), ("Sec-Fetch-Site", "same-origin")], None)).0, 403, "host {bad:?}");
    }
    // A foreign Origin is refused too.
    assert_eq!(raw(p, &req("GET", "/api/status", &[("Host", &host), ("Sec-Fetch-Site", "same-origin"), ("Origin", "http://evil.example.com")], None)).0, 403);
    // Allowed: same-origin and none, with 127.0.0.1 or localhost (with or without port).
    for (h, site) in [(host.as_str(), "same-origin"), (host.as_str(), "none"), ("localhost", "same-origin"), ("localhost:5173", "same-origin")] {
        assert_eq!(raw(p, &req("GET", "/api/status", &[("Host", h), ("Sec-Fetch-Site", site)], None)).0, 200, "{h} {site}");
    }
    // The agent token does not open the browser door.
    let auth = format!("Bearer {TOKEN}");
    assert_eq!(raw(p, &req("GET", "/api/status", &[("Host", &host), ("Authorization", &auth)], None)).0, 403);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_door_needs_the_token_and_a_local_host() {
    let s = start().await;
    let p = s.port;
    let host = format!("127.0.0.1:{p}");
    assert_eq!(raw(p, &req("GET", "/agent/v1/guide", &[("Host", &host)], None)).0, 401);
    assert_eq!(raw(p, &req("GET", "/agent/v1/guide", &[("Host", &host), ("Authorization", "Bearer nope")], None)).0, 401);
    assert_eq!(raw(p, &req("GET", "/agent/v1/guide", &[("Host", &host), ("Authorization", "Basic abc")], None)).0, 401);
    let auth = format!("Bearer {TOKEN}");
    assert_eq!(raw(p, &req("GET", "/agent/v1/guide", &[("Host", "evil.example.com"), ("Authorization", &auth)], None)).0, 403);
    let (st, _, body) = raw(p, &req("GET", "/agent/v1/guide", &[("Host", &host), ("Authorization", &auth)], None));
    assert_eq!(st, 200);
    assert!(body.contains("Search before you propose"));
    // The agent door does not need Sec-Fetch-Site (agents are not browsers).
    assert_eq!(agent(p, "GET", "/agent/v1/people?q=gus", "bot", None).0, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_cannot_edit_delete_approve_or_reject() {
    let s = start().await;
    let p = s.port;
    let gid = first_person_id(&s.st);
    let (st, r) = agent(p, "POST", "/agent/v1/proposals", "bot", Some(&json!({"summary": "x", "people": [{"ref": "a", "full_name": "New Nina"}]}).to_string()));
    assert_eq!(st, 201, "{r}");
    let pid = r["proposal_id"].as_str().unwrap().to_string();
    for (m, path) in [
        ("POST", format!("/agent/v1/proposals/{pid}/approve")),
        ("POST", format!("/agent/v1/proposals/{pid}/reject")),
        ("PATCH", format!("/agent/v1/people/{gid}")),
        ("PUT", format!("/agent/v1/people/{gid}")),
        ("DELETE", format!("/agent/v1/people/{gid}")),
        ("POST", "/agent/v1/people".to_string()),
        ("POST", "/agent/v1/relationships".to_string()),
        ("POST", format!("/agent/v1/history/{gid}/undo")),
        ("GET", "/agent/v1/backup".to_string()),
    ] {
        let (st, _) = agent(p, m, &path, "bot", Some("{}"));
        assert!(st == 404 || st == 405, "{m} {path} -> {st}");
    }
    // Browser-only routes with the agent token (and no browser headers) fail.
    let auth = format!("Bearer {TOKEN}");
    let host = format!("127.0.0.1:{p}");
    for (m, path) in [
        ("POST", format!("/api/proposals/{pid}/approve")),
        ("POST", format!("/api/proposals/{pid}/reject")),
        ("PATCH", format!("/api/people/{gid}")),
        ("DELETE", format!("/api/people/{gid}")),
        ("GET", "/api/backup".to_string()),
    ] {
        assert_eq!(raw(p, &req(m, &path, &[("Host", &host), ("Authorization", &auth)], Some("{}"))).0, 403, "{m} {path}");
    }
    // Nothing changed.
    let store = s.st.store.lock().unwrap();
    assert_eq!(store.get_proposal(&CTX, &pid).unwrap().status, "pending");
    assert!(store.get_person(&CTX, &gid).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_proposal_to_ui_approval_round_trip() {
    let s = start().await;
    let p = s.port;
    let gid = first_person_id(&s.st);
    let body = json!({"summary": "Add Gus's son", "source_text": "Gus had a son Omid",
        "people": [{"ref": "o", "given_name": "Omid", "family_name": "Tester", "sex": "M", "birth_date": "1950"}],
        "relationships": [{"type": "parent_child", "a": gid, "b": "o", "kind": "adopted"}]});
    let (st, r) = agent(p, "POST", "/agent/v1/proposals", "my bot", Some(&body.to_string()));
    assert_eq!(st, 201, "{r}");
    let pid = r["proposal_id"].as_str().unwrap().to_string();
    // The UI can ask the server to convert a Jalali date it is about to save.
    let (st, c) = ui(p, "POST", "/api/calendar/convert", Some(r#"{"calendar":"jalali","date":"1329-05-12"}"#));
    assert_eq!((st, c["stored"].as_str(), c["approximate"].as_bool()), (200, Some("1950-08-03"), Some(false)));
    let (_, c) = ui(p, "POST", "/api/calendar/convert", Some(r#"{"calendar":"jalali","date":"1329"}"#));
    assert_eq!((c["stored"].as_str(), c["approximate"].as_bool()), (Some("1950"), Some(true)));
    assert_eq!(ui(p, "POST", "/api/calendar/convert", Some(r#"{"calendar":"jalali","date":"1404-12-30"}"#)).0, 400);
    assert_eq!(ui(p, "POST", "/api/calendar/convert", Some(r#"{"calendar":"hijri","date":"1329"}"#)).0, 400);
    // The UI sees it as pending, with the agent's name.
    let (_, status) = ui(p, "GET", "/api/status", None);
    assert_eq!(status["pending_proposals"], 1);
    let (_, view) = ui(p, "GET", &format!("/api/proposals/{pid}"), None);
    assert_eq!(view["new_people"][0]["created_by"], "agent:my bot");
    assert_eq!(view["new_relationships"][0]["kind"], "adopted");
    // Verified-only agent search does not see him; own listing shows the proposal as pending.
    assert_eq!(agent(p, "GET", "/agent/v1/people?q=omid", "my bot", None).1["people"].as_array().unwrap().len(), 0);
    assert_eq!(agent(p, "GET", "/agent/v1/proposals", "my bot", None).1["proposals"][0]["status"], "pending");
    assert_eq!(agent(p, "GET", "/agent/v1/proposals", "someone else", None).1["proposals"].as_array().unwrap().len(), 0);
    // The human approves in the UI.
    let (st, rep) = ui(p, "POST", &format!("/api/proposals/{pid}/approve"), Some("{}"));
    assert_eq!(st, 200, "{rep}");
    assert_eq!(rep["people_verified"], 1);
    assert_eq!(agent(p, "GET", "/agent/v1/people?q=omid", "my bot", None).1["people"].as_array().unwrap().len(), 1);
    assert_eq!(agent(p, "GET", "/agent/v1/proposals", "my bot", None).1["proposals"][0]["status"], "approved");
    // Approving twice is a clear conflict.
    assert_eq!(ui(p, "POST", &format!("/api/proposals/{pid}/approve"), Some("{}")).0, 409);
    // Relationship + graph work through the agent door.
    let oid = agent(p, "GET", "/agent/v1/people?q=omid", "b", None).1["people"][0]["id"].as_str().unwrap().to_string();
    let (st, rel) = agent(p, "GET", &format!("/agent/v1/relationship?a={gid}&b={oid}"), "b", None);
    assert_eq!(st, 200);
    assert_eq!(rel["by_blood"], false);
    assert_eq!(rel["has_non_birth_link"], true);
    let (st, g) = agent(p, "GET", &format!("/agent/v1/graph?focus={gid}&down=1"), "b", None);
    assert_eq!(st, 200);
    assert_eq!(g["people"].as_array().unwrap().len(), 2);
    // History + undo through the UI.
    let (_, h) = ui(p, "GET", &format!("/api/history?target_type=person&target_id={oid}"), None);
    assert!(!h["revisions"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn field_errors_come_back_as_422() {
    let s = start().await;
    let (st, r) = agent(s.port, "POST", "/agent/v1/proposals", "bot", Some(&json!({"summary": "x", "people": [{"given_name": "A", "sex": "Q"}]}).to_string()));
    assert_eq!(st, 422);
    let paths: Vec<&str> = r["error"]["fields"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert!(paths.contains(&"$.people[0].sex") && paths.contains(&"$.people[0].ref"), "{paths:?}");
    // Oversized body.
    let big = format!("{{\"summary\": \"{}\"}}", "x".repeat(1_100_000));
    assert_eq!(agent(s.port, "POST", "/agent/v1/proposals", "bot", Some(&big)).0, 413);
    // Malformed JSON is a 4xx, not a crash.
    let (st, _) = agent(s.port, "POST", "/agent/v1/proposals", "bot", Some("{nope"));
    assert!((400..500).contains(&st));
}

fn mcp(p: u16, body: Value, extra: &[(&str, &str)]) -> (u16, String, Value) {
    let auth = format!("Bearer {TOKEN}");
    let host = format!("127.0.0.1:{p}");
    let mut h = vec![("Host", host.as_str()), ("Authorization", auth.as_str()), ("Accept", "application/json, text/event-stream")];
    h.extend_from_slice(extra);
    let (s, head, b) = raw(p, &req("POST", "/agent/mcp", &h, Some(&body.to_string())));
    (s, head, serde_json::from_str(&b).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_has_exactly_nine_tools_and_no_way_to_approve_edit_or_delete() {
    let s = start().await;
    let p = s.port;
    let (st, head, init) = mcp(
        p,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "Claude Desktop", "version": "1"}}}),
        &[],
    );
    assert_eq!(st, 200);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "family-quilt");
    let sid = head.lines().find_map(|l| l.strip_prefix("mcp-session-id: ")).unwrap().trim().to_string();
    // Notification: 202, no body.
    assert_eq!(mcp(p, json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), &[("Mcp-Session-Id", &sid)]).0, 202);
    let (_, _, list) = mcp(p, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), &[("Mcp-Session-Id", &sid)]);
    let mut names: Vec<String> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    names.sort();
    assert_eq!(names, ["create_proposal", "find_relationship", "get_graph", "get_person", "get_proposal", "list_my_proposals", "list_people", "search_people", "withdraw_proposal"]);
    for n in &names {
        for banned in ["approve", "reject", "edit", "delete", "update", "remove", "merge", "undo"] {
            assert!(!n.contains(banned), "{n}");
        }
    }
    // Descriptions carry the guidance the spec asks for.
    let cp = list["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "create_proposal").unwrap()["description"].as_str().unwrap().to_string();
    assert!(cp.contains("search before proposing") || cp.contains("Always search before proposing"));
    assert!(cp.contains("until the user approves") && cp.contains("adopted") && cp.contains("unknown"));
    // Calling a tool that does not exist (e.g. approve) is a tool error, and changes nothing.
    let gid = first_person_id(&s.st);
    for bad in ["approve_proposal", "reject_proposal", "update_person", "delete_person"] {
        let (_, _, r) = mcp(p, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": bad, "arguments": {"id": gid}}}), &[]);
        assert_eq!(r["result"]["isError"], true, "{bad}");
    }
    assert!(s.st.store.lock().unwrap().get_person(&CTX, &gid).is_ok());

    // Search, read, relationship, create proposal; the proposal is pending in the UI under the client's name.
    let call = |id: u64, name: &str, args: Value| mcp(p, json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}}), &[("Mcp-Session-Id", &sid)]).2;
    let r = call(4, "search_people", json!({"q": "gus"}));
    assert_eq!(r["result"]["isError"], false);
    let found: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(found["people"][0]["display_name"], "Gus Tester");
    assert_eq!(call(5, "get_person", json!({"id": gid}))["result"]["isError"], false);
    let dan = s.st.store.lock().unwrap().all_people(&CTX, false).unwrap()[1].id.clone();
    let r = call(6, "find_relationship", json!({"a": gid, "b": dan}));
    assert_eq!(r["result"]["isError"], false);
    let r = call(7, "create_proposal", json!({"summary": "via mcp", "people": [{"ref": "x", "full_name": "Mcp Person"}], "relationships": [{"type": "parent_child", "a": gid, "b": "x"}]}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    let created: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let pid = created["proposal_id"].as_str().or(created["id"].as_str()).unwrap_or_default().to_string();
    assert!(!pid.is_empty(), "{created}");
    // get_proposal returns the new person with an id; list_people finds the loose end and pages.
    let r = call(41, "get_proposal", json!({"id": pid}));
    let gp: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(gp["people"][0]["name"], "Mcp Person");
    assert_eq!(gp["people"][0]["proposal_id"], pid.as_str());
    assert_eq!(gp["people"][0]["parents"][0]["id"], gid.as_str());
    assert_eq!(gp["relationships"].as_array().unwrap().len(), 1);
    let r = call(42, "list_people", json!({"status": "pending", "created_by_proposal": pid, "limit": 1}));
    let lp: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!((lp["total"].as_u64(), lp["next_offset"].is_null()), (Some(1), true));
    let r = call(43, "list_people", json!({"status": "all", "no_parents": true, "limit": 1}));
    let lp: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(lp["total"].as_u64().unwrap() >= 1 && lp["people"].as_array().unwrap().len() == 1);
    assert_eq!(call(44, "list_people", json!({"status": "nope"}))["result"]["isError"], true);
    // Another agent's proposal is not readable.
    assert_eq!(call(45, "get_proposal", json!({"id": "does-not-exist"}))["result"]["isError"], true);
    let (_, status) = ui(p, "GET", "/api/status", None);
    assert_eq!(status["pending_proposals"], 1);
    let (_, list) = ui(p, "GET", "/api/proposals?status=pending", None);
    assert_eq!(list["proposals"][0]["created_by"], "agent:Claude Desktop");
    // Bad input: a tool error with field-level details.
    let r = call(8, "create_proposal", json!({"summary": "bad", "people": [{"ref": "x", "full_name": "A", "sex": "Z"}]}));
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("$.people[0].sex"));
    // Guide resource.
    let (_, _, g) = mcp(p, json!({"jsonrpc": "2.0", "id": 9, "method": "resources/read", "params": {"uri": "family-quilt://guide"}}), &[]);
    assert!(g["result"]["contents"][0]["text"].as_str().unwrap().contains("Search before you propose"));
    // Unknown method.
    assert_eq!(mcp(p, json!({"jsonrpc": "2.0", "id": 10, "method": "tools/approve"}), &[]).2["error"]["code"], -32601);
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_endpoint_security_and_protocol_rules() {
    let s = start().await;
    let p = s.port;
    let host = format!("127.0.0.1:{p}");
    let auth = format!("Bearer {TOKEN}");
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string();
    // No token.
    assert_eq!(raw(p, &req("POST", "/agent/mcp", &[("Host", &host)], Some(&body))).0, 401);
    // Foreign Origin (browser on another site) and forged Host.
    assert_eq!(raw(p, &req("POST", "/agent/mcp", &[("Host", &host), ("Authorization", &auth), ("Origin", "http://evil.example.com")], Some(&body))).0, 403);
    assert_eq!(raw(p, &req("POST", "/agent/mcp", &[("Host", "evil.example.com"), ("Authorization", &auth)], Some(&body))).0, 403);
    // GET / DELETE: no stream, no sessions to delete.
    assert_eq!(raw(p, &req("GET", "/agent/mcp", &[("Host", &host), ("Authorization", &auth), ("Accept", "text/event-stream")], None)).0, 405);
    assert_eq!(raw(p, &req("DELETE", "/agent/mcp", &[("Host", &host), ("Authorization", &auth)], None)).0, 405);
    // The newest revision asks for stateless metadata we do not speak: 400 with our supported list, so clients fall back.
    let (st, _, r) = mcp(p, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}), &[("MCP-Protocol-Version", "2026-07-28")]);
    assert_eq!(st, 400);
    assert!(r["error"]["data"]["supported"].as_array().unwrap().iter().any(|v| v == "2025-11-25"));
    // Known version header works; parse error and batch are rejected.
    assert_eq!(mcp(p, json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}), &[("MCP-Protocol-Version", "2025-06-18")]).0, 200);
    assert_eq!(raw(p, &req("POST", "/agent/mcp", &[("Host", &host), ("Authorization", &auth)], Some("{nope"))).0, 400);
    assert_eq!(raw(p, &req("POST", "/agent/mcp", &[("Host", &host), ("Authorization", &auth)], Some("[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]"))).0, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_bridge_forwards_and_explains_when_the_app_is_closed() {
    let s = start().await;
    let closed = tempfile::tempdir().unwrap();
    let dir = s.dir.path().to_path_buf();
    let gid = first_person_id(&s.st);
    let call = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "search_people", "arguments": {"q": "gus"}}});
    let (closed_path, dir2, call2) = (closed.path().to_path_buf(), dir.clone(), call.clone());
    let (r_closed, r_open, r_list, r_note) = tokio::task::spawn_blocking(move || {
        let mut agent = String::new();
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "clientInfo": {"name": "bridge-client"}}});
        // App closed (no endpoint file): initialize and tools/list still work, tools/call explains.
        bridge::handle_message(&closed_path, &mut agent, &init).unwrap();
        let list = bridge::handle_message(&closed_path, &mut agent, &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        let r_closed = bridge::handle_message(&closed_path, &mut agent, &call2).unwrap();
        // App open: the call is forwarded to the running app.
        std::fs::write(token::token_path(&dir2), TOKEN).unwrap();
        let note = bridge::handle_message(&dir2, &mut agent, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        (r_closed, bridge::handle_message(&dir2, &mut agent, &call2).unwrap_or_default(), list, note)
    })
    .await
    .unwrap_or_else(|_| unreachable!());
    assert_eq!(r_closed["result"]["isError"], true);
    assert!(r_closed["result"]["content"][0]["text"].as_str().unwrap().contains("Open the Family Tree app first"));
    assert_eq!(r_list["result"]["tools"].as_array().unwrap().len(), 9);
    assert!(r_note.is_none());
    // Without the endpoint file the open case is still "closed"; write it and retry.
    token::write_endpoint(&dir, s.port).unwrap();
    let dir3 = dir.clone();
    let r = tokio::task::spawn_blocking(move || {
        let mut agent = "bridge-client".to_string();
        bridge::handle_message(&dir3, &mut agent, &call).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("Gus Tester"));
    let _ = (r_open, gid);
    // The bridge's activity is logged under the client's name.
    assert!(s.st.activity.lock().unwrap().iter().any(|a| a.agent == "bridge-client"));
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_snippets_and_backup() {
    let s = start().await;
    let p = s.port;
    let (st, v) = ui(p, "GET", "/api/settings/agent", None);
    assert_eq!(st, 200);
    assert_eq!(v["token"], TOKEN);
    assert!(v["mcp_url"].as_str().unwrap().ends_with("/agent/mcp"));
    assert!(v["claude_desktop_config"].as_str().unwrap().contains("--mcp-stdio"));
    assert!(v["claude_code_http"].as_str().unwrap().contains("--transport http"));
    let (_, t) = ui(p, "POST", "/api/settings/agent/test", None);
    assert_eq!(t["tools"].as_array().unwrap().len(), 9);
    let (st, head, body) = raw(p, &req("GET", "/api/backup", &[("Host", &format!("127.0.0.1:{p}")), ("Sec-Fetch-Site", "same-origin")], None));
    assert_eq!(st, 200);
    assert!(head.contains("attachment"));
    assert!(body.starts_with("SQLite format 3"), "backup is a SQLite file");
}

#[tokio::test(flavor = "multi_thread")]
async fn human_edits_are_logged_and_undoable() {
    let s = start().await;
    let p = s.port;
    let gid = first_person_id(&s.st);
    let (st, _) = ui(p, "PATCH", &format!("/api/people/{gid}"), Some(&json!({"note": "hello", "birth_date": "1900"}).to_string()));
    assert_eq!(st, 200);
    let (st, e) = ui(p, "PATCH", &format!("/api/people/{gid}"), Some(&json!({"status": "pending"}).to_string()));
    assert_eq!(st, 422, "{e}");
    let (_, h) = ui(p, "GET", &format!("/api/history?target_type=person&target_id={gid}&limit=1"), None);
    let rid = h["revisions"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(ui(p, "POST", &format!("/api/history/{rid}/undo"), None).0, 200);
    let (_, d) = ui(p, "GET", &format!("/api/people/{gid}"), None);
    assert_eq!(d["person"]["note"], Value::Null);
}
