//! What the HTTP door and the stdio bridge both need to speak MCP correctly, as pure functions (no I/O, easy to test).
//!
//! This server is **dual-era**, as the 2026-07-28 revision allows:
//!
//! * **Modern** (`2026-07-28`): no handshake and no session. Every request says which revision it speaks in
//!   `_meta["io.modelcontextprotocol/protocolVersion"]` (and on HTTP in the `MCP-Protocol-Version` header, with
//!   `Mcp-Method` and, for calls that name something, `Mcp-Name`); who is calling is in `_meta` `clientInfo`.
//!   `server/discover` tells a client what we speak. Results carry `resultType`, and lists carry `ttlMs`/`cacheScope`.
//! * **Legacy** (`2025-11-25`, `2025-06-18`): the `initialize` handshake. `2025-03-26` (JSON-RPC batches) is not spoken.
//!   The legacy session is **stateless**: the `Mcp-Session-Id` we hand out is the client's name, encoded, so there is
//!   no session table to grow, expire or lose; an id that is not ours is a 404, which tells the client to
//!   initialise again.
//!
//! A request that carries a revision we do not speak gets `UnsupportedProtocolVersionError` (`-32022`) listing
//! what we do: that is how a client knows it reached a modern server and which version to retry with.

use serde_json::{json, Value};

pub const MODERN: &[&str] = &["2026-07-28"];
pub const LEGACY: &[&str] = &["2025-11-25", "2025-06-18"];

/// Everything we speak, newest first. This is what discovery and the unsupported-version error list.
pub fn supported() -> Vec<&'static str> {
    MODERN.iter().chain(LEGACY).copied().collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Era {
    Modern,
    Legacy,
}

pub fn era_of(version: &str) -> Option<Era> {
    if MODERN.contains(&version) {
        Some(Era::Modern)
    } else if LEGACY.contains(&version) {
        Some(Era::Legacy)
    } else {
        None
    }
}

// JSON-RPC and MCP error codes. -32020..-32099 belong to the MCP specification; -32000..-32019 are ours to use.
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const HEADER_MISMATCH: i64 = -32020;
pub const UNSUPPORTED_VERSION: i64 = -32022;
/// What the 2025 revisions answered for an unknown resource (2026-07-28 uses `INVALID_PARAMS`).
pub const LEGACY_RESOURCE_NOT_FOUND: i64 = -32002;
/// A legacy request whose `Mcp-Session-Id` is not one we issued.
pub const SESSION_NOT_FOUND: i64 = -32001;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into(), data: None }
    }

    /// The JSON-RPC error response for request `id`.
    pub fn response(&self, id: &Value) -> Value {
        let mut e = json!({"code": self.code, "message": self.message});
        if let Some(d) = &self.data {
            e["data"] = d.clone();
        }
        json!({"jsonrpc": "2.0", "id": id, "error": e})
    }
}

/// `UnsupportedProtocolVersionError`: what we do speak, and what was asked for.
pub fn unsupported_version(requested: &str) -> RpcError {
    RpcError { code: UNSUPPORTED_VERSION, message: "Unsupported protocol version".into(), data: Some(json!({"supported": supported(), "requested": requested})) }
}

/// An error plus the HTTP status it travels with.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpError {
    pub status: u16,
    pub error: RpcError,
}

fn http(status: u16, code: i64, message: impl Into<String>) -> HttpError {
    HttpError { status, error: RpcError::new(code, message) }
}

pub const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

pub fn server_info() -> Value {
    json!({"name": "family-quilt", "title": "Family Tree", "version": crate::VERSION})
}

const INSTRUCTIONS: &str = "Read and propose changes to the user's family tree. Search before proposing. Nothing is final until the user approves it in the Family Tree app.";

/// How long a client may keep a list or the discovery answer. They only change when the app is updated.
pub const TTL_MS: u64 = 300_000;

/// The answer to `server/discover` (modern).
pub fn discover_result() -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": supported(),
        "capabilities": {"tools": {}, "resources": {}},
        "_meta": {META_SERVER_INFO: server_info()},
        "instructions": INSTRUCTIONS,
        "ttlMs": TTL_MS,
        "cacheScope": "private"
    })
}

