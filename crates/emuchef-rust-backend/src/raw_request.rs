//! Duplicate-aware JSON parsing for protocol request ingress.
//!
//! `serde_json::Value` uses a map for objects and therefore cannot report a
//! duplicate key after parsing. This module temporarily preserves object
//! entries in arrival order, rejects duplicate runtime binding keys, and only
//! then converts compatible requests to the existing `Value` dispatch shape.

use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{json, Map, Number, Value};

use crate::errors::ApiError;

#[derive(Debug, PartialEq)]
enum RawJsonValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<RawJsonValue>),
    Object(Vec<(String, RawJsonValue)>),
}

impl<'de> Deserialize<'de> for RawJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawJsonVisitor)
    }
}

struct RawJsonVisitor;

impl<'de> Visitor<'de> for RawJsonVisitor {
    type Value = RawJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(RawJsonValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(RawJsonValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(RawJsonValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(RawJsonValue::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(RawJsonValue::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(RawJsonValue::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(RawJsonValue::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(RawJsonValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(RawJsonValue::Array(values))
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        while let Some(key) = mapping.next_key()? {
            entries.push((key, mapping.next_value()?));
        }
        Ok(RawJsonValue::Object(entries))
    }
}

impl RawJsonValue {
    fn object_field(&self, field: &str) -> Option<&Self> {
        let Self::Object(entries) = self else {
            return None;
        };
        entries
            .iter()
            .rev()
            .find_map(|(key, value)| (key == field).then_some(value))
    }

    fn string_value(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    fn duplicate_object_key(&self) -> Option<&str> {
        let Self::Object(entries) = self else {
            return None;
        };
        let mut seen = std::collections::HashSet::new();
        entries
            .iter()
            .find_map(|(key, _)| (!seen.insert(key.as_str())).then_some(key.as_str()))
    }

    fn runtime_duplicate_binding_key(&self) -> Option<&str> {
        let request_type = self.object_field("type")?.string_value()?;
        if !matches!(request_type, "describeConfiguration" | "planConfiguration") {
            return None;
        }
        let payload = self.object_field("payload")?;
        if let Some(key) = payload
            .object_field("bindings")
            .and_then(Self::duplicate_object_key)
        {
            return Some(key);
        }
        payload
            .object_field("userConfiguration")
            .and_then(|configuration| configuration.object_field("bindings"))
            .and_then(Self::duplicate_object_key)
    }

    fn request_id(&self) -> Option<String> {
        self.object_field("id")
            .and_then(Self::string_value)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string)
    }

    fn into_json_value(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) => Value::Number(value),
            Self::String(value) => Value::String(value),
            Self::Array(values) => {
                Value::Array(values.into_iter().map(Self::into_json_value).collect())
            }
            Self::Object(entries) => {
                let mut result = Map::new();
                for (key, value) in entries {
                    result.insert(key, value.into_json_value());
                }
                Value::Object(result)
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RawRequestError {
    InvalidJson,
    DuplicateBindingKey {
        request_id: Option<String>,
        key: String,
    },
}

impl RawRequestError {
    pub(crate) fn request_id(&self) -> Option<String> {
        match self {
            Self::InvalidJson => None,
            Self::DuplicateBindingKey { request_id, .. } => request_id.clone(),
        }
    }

    pub(crate) fn api_error(&self, invalid_json_message: &'static str) -> ApiError {
        match self {
            Self::InvalidJson => ApiError::invalid_request(invalid_json_message),
            Self::DuplicateBindingKey { key, .. } => ApiError::invalid_request_with_details(
                "Request field 'bindings' contains a duplicate key.",
                json!({
                    "reason": "duplicate_binding_key",
                    "field": "bindings",
                    "key": key,
                }),
            ),
        }
    }
}

/// Parse one raw request and reject duplicate runtime binding keys before map conversion.
pub(crate) fn parse(raw: &str) -> Result<Value, RawRequestError> {
    let value =
        serde_json::from_str::<RawJsonValue>(raw).map_err(|_| RawRequestError::InvalidJson)?;
    if let Some(key) = value.runtime_duplicate_binding_key() {
        return Err(RawRequestError::DuplicateBindingKey {
            request_id: value.request_id(),
            key: key.to_string(),
        });
    }
    Ok(value.into_json_value())
}
