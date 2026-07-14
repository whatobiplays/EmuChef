use serde::Serialize;
use serde_json::{json, Value};

/// Stable error codes exposed by the JSON protocol surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    InvalidRequest,
    InvalidCommand,
    CommandFailed,
    LoadFailed,
    SaveFailed,
    UnknownDocument,
    ValidationFailed,
    InternalError,
    InvalidExecutionPlan,
    PlanDigestMismatch,
    TargetDeviceMismatch,
    ExecutionInProgress,
    UnknownExecution,
    ExecutionStartFailed,
    LaunchUnavailable,
    LaunchFailed,
}

/// JSON-serializable API failure payload used inside `ok: false` envelopes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    pub details: Value,
}

impl ApiError {
    /// Build a failure with a product-protocol code and structured details.
    pub fn new(code: ApiErrorCode, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details,
        }
    }
    /// Build an `invalid_request` error with default empty details.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: ApiErrorCode::InvalidRequest,
            message: message.into(),
            details: json!({}),
        }
    }

    /// Build an `invalid_request` error with stable structured details.
    pub fn invalid_request_with_details(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: ApiErrorCode::InvalidRequest,
            message: message.into(),
            details,
        }
    }

    /// Build an `invalid_command` error with stable structured details.
    pub fn invalid_command_with_details(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: ApiErrorCode::InvalidCommand,
            message: message.into(),
            details,
        }
    }

    /// Build a `command_failed` error for command execution failures.
    pub fn command_failed(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: ApiErrorCode::CommandFailed,
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

    /// Build a `save_failed` error for document save failures.
    pub fn save_failed(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: ApiErrorCode::SaveFailed,
            message: message.into(),
            details,
        }
    }

    /// Build an `unknown_document` error for missing sidecar document sessions.
    pub fn unknown_document(document_id: impl Into<String>) -> Self {
        let document_id = document_id.into();
        Self {
            code: ApiErrorCode::UnknownDocument,
            message: format!("Unknown document id: {document_id}"),
            details: json!({ "documentId": document_id }),
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

    /// Convert the error into the protocol JSON object shape.
    pub fn to_value(&self) -> Value {
        json!({
            "code": self.code,
            "message": self.message,
            "details": self.details,
        })
    }
}
