use serde::Serialize;
use serde_json::{json, Value};

/// Stable editor API error codes known to the Phase 6C protocol skeleton.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    InvalidRequest,
    LoadFailed,
    ValidationFailed,
    InternalError,
}

/// JSON-serializable API failure payload used inside `ok: false` envelopes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    pub details: Value,
}

impl ApiError {
    /// Build an `invalid_request` error with default empty details.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: ApiErrorCode::InvalidRequest,
            message: message.into(),
            details: json!({}),
        }
    }

    /// Build an `invalid_request` error with Python-compatible structured details.
    pub fn invalid_request_with_details(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: ApiErrorCode::InvalidRequest,
            message: message.into(),
            details,
        }
    }

    /// Build a `load_failed` error for one-shot authored YAML load/emit failures.
    pub fn load_failed(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: ApiErrorCode::LoadFailed,
            message: message.into(),
            details,
        }
    }

    /// Build a `validation_failed` error for unexpected validation request failures.
    pub fn validation_failed(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: ApiErrorCode::ValidationFailed,
            message: message.into(),
            details,
        }
    }

    /// Convert the error into the Python-compatible JSON object shape.
    pub fn to_value(&self) -> Value {
        json!({
            "code": self.code,
            "message": self.message,
            "details": self.details,
        })
    }
}
