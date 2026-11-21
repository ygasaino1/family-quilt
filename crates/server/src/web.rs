//! The user interface: plain files from `web/`, compiled into the executable.
//! Set `FAMILY_WEB_DIR` to serve them from disk instead (for development and tests).

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

macro_rules! asset {
    ($p:literal) => {
        ($p, include_bytes!(concat!("../../../web/", $p)) as &[u8])
    };
}

const ASSETS: &[(&str, &[u8])] = &[
    asset!("index.html"),
    asset!("app.css"),
    asset!("locales/en.json"),
    asset!("js/main.js"),
    asset!("js/i18n.js"),
    asset!("js/api.js"),
    asset!("js/quilt.js"),
    asset!("js/view.js"),
    asset!("js/dom.js"),
    asset!("js/prefs.js"),
    asset!("js/minimap.js"),
    asset!("js/sidebar.js"),
    asset!("js/review.js"),
    asset!("js/settings.js"),
    asset!("js/export.js"),
];

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

pub async fn serve_asset(uri: Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() {
        path = "index.html".into();
    }
    if path.contains("..") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let bytes: Option<Vec<u8>> = match std::env::var("FAMILY_WEB_DIR") {
        Ok(dir) => std::fs::read(std::path::Path::new(&dir).join(&path)).ok(),
        Err(_) => ASSETS.iter().find(|(p, _)| *p == path).map(|(_, b)| b.to_vec()),
    };
    match bytes {
        Some(b) => (
            [
                (header::CONTENT_TYPE, mime(&path)),
                (header::CACHE_CONTROL, "no-store"),
                (header::CONTENT_SECURITY_POLICY, "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            b,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
