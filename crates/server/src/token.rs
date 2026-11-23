//! The agent token and the endpoint file the stdio bridge reads.

use std::io;
use std::path::{Path, PathBuf};

pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-token.txt")
}
pub fn endpoint_path(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-endpoint.json")
}

fn random_token() -> io::Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// `FAMILY_AGENT_TOKEN` wins; otherwise the token file (created once).
pub fn load_or_create(data_dir: &Path) -> io::Result<String> {
    if let Ok(t) = std::env::var("FAMILY_AGENT_TOKEN") {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    let p = token_path(data_dir);
    if let Ok(t) = std::fs::read_to_string(&p) {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    std::fs::create_dir_all(data_dir)?;
    let t = random_token()?;
    std::fs::write(&p, &t)?;
    Ok(t)
}

/// Tell the bridge where the running app listens. Removed on clean shutdown by the app.
pub fn write_endpoint(data_dir: &Path, port: u16) -> io::Result<()> {
    std::fs::write(endpoint_path(data_dir), format!("{{\"port\": {port}, \"token_file\": {:?}}}", token_path(data_dir).to_string_lossy()))
}

/// A fresh random secret for one run of the app (the UI cookie).
pub fn random_launch_token() -> io::Result<String> {
    random_token()
}

/// Is another copy of the app already serving this data folder? (Its endpoint file names a port that answers.)
pub fn already_running(data_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(endpoint_path(data_dir)) else { return false };
    let Some(port) = text.split("\"port\":").nth(1).and_then(|r| r.trim_start().split(|c: char| !c.is_ascii_digit()).next()).and_then(|p| p.parse::<u16>().ok()) else {
        return false;
    };
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), std::time::Duration::from_millis(400)).is_ok()
}

/// Open `url` in the default web browser (the `--browser` mode and the fallback when no window is built in).
pub fn open_in_browser(url: &str) -> io::Result<()> {
    #[cfg(windows)]
    let mut c = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut c = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut c = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    c.spawn().map(|_| ())
}
