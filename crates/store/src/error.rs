//! One error type for the whole store layer.
//!
//! The UI translates errors itself (section 20 of the spec), so every error has
//! a **stable code** and **params**; the English `Display` text is only for logs.

use serde_json::{json, Value};

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{kind} not found: {id}")]
    NotFound { kind: &'static str, id: String },

    /// A field has a value that is not allowed (bad enum word, bad date, empty name ...).
    #[error("invalid value for {field}: {reason}")]
    Invalid { field: String, reason: String },

    #[error("this link would make a person their own ancestor")]
    Cycle,

    #[error("that {what} already exists")]
    Duplicate { what: &'static str, id: String },

    /// Death date and "living" contradict each other, etc.
    #[error("conflicting fields: {0}")]
    Conflict(String),

    // ---- protection (section 16) ----
    #[error("this tree is protected by a password")]
    PasswordRequired,
    #[error("wrong password or recovery key")]
    WrongPassword,
    #[error("the key file next to this tree is missing or damaged: {0}")]
    KeyFile(String),
    #[error("password protection is not available in this build")]
    EncryptionUnavailable,
    #[error("a tree already exists at {0}")]
    AlreadyExists(String),
    #[error("no tree found at {0}")]
    TreeMissing(String),
    #[error("a password cannot be empty")]
    EmptyPassword,

    // ---- infrastructure ----
    #[error("the file was made by a newer version of the app (schema {found}, this app understands up to {supported})")]
    SchemaTooNew { found: u32, supported: u32 },
    #[error("stored value is damaged: {0}")]
    Corrupt(String),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl StoreError {
    pub fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        StoreError::Invalid { field: field.into(), reason: reason.into() }
    }

    /// Stable machine-readable code (never changes once released).
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::NotFound { .. } => "not_found",
            StoreError::Invalid { .. } => "invalid_value",
            StoreError::Cycle => "link_cycle",
            StoreError::Duplicate { .. } => "duplicate",
            StoreError::Conflict(_) => "conflicting_fields",
            StoreError::PasswordRequired => "locked",
            StoreError::WrongPassword => "wrong_password",
            StoreError::KeyFile(_) => "key_file_problem",
            StoreError::EncryptionUnavailable => "encryption_unavailable",
            StoreError::AlreadyExists(_) => "tree_exists",
            StoreError::TreeMissing(_) => "tree_missing",
            StoreError::EmptyPassword => "empty_password",
            StoreError::SchemaTooNew { .. } => "schema_too_new",
            StoreError::Corrupt(_) => "stored_value_damaged",
            StoreError::Sqlite(_) => "database_error",
            StoreError::Io(_) => "file_error",
            StoreError::Json(_) => "json_error",
        }
    }

    /// Parameters for the translated message. Never contains secrets.
    pub fn params(&self) -> Value {
        match self {
            StoreError::NotFound { kind, id } => json!({ "kind": kind, "id": id }),
            StoreError::Invalid { field, reason } => json!({ "field": field, "reason": reason }),
            StoreError::Duplicate { what, id } => json!({ "what": what, "id": id }),
            StoreError::SchemaTooNew { found, supported } => json!({ "found": found, "supported": supported }),
            StoreError::AlreadyExists(p) | StoreError::TreeMissing(p) => json!({ "path": p }),
            _ => json!({}),
        }
    }
}