/// The answer to `initialize` (legacy): the client's revision if we speak it, else our newest legacy one.
pub fn initialize_result(requested: Option<&str>) -> Value {
    let version = requested.filter(|v| LEGACY.contains(v)).unwrap_or(LEGACY[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}, "resources": {"listChanged": false, "subscribe": false}},
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS
    })
}

/// The methods of the modern revision that we implement. Anything else is `Method not found` (404 on HTTP). `ping`,
/// `logging/setLevel` and the handshake no longer exist in it.
pub const MODERN_METHODS: &[&str] = &["server/discover", "tools/list", "tools/call", "resources/list", "resources/read", "resources/templates/list", "prompts/list"];

/// Results a client may cache: they carry `ttlMs` and `cacheScope`.
const CACHEABLE: &[&str] = &["server/discover", "tools/list", "resources/list", "resources/templates/list", "resources/read", "prompts/list"];

/// What every modern result carries: `resultType`, our identity in `_meta`, and for lists and reads how long they stay fresh.
pub fn decorate_modern(method: &str, mut result: Value) -> Value {
    let Some(o) = result.as_object_mut() else { return result };
    o.entry("resultType").or_insert_with(|| json!("complete"));
    if !o.get("_meta").is_some_and(Value::is_object) {
        o.insert("_meta".into(), json!({}));
    }
    if let Some(m) = o.get_mut("_meta").and_then(Value::as_object_mut) {
        m.entry(META_SERVER_INFO).or_insert_with(server_info);
    }
    if CACHEABLE.contains(&method) {
        o.entry("ttlMs").or_insert_with(|| json!(TTL_MS));
        o.entry("cacheScope").or_insert_with(|| json!("private"));
    }
    result
}

/// The request metadata a POST carries as HTTP headers (`None` = absent).
#[derive(Debug, Default, Clone)]
pub struct Headers {
    pub version: Option<String>,
    pub method: Option<String>,
    pub name: Option<String>,
}

/// Which revision a POST speaks, from its headers. `initialize` is always the legacy handshake (a legacy client sends
/// it before it has a version to put in a header); anything else must say which revision it is.
pub fn classify(method: &str, h: &Headers) -> Result<Era, HttpError> {
    if method == "initialize" {
        return Ok(Era::Legacy);
    }
    match h.version.as_deref() {
        None => Err(http(400, HEADER_MISMATCH, "The MCP-Protocol-Version header is required on every request after the handshake (modern clients send it with each request)")),
        Some(v) => era_of(v).ok_or_else(|| HttpError { status: 400, error: unsupported_version(v) }),
    }
}

/// What a valid modern request tells us.
#[derive(Debug, PartialEq)]
pub struct ModernMeta {
    pub client: Option<String>,
}

/// A modern HTTP request: the body's `_meta` revision matches the header, `Mcp-Method` matches the method, and for
/// calls that name something `Mcp-Name` matches that name (a proxy that routes on headers must see what we execute).
pub fn check_modern(msg: &Value, h: &Headers) -> Result<ModernMeta, HttpError> {
    let method = msg["method"].as_str().unwrap_or("");
    let params = &msg["params"];
    let header_version = h.version.as_deref().unwrap_or("");
    match params["_meta"][META_VERSION].as_str() {
        None => return Err(http(400, INVALID_PARAMS, format!("`_meta` must carry {META_VERSION}: this is a {header_version} request"))),
        Some(v) if v != header_version => return Err(http(400, HEADER_MISMATCH, format!("Header mismatch: MCP-Protocol-Version header value '{header_version}' does not match _meta value '{v}'"))),
        Some(_) => {}
    }
    match h.method.as_deref() {
        None => return Err(http(400, HEADER_MISMATCH, "Header mismatch: the Mcp-Method header is required")),
        Some(m) if m != method => return Err(http(400, HEADER_MISMATCH, format!("Header mismatch: Mcp-Method header value '{m}' does not match body value '{method}'"))),
        Some(_) => {}
    }
    let named = match method {
        "tools/call" | "prompts/get" => Some(params["name"].as_str().unwrap_or("")),
        "resources/read" => Some(params["uri"].as_str().unwrap_or("")),
        _ => None,
    };
    if let Some(body_name) = named {
        match h.name.as_deref().map(decode_header_value) {
            None => return Err(http(400, HEADER_MISMATCH, format!("Header mismatch: the Mcp-Name header is required for {method}"))),
            Some(None) => return Err(http(400, HEADER_MISMATCH, "Header mismatch: the Mcp-Name header could not be decoded")),
            Some(Some(n)) if n != body_name => return Err(http(400, HEADER_MISMATCH, format!("Header mismatch: Mcp-Name header value '{n}' does not match body value '{body_name}'"))),
            Some(Some(_)) => {}
        }
    }
    Ok(ModernMeta { client: params["_meta"][META_CLIENT_INFO]["name"].as_str().map(str::to_string) })
}

