//! `family-review`: everything between "an agent sends JSON" and "the human approves it".
//!
//! * [`fields`] which fields an agent may propose to change, and how to read/apply them
//! * [`submit`] strict validation + storing a proposal (people, links, changes) as pending data
//! * [`review`] the review view: before/after per field, conflicts, warnings on the resulting state
//! * [`apply`] approve (accepted changes + new records, one transaction), reject
//! * [`query`] read side for agents and the UI (search, person, relationship, graph neighbourhood)
//!
//! No HTTP here: the server and the MCP endpoint are thin layers over these functions, so they
//! inherit the same limits. There is deliberately no function that edits or deletes verified data
//! on an agent's behalf.

pub mod apply;
pub mod fields;
pub mod guide;
pub mod query;
pub mod review;
pub mod submit;

use serde::Serialize;

/// A problem in agent input that the agent can act on.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FieldError {
    /// e.g. `people[0].sex` or `changes[2].fields.birth_date.from`
    pub path: String,
    /// Stable machine-readable code.
    pub code: &'static str,
    pub message: String,
}

impl FieldError {
    pub fn new(path: impl Into<String>, code: &'static str, message: impl Into<String>) -> Self {
        FieldError { path: path.into(), code, message: message.into() }
    }
}

/// Everything that can go wrong in this crate.
#[derive(Debug)]
pub enum ReviewError {
    /// Input problems (HTTP 422 / MCP tool error with the list).
    Invalid(Vec<FieldError>),
    /// Not found / already decided / locked etc. from the store.
    Store(family_store::StoreError),
}

impl From<family_store::StoreError> for ReviewError {
    fn from(e: family_store::StoreError) -> Self {
        ReviewError::Store(e)
    }
}

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReviewError::Invalid(v) => {
                write!(f, "invalid proposal: ")?;
                for (i, e) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{}: {}", e.path, e.message)?;
                }
                Ok(())
            }
            ReviewError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReviewError {}

pub type Result<T> = std::result::Result<T, ReviewError>;

/// Agent names become `created_by = "agent:<name>"`; keep them short and boring.
pub fn clean_agent_name(raw: &str) -> String {
    let s: String = raw.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '.' | '-')).take(40).collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "unnamed".to_string()
    } else {
        s
    }
}
