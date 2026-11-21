//! The UI files compiled into the executable must be all the files in `web/` and be served safely.

use family_server::{serve, AppState};
use family_store::Store;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

fn get(port: u16, path: &str) -> (u16, String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let at = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&buf[..at]).to_lowercase();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut body = buf[at + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        let mut out = vec![];
        let mut rest = &body[..];
        loop {
            let p = rest.windows(2).position(|w| w == b"\r\n").unwrap();
            let n = usize::from_str_radix(std::str::from_utf8(&rest[..p]).unwrap().trim(), 16).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&rest[p + 2..p + 2 + n]);
            rest = &rest[p + 2 + n + 2..];
        }
        body = out;
    }
    (status, head, body)
}

fn files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            if p.file_name().unwrap() != "tests" {
                files(&p, base, out);
            }
        } else {
            out.push(p.strip_prefix(base).unwrap().to_path_buf());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_file_in_web_is_embedded_and_served_with_a_strict_policy() {
    std::env::remove_var("FAMILY_WEB_DIR");
    let dir = tempfile::tempdir().unwrap();
    let st = AppState::new(Store::open_in_memory().unwrap(), "t".into(), dir.path().to_path_buf());
    let (port, _h) = serve(st, 0).await.unwrap();
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web");
    let mut list = vec![];
    files(&web, &web, &mut list);
    assert!(list.len() >= 10);
    for f in list {
        let url = format!("/{}", f.to_string_lossy().replace('\\', "/"));
        let (status, head, body) = get(port, &url);
        assert_eq!(status, 200, "{url} is not embedded: add it to ASSETS in web.rs");
        assert_eq!(body, std::fs::read(web.join(&f)).unwrap(), "{url} differs");
        assert!(head.contains("content-security-policy: default-src 'self'"), "{url} needs the CSP");
    }
    let (s, h, _) = get(port, "/");
    assert_eq!(s, 200);
    assert!(h.contains("text/html"));
    assert_eq!(get(port, "/nope.js").0, 404);
    assert_eq!(get(port, "/../Cargo.toml").0, 404);
    // A forged Host is still refused for the UI files.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n").unwrap();
    let mut b = String::new();
    s.read_to_string(&mut b).unwrap();
    assert!(b.contains(" 403 "));
}
