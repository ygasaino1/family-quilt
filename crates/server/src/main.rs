//! `family-app`. Headless for now (`--data DIR`); the first-run wizard and unlock screen are in the page, the window comes next. `--mcp-stdio` is the bridge for agent apps that start local MCP servers.

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
    std::env::var("LOCALAPPDATA").or_else(|_| std::env::var("XDG_DATA_HOME")).map(|p| PathBuf::from(p).join("FamilyTree")).unwrap_or_else(|_| PathBuf::from("data"))
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
    std::fs::create_dir_all(&dir).expect("cannot create the data folder");
    let db = dir.join("tree.sqlite");
    let tok = token::load_or_create(&dir).expect("cannot create the agent token");
    let state = AppState::new_closed(tok, dir.clone(), db, args.iter().any(|a| a == "--fast-kdf-for-tests"));
    // An unprotected tree opens at once; a protected one waits for the unlock screen; a missing one for the wizard.
    if let Err(e) = tree::open_if_plain(&state) {
        eprintln!("Cannot read the tree: {}", e.message);
        std::process::exit(1);
    }
    let (port, handle) = serve(state, args.iter().position(|a| a == "--port").and_then(|i| args.get(i + 1)).and_then(|p| p.parse().ok()).unwrap_or(0)).await.expect("cannot start the server");
    token::write_endpoint(&dir, port).ok();
    eprintln!("Family tree server on http://127.0.0.1:{port} (data: {})", dir.display());
    let _ = tokio::signal::ctrl_c().await;
    let _ = std::fs::remove_file(token::endpoint_path(&dir));
    handle.abort();
}
