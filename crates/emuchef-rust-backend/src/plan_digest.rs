//! Canonical execution-plan hashing for review-to-apply integrity checks.
//!
//! A digest is the lowercase SHA-256 hex encoding of UTF-8 JSON produced after
//! recursively sorting every object key lexicographically. Array order is
//! preserved because recipe groups and steps are already normalized plan data.
//! JSON is emitted without insignificant whitespace.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::planner::ExecutionPlan;

/// Compute the canonical JSON SHA-256 digest for an execution plan.
pub(crate) fn execution_plan_digest(plan: &ExecutionPlan) -> Result<String, serde_json::Error> {
    let canonical = canonicalize(serde_json::to_value(plan)?);
    let bytes = serde_json::to_vec(&canonical)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::canonicalize;

    #[test]
    fn canonical_json_sorts_object_keys_and_preserves_array_order() {
        assert_eq!(
            serde_json::to_string(&canonicalize(json!({
                "z": {"b": 2, "a": 1},
                "a": [{"d": 4, "c": 3}, 2, 1]
            })))
            .unwrap(),
            r#"{"a":[{"c":3,"d":4},2,1],"z":{"a":1,"b":2}}"#
        );
    }
}
