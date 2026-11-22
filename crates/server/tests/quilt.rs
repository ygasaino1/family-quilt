use family_server::{serve, AppState, Shared};
use family_store::*;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;

const CTX: AccessContext = AccessContext::Everything;

fn get(port: u16, path: &str) -> (u16, Value) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let r = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nSec-Fetch-Site: same-origin\r\nConnection: close\r\n\r\n");
    s.write_all(r.as_bytes()).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let t = String::from_utf8_lossy(&buf).to_string();
    let (h, b) = t.split_once("\r\n\r\n").unwrap();
    let status = h.split_whitespace().nth(1).unwrap().parse().unwrap();
    let b = if h.to_lowercase().contains("chunked") { dechunk(b) } else { b.to_string() };
    (status, serde_json::from_str(&b).unwrap_or(Value::Null))
}

fn dechunk(b: &str) -> String {
    let mut out = String::new();
    let mut rest = b;
    while let Some((len, r)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(len.trim(), 16).unwrap_or(0);
        if n == 0 || r.len() < n {
            break;
        }
        out.push_str(&r[..n]);
        rest = r[n..].trim_start_matches("\r\n");
    }
    out
}

async fn start() -> (u16, Shared, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let mut ids = vec![];
    for (n, s) in [("Anna Root", Sex::Female), ("Bob Root", Sex::Male), ("Cy Root", Sex::Male)] {
        ids.push(store.create_person(&CTX, NewPerson { sex: s, names: vec![NewName::full(n)], ..Default::default() }, &Origin::human()).unwrap().id);
    }
    store.add_relationship(&CTX, NewRelationship::partnership(&ids[0], &ids[1], PartnershipState::Married), &Origin::human()).unwrap();
    store.add_relationship(&CTX, NewRelationship::parent_child(&ids[0], &ids[2], LinkKind::Birth), &Origin::human()).unwrap();
    store.add_relationship(&CTX, NewRelationship::parent_child(&ids[1], &ids[2], LinkKind::Adopted), &Origin::human()).unwrap();
    let st = AppState::new(store, "t".into(), dir.path().to_path_buf());
    let (port, _h) = serve(st.clone(), 0).await.unwrap();
    (port, st, dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn quilt_quick_then_full_and_cached() {
    let (p, _st, _d) = start().await;
    let (s, v) = get(p, "/api/quilt");
    assert_eq!(s, 200);
    assert!(v["layout"] == "quick" || v["layout"] == "full");
    assert_eq!(v["people"].as_array().unwrap().len(), 3);
    assert_eq!(v["stats"]["people"], 3);
    // The careful layout finishes and replaces the quick one.
    let mut full = false;
    for _ in 0..100 {
        let (_, pr) = get(p, "/api/quilt/progress");
        if pr["layout"] == "full" && pr["current"] == true {
            full = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(full);
    let (_, v2) = get(p, "/api/quilt");
    assert_eq!(v2["layout"], "full");
    assert_eq!(v2["people"], v["people"]);
    let (s, v3) = get(p, "/api/quilt?mode=layer");
    assert_eq!(s, 200);
    assert_eq!(v3["mode"], "layer");
}

#[tokio::test(flavor = "multi_thread")]
async fn quilt_payload_has_columns_and_link_kinds() {
    let (p, _st, _d) = start().await;
    let (_, v) = get(p, "/api/quilt");
    let cols = v["columns"].as_array().unwrap();
    assert_eq!(cols.len(), 1);
    let kids = cols[0]["c"].as_array().unwrap();
    assert_eq!(kids.len(), 1);
    assert_eq!(cols[0]["h"], "married");
}

#[tokio::test(flavor = "multi_thread")]
async fn dot_engine_is_optional_and_never_breaks_the_quilt() {
    let (p, _st, _d) = start().await;
    let (s, v) = get(p, "/api/quilt?engine=dot");
    assert_eq!(s, 200);
    assert_eq!(v["people"].as_array().unwrap().len(), 3);
    let mut engine = String::new();
    for _ in 0..100 {
        let (_, v) = get(p, "/api/quilt?engine=dot");
        if v["layout"] == "full" {
            engine = v["engine"].as_str().unwrap_or("").to_string();
            assert_eq!(v["columns"].as_array().unwrap().len(), 1);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Graphviz present: the original's engine made it; absent: the built-in one, with the reason.
    if family_layout::dot::find_dot().is_some() {
        assert_eq!(engine, "dot");
    } else {
        assert!(engine.starts_with("own"), "{engine}");
    }
    // The built-in engine's slot is separate and unaffected.
    let (_, v) = get(p, "/api/quilt");
    assert!(v["engine"] == "own");
}
