# Porting checklist (Windows first; macOS and Linux later)

Core crates are OS-neutral: `PathBuf` everywhere, no hard-coded separators, paths passed to SQL must be UTF-8 (checked, clear error otherwise), tested with Persian folder names. OS-specific code will live in one `platform` module (Phase 5/6).

| Topic | Windows (now) | macOS | Linux |
|---|---|---|---|
| Web view | WebView2 (check, friendly message + link) | WKWebView | WebKitGTK |
| App data | `%APPDATA%` (or `data/` next to exe with a `portable` marker) via `dirs` | `~/Library/Application Support` | `~/.local/share` |
| Single instance | named mutex / local socket | local socket | local socket |
| C runtime | static (`.cargo/config.toml`, `+crt-static`), verify with `dumpbin /dependents` | n/a | n/a |
| SQLCipher/OpenSSL | vendored OpenSSL (needs Perl at build time); verify build | same crate feature | same crate feature (verified) |
| Signing | unsigned exe triggers SmartScreen; needs a code-signing certificate | Developer ID + notarisation or Gatekeeper blocks it | not required |
