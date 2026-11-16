//! Complete Device Profile eligibility, independent of Device Plan availability.

use crate::{catalog_source::CatalogSnapshot, device_probe::DetectedDeviceFacts, errors::ApiError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ProfileMatchYaml {
    #[serde(default)]
    pub(crate) manufacturer_contains: Vec<String>,
    #[serde(default)]
    pub(crate) brand_contains: Vec<String>,
    #[serde(default)]
    pub(crate) model_patterns: Vec<String>,
    pub(crate) android_version: Option<AndroidRangeYaml>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct AndroidRangeYaml {
    pub(crate) min: Option<i64>,
    pub(crate) max: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct ProfileYaml {
    id: String,
    name: String,
    description: Option<String>,
    #[serde(rename = "match", default)]
    match_: ProfileMatchYaml,
    #[serde(default)]
    metadata: Value,
}

#[derive(Clone, Debug, Deserialize)]
struct PlanYaml {
    id: String,
    name: String,
    description: Option<String>,
    device_profile_ref: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MatchEvidence {
    pub(crate) matched: usize,
    pub(crate) declared: usize,
    pub(crate) missing: usize,
    pub(crate) conflicts: usize,
    pub(crate) family_matches: usize,
    pub(crate) family_conflicts: usize,
    pub(crate) android_conflict: bool,
    pub(crate) reasons: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedProfile {
    pub profile_id: String,
    pub profile_name: String,
    pub profile_description: Option<String>,
    pub confidence: String,
    pub generic: bool,
    pub eligibility: Eligibility,
    pub requires_explicit_choice: bool,
    pub reasons: Vec<String>,
    pub plans: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Eligibility {
    Eligible,
    Incomplete,
    Incompatible,
}

pub(crate) fn match_device(
    snapshot: &CatalogSnapshot,
    facts: &DetectedDeviceFacts,
) -> Result<Value, ApiError> {
    let profiles: Vec<ProfileYaml> = load_yaml_directory(snapshot.root(), "device_profiles")?;
    let plans: Vec<PlanYaml> = load_yaml_directory(snapshot.root(), "device_plans")?;
    let mut resolved = Vec::new();
    let explicit_only_model = facts.model.as_deref().is_some_and(|model| {
        profiles.iter().any(|profile| {
            profile_requires_explicit_selection(profile)
                && profile
                    .match_
                    .model_patterns
                    .iter()
                    .any(|pattern| regex::Regex::new(pattern).is_ok_and(|r| r.is_match(model)))
        })
    });
    for profile in profiles {
        let evidence = evaluate_profile(facts, &profile.match_);
        let generic = profile_is_generic(&profile);
        let eligibility = if evidence.conflicts > 0 || evidence.declared == 0 {
            Eligibility::Incompatible
        } else if evidence.missing > 0 {
            Eligibility::Incomplete
        } else {
            Eligibility::Eligible
        };
        let confidence = if evidence.missing == 0 {
            "exact"
        } else if evidence.matched >= 2 {
            "high"
        } else {
            "low"
        };
        let profile_plans = plans.iter().filter(|plan| plan.device_profile_ref == profile.id).map(|plan| json!({
            "planId": plan.id, "name": plan.name, "description": plan.description,
            "profileId": profile.id, "profileName": profile.name,
            "profileDescription": profile.description, "confidence": confidence,
            "reasons": evidence.reasons, "requiresExplicitChoice": generic || profile_requires_explicit_selection(&profile) || explicit_only_model,
        })).collect();
        resolved.push(ResolvedProfile {
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            profile_description: profile.description.clone(),
            confidence: confidence.to_string(),
            generic,
            eligibility,
            requires_explicit_choice: generic
                || profile_requires_explicit_selection(&profile)
                || explicit_only_model,
            reasons: evidence.reasons,
            plans: profile_plans,
        });
    }
    resolved.sort_by(|a, b| a.profile_id.cmp(&b.profile_id));
    let eligible: Vec<_> = resolved
        .iter()
        .filter(|p| !p.generic && p.eligibility == Eligibility::Eligible)
        .collect();
    let incomplete = resolved
        .iter()
        .any(|p| !p.generic && p.eligibility == Eligibility::Incomplete);
    let automatic = !incomplete && eligible.len() == 1 && !eligible[0].requires_explicit_choice;
    let status = if incomplete {
        "incomplete"
    } else if automatic {
        "resolved"
    } else if !eligible.is_empty() {
        "choice_required"
    } else {
        "unsupported"
    };
    let selected = automatic.then(|| eligible[0].profile_id.clone());
    let candidates: Vec<_> = eligible.iter().flat_map(|p| p.plans.clone()).collect();
    let generic: Vec<_> = resolved
        .iter()
        .filter(|p| p.generic && p.eligibility == Eligibility::Eligible)
        .flat_map(|p| p.plans.clone())
        .collect();
    let recommended = if automatic && eligible[0].plans.len() == 1 {
        eligible[0]
            .plans
            .first()
            .and_then(|p| p.get("planId"))
            .cloned()
    } else {
        None
    };
    let blank: Vec<_> = candidates
        .iter()
        .chain(generic.iter())
        .map(|p| {
            let mut p = p.clone();
            p["selectionMode"] = json!("blank");
            p["requiresExplicitChoice"] = json!(true);
            p
        })
        .collect();
    let blocked = eligible.is_empty() && generic.is_empty();
    Ok(
        json!({ "profiles": resolved, "resolutionStatus": status, "selectedProfileId": selected,
        "confidence": resolved.iter().filter(|p| !p.generic && p.eligibility != Eligibility::Incompatible).max_by_key(|p| match p.confidence.as_str() { "exact" => 3, "high" => 2, _ => 1 }).map(|p| p.confidence.as_str()).unwrap_or("none"),
        "recommendedPlanId": recommended, "requiresExplicitChoice": !automatic && !blocked,
        "candidates": candidates, "safeGenericPlans": generic, "blankSetupPlans": blank,
        "blocked": blocked, "blockReason": blocked.then_some("Device identification is incomplete or unsupported. Read device information again.") }),
    )
}

fn load_yaml_directory<T: for<'de> Deserialize<'de>>(
    root: &Path,
    directory: &str,
) -> Result<Vec<T>, ApiError> {
    let mut paths = fs::read_dir(root.join(directory))
        .map_err(|_| catalog_match_load_error(directory))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_yaml(path))
        .collect::<Vec<PathBuf>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(path).map_err(|_| catalog_match_load_error(directory))?;
            serde_yaml::from_slice(&bytes).map_err(|_| catalog_match_load_error(directory))
        })
        .collect()
}

fn catalog_match_load_error(directory: &str) -> ApiError {
    ApiError::load_failed(
        "Catalog matching data is unavailable.",
        json!({ "reason": "catalog_match_data_invalid", "directory": directory }),
    )
}

fn is_yaml(path: &Path) -> bool {
    path.is_file()
        && matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        )
}

fn profile_is_generic(profile: &ProfileYaml) -> bool {
    profile
        .metadata
        .get("safe_generic")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || profile.id.split('.').any(|part| part == "generic")
}

fn profile_requires_explicit_selection(profile: &ProfileYaml) -> bool {
    profile
        .metadata
        .get("automatic_profile_matching")
        .and_then(Value::as_bool)
        == Some(false)
        || profile
            .metadata
            .get("selection_policy")
            .and_then(Value::as_str)
            == Some("explicit_operator_selection")
}

pub(crate) fn evaluate_profile(
    facts: &DetectedDeviceFacts,
    criteria: &ProfileMatchYaml,
) -> MatchEvidence {
    let mut result = MatchEvidence::default();
    evaluate_contains(
        "manufacturer",
        facts.manufacturer.as_deref(),
        &criteria.manufacturer_contains,
        true,
        &mut result,
    );
    evaluate_contains(
        "brand",
        facts.brand.as_deref(),
        &criteria.brand_contains,
        true,
        &mut result,
    );
    evaluate_patterns(
        facts.model.as_deref(),
        &criteria.model_patterns,
        &mut result,
    );
    if let Some(range) = &criteria.android_version {
        result.declared += 1;
        match facts.android_version {
            None => {
                result.missing += 1;
                result
                    .reasons
                    .push("Android version was unavailable.".to_string());
            }
            Some(version)
                if range.min.is_some_and(|minimum| version < minimum)
                    || range.max.is_some_and(|maximum| version > maximum) =>
            {
                result.conflicts += 1;
                result.android_conflict = true;
                result
                    .reasons
                    .push("Android version is outside the supported range.".to_string());
            }
            Some(_) => {
                result.matched += 1;
                result
                    .reasons
                    .push("Android version is supported.".to_string());
            }
        }
    }
    result
}

fn evaluate_contains(
    label: &str,
    actual: Option<&str>,
    expected: &[String],
    family: bool,
    result: &mut MatchEvidence,
) {
    if expected.is_empty() {
        return;
    }
    result.declared += 1;
    let Some(actual) = actual else {
        result.missing += 1;
        result
            .reasons
            .push(format!("Device {label} was unavailable."));
        return;
    };
    let actual = actual.to_lowercase();
    if expected
        .iter()
        .any(|value| actual.contains(&value.to_lowercase()))
    {
        result.matched += 1;
        if family {
            result.family_matches += 1;
        }
        result.reasons.push(format!("Device {label} matched."));
    } else {
        result.conflicts += 1;
        if family {
            result.family_conflicts += 1;
        }
        result
            .reasons
            .push(format!("Device {label} did not match."));
    }
}

fn evaluate_patterns(actual: Option<&str>, patterns: &[String], result: &mut MatchEvidence) {
    if patterns.is_empty() {
        return;
    }
    result.declared += 1;
    let Some(actual) = actual else {
        result.missing += 1;
        result
            .reasons
            .push("Device model was unavailable.".to_string());
        return;
    };
    if patterns.iter().any(|pattern| {
        regex::Regex::new(pattern)
            .map(|regex| regex.is_match(actual))
            .unwrap_or(false)
    }) {
        result.matched += 1;
        result.reasons.push("Device model matched.".to_string());
    } else {
        result.conflicts += 1;
        result
            .reasons
            .push("Device model did not match.".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolve(
        profiles: &[(&str, &str)],
        plans: &[(&str, &str)],
        facts: DetectedDeviceFacts,
    ) -> Value {
        let root = tempfile::tempdir().unwrap();
        for directory in ["apps", "recipes", "device_profiles", "device_plans"] {
            fs::create_dir(root.path().join(directory)).unwrap();
        }
        for (id, criteria) in profiles {
            fs::write(
                root.path().join(format!("device_profiles/{id}.yaml")),
                format!("id: {id}\nname: {id}\nmatch:\n{criteria}\n"),
            )
            .unwrap();
        }
        for (id, profile) in plans {
            fs::write(
                root.path().join(format!("device_plans/{id}.yaml")),
                format!("id: {id}\nname: {id}\ndevice_profile_ref: {profile}\n"),
            )
            .unwrap();
        }
        match_device(&CatalogSnapshot::legacy_local(root.path()).unwrap(), &facts).unwrap()
    }
    fn facts() -> DetectedDeviceFacts {
        DetectedDeviceFacts {
            manufacturer: Some("ARBOR".into()),
            model: Some("GT78-VN".into()),
            ..Default::default()
        }
    }
    #[test]
    fn multiple_plans_do_not_manufacture_profile_ambiguity_or_arbitrary_recommendation() {
        let result = resolve(
            &[("a", "  manufacturer_contains: [ARBOR]")],
            &[("z", "a"), ("a", "a")],
            facts(),
        );
        assert_eq!(result["selectedProfileId"], "a");
        assert_eq!(result["resolutionStatus"], "resolved");
        assert!(result["recommendedPlanId"].is_null());
        assert_eq!(result["profiles"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn exact_and_brand_profiles_require_choice_in_either_catalog_order() {
        for (exact, brand) in [("a", "z"), ("z", "a")] {
            let result = resolve(
                &[
                    (
                        exact,
                        "  manufacturer_contains: [ARBOR]\n  model_patterns: ['^GT78-VN$']",
                    ),
                    (brand, "  manufacturer_contains: [ARBOR]"),
                ],
                &[("one", exact), ("two", brand)],
                facts(),
            );
            assert_eq!(result["resolutionStatus"], "choice_required");
            assert!(result["selectedProfileId"].is_null());
            assert!(result["recommendedPlanId"].is_null());
        }
    }

    #[test]
    fn checked_in_konkr_and_ayaneo_profiles_require_a_choice_when_both_match() {
        let authored_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let snapshot = CatalogSnapshot::legacy_local(&authored_root).unwrap();
        let facts = DetectedDeviceFacts {
            manufacturer: Some("ARBOR".into()),
            // Both checked-in matchers accept ARBOR, so the shared model stays ambiguous.
            brand: Some("ARBOR".into()),
            model: Some("GT78-VN".into()),
            android_version: Some(12),
            android_api_level: Some(31),
            ..Default::default()
        };

        let result = match_device(&snapshot, &facts).unwrap();

        assert_eq!(result["resolutionStatus"], "choice_required");
        assert!(result["selectedProfileId"].is_null());
        assert_eq!(result["requiresExplicitChoice"], true);
        let mut plan_ids = result["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|plan| plan["planId"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        plan_ids.sort();
        assert_eq!(
            plan_ids,
            vec!["ayaneo.pocket_air_mini.base", "konkr.pocket_advance.base"]
        );
    }

    #[test]
    fn plausible_incomplete_competitor_blocks_a_complete_eligible_profile() {
        let result = resolve(
            &[
                ("complete", "  manufacturer_contains: [ARBOR]"),
                (
                    "incomplete",
                    "  manufacturer_contains: [ARBOR]\n  brand_contains: [ARBOR]",
                ),
            ],
            &[("one", "complete")],
            facts(),
        );
        assert_eq!(result["resolutionStatus"], "incomplete");
        assert_eq!(result["profiles"][1]["eligibility"], "incomplete");
        assert_eq!(result["profiles"][1]["confidence"], "low");
        assert!(result["selectedProfileId"].is_null());
    }
    #[test]
    fn known_conflict_excludes_a_competitor_even_when_other_facts_are_missing() {
        let result = resolve(
            &[
                ("complete", "  manufacturer_contains: [ARBOR]"),
                (
                    "conflict",
                    "  manufacturer_contains: [Other]\n  brand_contains: [Other]",
                ),
            ],
            &[("one", "complete")],
            facts(),
        );
        assert_eq!(result["resolutionStatus"], "resolved");
        assert_eq!(result["profiles"][1]["eligibility"], "incompatible");
    }
    #[test]
    fn generic_needs_every_declared_criterion_and_never_selects_automatically() {
        let result = resolve(
            &[(
                "a.generic",
                "  manufacturer_contains: [ARBOR]\n  brand_contains: [ARBOR]",
            )],
            &[("one", "a.generic")],
            facts(),
        );
        assert_eq!(result["profiles"][0]["eligibility"], "incomplete");
        assert!(result["safeGenericPlans"].as_array().unwrap().is_empty());
        assert!(result["selectedProfileId"].is_null());
    }
}
