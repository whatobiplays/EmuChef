//! Pure detected-device profile matching for future Rust planner ownership.
//!
//! This module mirrors the current Python mismatch-warning criteria without
//! probing devices or wiring warnings into planner routes. Callers must provide
//! already-detected facts and already-loaded authored profile criteria.

use regex::Regex;
use serde_json::json;

use crate::device_probe::DetectedDeviceFacts;
use crate::planner::PlannerMessage;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeviceProfileMatchCriteria {
    pub manufacturer_contains: Vec<String>,
    pub brand_contains: Vec<String>,
    pub model_patterns: Vec<String>,
    pub android_version: Option<AndroidVersionRangeCriteria>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AndroidVersionRangeCriteria {
    pub min: Option<i64>,
    pub max: Option<i64>,
}

pub(crate) fn build_detected_device_profile_mismatch_warning(
    device_plan_ref: &str,
    profile_ref: &str,
    detected_facts: &DetectedDeviceFacts,
    profile_match: &DeviceProfileMatchCriteria,
) -> Option<PlannerMessage> {
    let match_result = match_detected_device_profile(detected_facts, profile_match);
    if match_result.matched {
        return None;
    }

    Some(PlannerMessage {
        code: "device_profile_mismatch".to_string(),
        message: format!(
            "Selected device plan profile {} does not match the detected device {} {}.",
            python_repr_string(profile_ref),
            detected_facts.manufacturer.as_deref().unwrap_or("Unknown"),
            detected_facts.model.as_deref().unwrap_or("Unknown"),
        ),
        details: json!({
            "device_plan_ref": device_plan_ref,
            "device_profile_ref": profile_ref,
            "serial": detected_facts.serial,
            "manufacturer": detected_facts.manufacturer,
            "brand": detected_facts.brand,
            "model": detected_facts.model,
            "android_version": detected_facts.android_version,
            "reasons": match_result.reasons,
        }),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProfileMatchResult {
    matched: bool,
    reasons: Vec<String>,
}

fn match_detected_device_profile(
    detected_facts: &DetectedDeviceFacts,
    profile_match: &DeviceProfileMatchCriteria,
) -> ProfileMatchResult {
    let mut reasons = Vec::new();
    let mut matched = true;

    if let Some(manufacturer) = &detected_facts.manufacturer {
        if !profile_match.manufacturer_contains.is_empty() {
            let expected = &profile_match.manufacturer_contains;
            if contains_any_case_insensitive(manufacturer, expected) {
                reasons.push(format!(
                    "manufacturer matched one of: {}",
                    expected.join(", ")
                ));
            } else {
                matched = false;
                reasons.push(format!(
                    "manufacturer {} did not contain any of: {}",
                    python_repr_string(manufacturer),
                    expected.join(", ")
                ));
            }
        }
    }

    if let Some(brand) = &detected_facts.brand {
        if !profile_match.brand_contains.is_empty() {
            let expected = &profile_match.brand_contains;
            if contains_any_case_insensitive(brand, expected) {
                reasons.push(format!("brand matched one of: {}", expected.join(", ")));
            } else {
                matched = false;
                reasons.push(format!(
                    "brand {} did not contain any of: {}",
                    python_repr_string(brand),
                    expected.join(", ")
                ));
            }
        }
    }

    if let Some(model) = &detected_facts.model {
        if !profile_match.model_patterns.is_empty() {
            let patterns = &profile_match.model_patterns;
            if patterns.iter().any(|pattern| {
                Regex::new(pattern)
                    .map(|regex| regex.is_match(model))
                    .unwrap_or(false)
            }) {
                reasons.push(format!("model matched one of: {}", patterns.join(", ")));
            } else {
                matched = false;
                reasons.push(format!(
                    "model {} did not match any of: {}",
                    python_repr_string(model),
                    patterns.join(", ")
                ));
            }
        }
    }

    if let Some(android_version) = detected_facts.android_version {
        let minimum_android = profile_match
            .android_version
            .as_ref()
            .and_then(|criteria| criteria.min);
        if let Some(minimum_android) = minimum_android {
            if android_version >= minimum_android {
                reasons.push(format!(
                    "android version {android_version} met minimum {minimum_android}"
                ));
            } else {
                matched = false;
                reasons.push(format!(
                    "android version {android_version} was below minimum {minimum_android}"
                ));
            }
        }
    }

    ProfileMatchResult { matched, reasons }
}

fn contains_any_case_insensitive(value: &str, expected_tokens: &[String]) -> bool {
    let folded_value = value.to_lowercase();
    expected_tokens
        .iter()
        .any(|token| folded_value.contains(&token.to_lowercase()))
}

fn python_repr_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}