// ---- the legacy session: the client's name, encoded. No table, nothing to expire.

const SESSION_PREFIX: &str = "fq-";

pub fn session_id(client: &str) -> String {
    format!("{SESSION_PREFIX}{}", client.bytes().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The client a session id was issued to; `None` for anything that is not one of ours.
pub fn client_of_session(id: &str) -> Option<String> {
    let hex = id.strip_prefix(SESSION_PREFIX)?;
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect();
    String::from_utf8(bytes?).ok().filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
}

// ---- header values: plain ASCII, or the Base64 sentinel `=?base64?...?=` when plain would not be safe.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes() {
        let v = B64.iter().position(|&b| b == ch)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

const SENTINEL_START: &str = "=?base64?";
const SENTINEL_END: &str = "?=";

fn is_sentinel(v: &str) -> bool {
    v.len() >= SENTINEL_START.len() + SENTINEL_END.len() && v.starts_with(SENTINEL_START) && v.ends_with(SENTINEL_END)
}

/// A header value for `v`: as is when it is visible ASCII with no padding space and does not look like the sentinel,
/// otherwise Base64 of its UTF-8 in the sentinel form.
pub fn encode_header_value(v: &str) -> String {
    let plain = !v.is_empty() && v.bytes().all(|b| (0x20..=0x7e).contains(&b)) && v == v.trim() && !is_sentinel(v);
    if plain {
        v.to_string()
    } else {
        format!("{SENTINEL_START}{}{SENTINEL_END}", b64_encode(v.as_bytes()))
    }
}

/// The inverse of [`encode_header_value`]; `None` when a sentinel value is not valid Base64 of UTF-8.
pub fn decode_header_value(v: &str) -> Option<String> {
    if is_sentinel(v) {
        let inner = &v[SENTINEL_START.len()..v.len() - SENTINEL_END.len()];
        String::from_utf8(b64_decode(inner)?).ok()
    } else {
        Some(v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(version: Option<&str>, method: Option<&str>, name: Option<&str>) -> Headers {
        Headers { version: version.map(String::from), method: method.map(String::from), name: name.map(String::from) }
    }

    fn call(version: &str, name: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": {}, "_meta": {META_VERSION: version, META_CLIENT_INFO: {"name": "Tester", "version": "1"}}}})
    }

    #[test]
    fn we_speak_one_modern_and_two_legacy_revisions_and_say_so_newest_first() {
        assert_eq!(supported(), ["2026-07-28", "2025-11-25", "2025-06-18"]);
        assert_eq!(era_of("2026-07-28"), Some(Era::Modern));
        assert_eq!(era_of("2025-06-18"), Some(Era::Legacy));
        assert_eq!(era_of("2025-03-26"), None, "the batching revision is not spoken");
        assert_eq!(era_of("1900-01-01"), None);
    }

    #[test]
    fn the_unsupported_version_error_is_the_specs_shape() {
        let e = unsupported_version("1900-01-01").response(&json!(7));
        assert_eq!(e["id"], 7);
        assert_eq!(e["error"]["code"], -32022);
        assert_eq!(e["error"]["data"]["requested"], "1900-01-01");
        assert_eq!(e["error"]["data"]["supported"], json!(["2026-07-28", "2025-11-25", "2025-06-18"]));
    }

    #[test]
    fn discovery_has_everything_the_spec_requires() {
        let d = discover_result();
        assert_eq!(d["resultType"], "complete");
        assert_eq!(d["supportedVersions"], json!(supported()));
        assert!(d["capabilities"]["tools"].is_object() && d["capabilities"]["resources"].is_object());
        assert_eq!(d["_meta"][META_SERVER_INFO]["name"], "family-quilt");
        assert!(d["ttlMs"].as_u64().unwrap() > 0 && d["cacheScope"] == "private");
        assert!(d["instructions"].as_str().unwrap().contains("Search before proposing"));
    }

    #[test]
    fn the_handshake_negotiates_only_legacy_revisions() {
        for (asked, got) in [
            (Some("2025-06-18"), "2025-06-18"),
            (Some("2025-11-25"), "2025-11-25"),
            (Some("2025-03-26"), "2025-11-25"),
            (Some("2026-07-28"), "2025-11-25"),
            (Some("nonsense"), "2025-11-25"),
            (None, "2025-11-25"),
        ] {
            assert_eq!(initialize_result(asked)["protocolVersion"], got, "{asked:?}");
        }
    }

    #[test]
    fn a_post_says_which_era_it_speaks() {
        let none = Headers::default();
        assert_eq!(classify("initialize", &none), Ok(Era::Legacy), "the handshake needs no header");
        assert_eq!(classify("initialize", &headers(Some("garbage"), None, None)), Ok(Era::Legacy));
        assert_eq!(classify("tools/call", &headers(Some("2026-07-28"), None, None)), Ok(Era::Modern));
        assert_eq!(classify("tools/call", &headers(Some("2025-11-25"), None, None)), Ok(Era::Legacy));
        let missing = classify("tools/call", &none).unwrap_err();
        assert_eq!((missing.status, missing.error.code), (400, HEADER_MISMATCH));
        let unknown = classify("tools/call", &headers(Some("1900-01-01"), None, None)).unwrap_err();
        assert_eq!((unknown.status, unknown.error.code), (400, UNSUPPORTED_VERSION));
        assert!(unknown.error.data.unwrap()["supported"].as_array().unwrap().len() == 3);
        // The batching revision is refused as unsupported (it would need JSON-RPC batches).
        assert_eq!(classify("tools/call", &headers(Some("2025-03-26"), None, None)).unwrap_err().error.code, UNSUPPORTED_VERSION);
    }

    #[test]
    fn a_valid_modern_request_passes_and_names_its_client() {
        let ok = check_modern(&call("2026-07-28", "search_people"), &headers(Some("2026-07-28"), Some("tools/call"), Some("search_people"))).unwrap();
        assert_eq!(ok.client.as_deref(), Some("Tester"));
        // A method that names nothing needs no Mcp-Name.
        let list = json!({"method": "tools/list", "params": {"_meta": {META_VERSION: "2026-07-28"}}});
        assert_eq!(check_modern(&list, &headers(Some("2026-07-28"), Some("tools/list"), None)).unwrap().client, None);
    }

    #[test]
    fn every_way_the_headers_can_disagree_with_the_body_is_a_header_mismatch() {
        let ok = headers(Some("2026-07-28"), Some("tools/call"), Some("search_people"));
        let code = |msg: &Value, h: &Headers| check_modern(msg, h).unwrap_err();
        // version header vs _meta
        let e = code(&call("2025-11-25", "search_people"), &ok);
        assert_eq!((e.status, e.error.code), (400, HEADER_MISMATCH));
        // Mcp-Method missing or different
        assert_eq!(code(&call("2026-07-28", "search_people"), &headers(Some("2026-07-28"), None, Some("search_people"))).error.code, HEADER_MISMATCH);
        assert_eq!(code(&call("2026-07-28", "search_people"), &headers(Some("2026-07-28"), Some("tools/list"), Some("search_people"))).error.code, HEADER_MISMATCH);
        // Mcp-Name missing or different (what a proxy routes on must be what we run)
        assert_eq!(code(&call("2026-07-28", "search_people"), &headers(Some("2026-07-28"), Some("tools/call"), None)).error.code, HEADER_MISMATCH);
        let e = code(&call("2026-07-28", "withdraw_proposal"), &ok);
        assert_eq!(e.error.code, HEADER_MISMATCH);
        assert!(e.error.message.contains("search_people") && e.error.message.contains("withdraw_proposal"), "{}", e.error.message);
        // resources/read is named by its uri
        let read = json!({"method": "resources/read", "params": {"uri": "family-quilt://guide", "_meta": {META_VERSION: "2026-07-28"}}});
        assert!(check_modern(&read, &headers(Some("2026-07-28"), Some("resources/read"), Some("family-quilt://guide"))).is_ok());
        assert_eq!(code(&read, &headers(Some("2026-07-28"), Some("resources/read"), Some("family-quilt://other"))).error.code, HEADER_MISMATCH);
    }

    #[test]
    fn a_modern_body_without_its_version_in_meta_is_invalid_params() {
        let msg = json!({"method": "tools/list", "params": {}});
        let e = check_modern(&msg, &headers(Some("2026-07-28"), Some("tools/list"), None)).unwrap_err();
        assert_eq!((e.status, e.error.code), (400, INVALID_PARAMS));
        assert!(e.error.message.contains(META_VERSION));
    }

    #[test]
    fn a_base64_name_header_is_decoded_before_comparing() {
        let name = "Zoë's tool";
        let enc = encode_header_value(name);
        assert!(enc.starts_with("=?base64?") && enc.ends_with("?="));
        let h = headers(Some("2026-07-28"), Some("tools/call"), Some(&enc));
        assert!(check_modern(&call("2026-07-28", name), &h).is_ok());
        let broken = headers(Some("2026-07-28"), Some("tools/call"), Some("=?base64?***?="));
        assert_eq!(check_modern(&call("2026-07-28", name), &broken).unwrap_err().error.code, HEADER_MISMATCH);
    }

    #[test]
    fn modern_results_say_what_they_are_and_lists_say_how_long_they_keep() {
        let r = decorate_modern("tools/list", json!({"tools": []}));
        assert_eq!(r["resultType"], "complete");
        assert_eq!(r["_meta"][META_SERVER_INFO]["name"], "family-quilt");
        assert!(r["ttlMs"].is_u64() && r["cacheScope"] == "private");
        // A tool call is not cacheable, and an existing `_meta` is kept.
        let c = decorate_modern("tools/call", json!({"content": [], "_meta": {"mine": 1}}));
        assert!(c.get("ttlMs").is_none() && c.get("cacheScope").is_none());
        assert_eq!((c["_meta"]["mine"].as_i64(), c["resultType"].as_str()), (Some(1), Some("complete")));
        for m in CACHEABLE {
            assert!(decorate_modern(m, json!({}))["ttlMs"].is_u64(), "{m}");
        }
        assert!(MODERN_METHODS.contains(&"server/discover") && !MODERN_METHODS.contains(&"ping") && !MODERN_METHODS.contains(&"initialize"));
    }

    #[test]
    fn a_session_id_is_the_client_name_and_nothing_else_is_accepted() {
        for name in ["claude", "Claude Desktop 1.2", "Zoë", "x"] {
            let id = session_id(name);
            assert!(id.bytes().all(|b| (0x21..=0x7e).contains(&b)), "visible ASCII only: {id}");
            assert_eq!(client_of_session(&id).as_deref(), Some(name));
        }
        for junk in ["", "fq-", "fq-0", "fq-zz", "0123456789abcdef0123456789abcdef", "fq-ff", "fq-00", "FQ-6162"] {
            assert_eq!(client_of_session(junk), None, "{junk}");
        }
    }

    #[test]
    fn base64_round_trips_every_length_and_any_utf8() {
        for s in ["", "a", "ab", "abc", "abcd", "abcde", "Hello, 世界", "line1\nline2", " padded "] {
            assert_eq!(b64_decode(&b64_encode(s.as_bytes())).unwrap(), s.as_bytes(), "{s:?}");
        }
        assert_eq!(b64_encode(b"Hello, \xe4\xb8\x96\xe7\x95\x8c"), "SGVsbG8sIOS4lueVjA==", "matches the spec's own example");
        assert_eq!(b64_decode("not*base64"), None);
    }

    #[test]
    fn header_values_stay_plain_unless_plain_would_be_unsafe() {
        assert_eq!(encode_header_value("get_weather"), "get_weather");
        assert_eq!(encode_header_value("file:///projects/app/config.json"), "file:///projects/app/config.json");
        for tricky in ["Hello, 世界", " padded ", "line1\nline2", "", "=?base64?literal?="] {
            let enc = encode_header_value(tricky);
            assert!(is_sentinel(&enc), "{tricky:?} -> {enc}");
            assert_eq!(decode_header_value(&enc).as_deref(), Some(tricky));
        }
        assert_eq!(encode_header_value("=?base64?literal?="), "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?=", "the spec's example");
        assert_eq!(decode_header_value("plain").as_deref(), Some("plain"));
        assert_eq!(decode_header_value("=?base64?***?="), None);
    }
}
