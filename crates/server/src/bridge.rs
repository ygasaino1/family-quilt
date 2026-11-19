//! `family-app --mcp-stdio`: a stdio MCP server that forwards tool calls to the **running** app's
//! localhost agent API. It never opens the database itself, so a second copy can never appear.
//! `initialize`, `tools/list`, `ping` and the guide resource are answered locally, so the client
//! shows the server as healthy; a tool call while the app is closed returns a plain tool error.

use crate::{mcp, token};
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

const APP_CLOSED: &str = "Open the Family Tree app first, then try again. (The app must be running for AI assistants to reach the tree.)";

fn tool_error(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": msg}], "isError": true})
}

/// Minimal HTTP/1.1 POST over a plain socket (localhost only, no TLS, no extra dependency).
pub fn http_post(port: u16, path: &str, headers: &[(&str, &str)], body: &str) -> std::io::Result<(u16, String)> {
    let mut s = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(60)))?;
    let mut req = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        if !v.contains(['\r', '\n']) {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    s.write_all(body.as_bytes())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| std::io::Error::other("bad response"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
    let body = &raw[split + 4..];
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    let body = if chunked { dechunk(body) } else { body.to_vec() };
    Ok((status, String::from_utf8_lossy(&body).to_string()))
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    while let Some(p) = b.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..p]).split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        b = &b[p + 2..];
        if size == 0 || b.len() < size {
            break;
        }
        out.extend_from_slice(&b[..size]);
        b = b.get(size + 2..).unwrap_or(&[]);
    }
    out
}

fn endpoint(data_dir: &Path) -> Option<(u16, String)> {
    let e: Value = serde_json::from_str(&std::fs::read_to_string(token::endpoint_path(data_dir)).ok()?).ok()?;
    let port = e["port"].as_u64()? as u16;
    let tok = std::env::var("FAMILY_AGENT_TOKEN").ok().filter(|t| !t.trim().is_empty()).or_else(|| std::fs::read_to_string(token::token_path(data_dir)).ok())?;
    Some((port, tok.trim().to_string()))
}

/// Forward one `tools/call` to the running app. Returns the JSON-RPC `result`.
fn forward(data_dir: &Path, agent: &str, request: &Value) -> Value {
    let Some((port, tok)) = endpoint(data_dir) else { return tool_error(APP_CLOSED) };
    let auth = format!("Bearer {tok}");
    match http_post(port, "/agent/mcp", &[("Authorization", &auth), ("X-Agent-Name", agent)], &request.to_string()) {
        Err(_) => tool_error(APP_CLOSED),
        Ok((401 | 403, _)) => tool_error("The app refused the connection (the access token changed). Restart the AI assistant, or copy the settings again from Settings > AI assistant."),
        Ok((_, body)) => match serde_json::from_str::<Value>(&body) {
            Ok(v) if v.get("result").is_some() => v["result"].clone(),
            Ok(v) => tool_error(v["error"]["message"].as_str().unwrap_or("The app could not answer.")),
            Err(_) => tool_error("The app sent an answer that could not be read."),
        },
    }
}

/// One JSON-RPC message in, zero or one out.
pub fn handle_message(data_dir: &Path, agent: &mut String, msg: &Value) -> Option<Value> {
    let method = msg["method"].as_str()?;
    let id = msg.get("id").filter(|i| !i.is_null())?.clone(); // notifications get no answer
    let params = &msg["params"];
    let result = match method {
        "initialize" => {
            *agent = family_review::clean_agent_name(params["clientInfo"]["name"].as_str().unwrap_or("mcp-client"));
            mcp::initialize_result(params["protocolVersion"].as_str())
        }
        "tools/call" => forward(data_dir, agent, msg),
        m => match mcp::static_method(m, params) {
            Some(r) => r,
            None => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {m}")}})),
        },
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Run until stdin closes. One JSON message per line, as the stdio transport defines.
pub fn run(data_dir: &Path) {
    let mut agent = String::from("mcp-client");
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}));
            continue;
        };
        if let Some(resp) = handle_message(data_dir, &mut agent, &msg) {
            let _ = writeln!(out, "{resp}");
            let _ = out.flush();
        }
    }
}
