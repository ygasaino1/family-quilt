//! JSON errors for both doors: `{"error": {"code", "message", "fields": [...]}}`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use family_review::{FieldError, ReviewError};
use family_store::StoreError;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
    pub fields: Vec<FieldError>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: &str) -> Self {
        ApiError { status, code: code.into(), message: message.into(), fields: vec![] }
    }
    pub fn bad_request(message: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }
    pub fn body(&self) -> serde_json::Value {
        json!({"error": {"code": self.code, "message": self.message, "fields": self.fields}})
    }
    pub fn into_response_inner(self) -> Response {
        (self.status, axum::Json(self.body())).into_response()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        self.into_response_inner()
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        let status = match &e {
            StoreError::NotFound { .. } => StatusCode::NOT_FOUND,
            StoreError::Invalid { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            StoreError::Cycle | StoreError::Duplicate { .. } | StoreError::Conflict(_) => StatusCode::CONFLICT,
            StoreError::PasswordRequired | StoreError::WrongPassword | StoreError::KeyFile(_) => StatusCode::LOCKED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        // Never leak internals for server-side faults.
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR { "The tree could not be read or saved. Nothing was changed.".to_string() } else { e.to_string() };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("store error: {e}");
        }
        ApiError { status, code: e.code().into(), message, fields: vec![] }
    }
}

impl From<ReviewError> for ApiError {
    fn from(e: ReviewError) -> Self {
        match e {
            ReviewError::Invalid(fields) => {
                ApiError { status: StatusCode::UNPROCESSABLE_ENTITY, code: "invalid_proposal".into(), message: "The request has problems; see `fields`. Nothing was stored.".into(), fields }
            }
            ReviewError::Store(s) => s.into(),
        }
    }
}
