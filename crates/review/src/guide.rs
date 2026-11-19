//! The agent guide text (served at `/agent/v1/guide`, exposed as an MCP resource, and written to `AGENTS.md`).

pub const GUIDE: &str = include_str!("guide.md");
