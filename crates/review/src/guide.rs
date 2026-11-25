//! The agent guide text (served at `/agent/v1/guide`, exposed as an MCP resource, and written to `AGENTS.md`).

pub const GUIDE: &str = include_str!("guide.md");

/// The short version for read-only work (served as `family-quilt://guide/reading` and `/agent/v1/guide/reading`).
pub const GUIDE_READING: &str = include_str!("guide_reading.md");
