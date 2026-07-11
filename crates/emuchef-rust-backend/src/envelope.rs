use serde_json::{json, Value};

use crate::errors::ApiError;

/// Create the stable success envelope used by one-shot responses.
pub fn success(result: Value) -> Value {
    json!({
        "ok": true,
        "result": result,
    })
}

/// Create the stable failure envelope used by one-shot responses.
pub fn failure(error: ApiError) -> Value {
    json!({
        "ok": false,
        "error": error.to_value(),
    })
}

/// Add the JSONL sidecar request id around a protocol envelope.
///
/// Invalid ids and malformed JSONL lines pass `None`, which serializes as
/// `id: null` for requests that do not supply a transport identifier.
pub fn with_id(response: Value, id: Option<String>) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("id".to_string(), id.map_or(Value::Null, Value::String));
    if let Value::Object(response_object) = response {
        object.extend(response_object);
    }
    Value::Object(object)
}
