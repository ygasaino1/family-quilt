//! `family-app`: by default opens its own window (build with `--features window`); `--browser` uses the default
//! browser; `--headless` only serves (tests, development). `--mcp-stdio` is the bridge for agent apps.

// Without a console, a Windows release build would show nothing on errors; messages go to dialogs instead.
#![cfg_attr(all(windows, feature = "window", not(debug_assertions)), windows_subsystem = "windows")]

use family_server::{bridge, serve, token, tree, AppState};
use std::path::PathBuf;

fn data_dir(args: &[String]) -> PathBuf {
    if let Some(i) = args.iter().position(|a| a == "--data") {
        if let Some(p) = args.get(i + 1) {
            return PathBuf::from(p);
        }
    }
    if let Ok(p) = std::env::var("FAMILY_DATA_DIR") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable").exists() {
                return dir.join("data");
            }
        }
    }
    std::env::var("APPDATA")
        .or_else(|_| std::env::var("XDG_DATA_HOME"))
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.local/share")))
        .map(|p| PathBuf::from(p).join("FamilyTree"))
        .unwrap_or_else(|_| PathBuf::from("data"))
}

/// Tell the person something when there may be no console (a double-clicked Windows exe).
fn tell(title: &str, text: &str) {
    eprintln!("{title}: {text}");
    #[cfg(feature = "window")]
    family_server::window::message(title, text);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = data_dir(&args);
    if args.iter().any(|a| a == "--mcp-stdio") {
        bridge::run(&dir);
        return;
    }
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    let headless = args.iter().any(|a| a == "--headless");
    let use_browser = args.iter().any(|a| a == "--browser") || (!headless && !cfg!(feature = "window"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tell("Family Tree", &format!("The data folder cannot be created ({}): {e}", dir.display()));
        std::process::exit(1);
    }
    if !headless && token::already_running(&dir) {
        tell("Family Tree", "Family Tree is already open. Look for its window (or close it first).");
        std::process::exit(1);
    }
    let db = dir.join("tree.sqlite");
    let tok = token::load_or_create(&dir).expect("cannot create the agent token");
    let state = AppState::new_closed(tok, dir.clone(), db, args.iter().any(|a| a == "--fast-kdf-for-tests"));
    // The window or browser reaches /api only with this run's cookie; headless runs (tests) have none.
    let launch = if headless { None } else { token::random_launch_token().ok() };
    if let Some(t) = &launch {
        let _ = state.ui_token.set(t.clone());
    }
    // An unprotected tree opens at once; a protected one waits for the unlock screen; a missing one for the wizard.
    if let Err(e) = tree::open_if_plain(&state) {
        tell("Family Tree", &format!("The tree cannot be read: {}", e.message));
        std::process::exit(1);
    }
    let want_port = args.iter().position(|a| a == "--port").and_then(|i| args.get(i + 1)).and_then(|p| p.parse().ok()).unwrap_or(0);
    let (port, handle) = match serve(state.clone(), want_port).await {
        Ok(x) => x,
        Err(e) => {
            tell("Family Tree", &format!("The program could not start its local service: {e}"));
            std::process::exit(1);
        }
    };
    token::write_endpoint(&dir, port).ok();
    eprintln!("Family tree server on http://127.0.0.1:{port} (data: {})", dir.display());
    let url = match &launch {
        Some(t) => format!("http://127.0.0.1:{port}/?t={t}"),
        None => format!("http://127.0.0.1:{port}/"),
    };
    if headless {
        let _ = tokio::signal::ctrl_c().await;
    } else if use_browser {
        if let Err(e) = token::open_in_browser(&url) {
            eprintln!("Could not open the browser ({e}). Open this address yourself: {url}");
        }
        let _ = tokio::signal::ctrl_c().await;
    } else {
        #[cfg(feature = "window")]
        if let Err(msg) = family_server::window::run(&url, "Family Tree") {
            tell("Family Tree", &msg);
        }
    }
    // Closing: lock (flushes and closes the file, zeroes the key), remove the endpoint file, stop serving.
    let st = state.clone();
    let _ = tokio::task::spawn_blocking(move || tree::lock_now(&st)).await;
    let _ = std::fs::remove_file(token::endpoint_path(&dir));
    handle.abort();
}
