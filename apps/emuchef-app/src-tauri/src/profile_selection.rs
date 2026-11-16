//! Native Device Profile resolution and selection authority for one setup session.

use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProfileBinding {
    pub device_handle: String,
    pub session_epoch: u64,
    pub runtime_generation: u64,
    pub platform_tools_revision: u64,
    pub facts: Value,
    pub catalog: Value,
    pub qualification: Option<String>,
}

#[derive(Clone, Debug)]
struct Resolution {
    handle: String,
    binding: ProfileBinding,
    result: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct SelectedProfile {
    pub handle: String,
    pub profile_id: String,
    pub binding: ProfileBinding,
    pub plans: Vec<String>,
    pub setup_generation: u64,
}

/// Retains at most one resolution and one selected profile. Equivalent reads
/// reuse their handles; a material change retires the entire setup authority.
#[derive(Default)]
pub(crate) struct ProfileSelectionState {
    resolution: Option<Resolution>,
    selected: Option<SelectedProfile>,
    generation: u64,
    notice: Option<&'static str>,
}

fn selection_error() -> String {
    crate::commands::safe_error(
        "profile_selection_stale",
        "Choose a Device Profile again. The device or setup authority changed.",
    )
}

impl ProfileSelectionState {
    pub fn invalidate(&mut self) {
        self.resolution = None;
        self.selected = None;
        self.generation = self.generation.saturating_add(1);
    }

    pub fn invalidate_device(&mut self, device: &str, reason: &'static str) {
        if self
            .resolution
            .as_ref()
            .is_some_and(|r| r.binding.device_handle == device)
        {
            self.invalidate_with_reason(reason);
        }
    }

    pub fn invalidate_with_reason(&mut self, reason: &'static str) {
        self.invalidate();
        self.notice = Some(reason);
    }

    pub fn observe(
        &mut self,
        binding: ProfileBinding,
        result: Value,
        change_allowed: bool,
    ) -> Value {
        if self
            .resolution
            .as_ref()
            .is_none_or(|r| r.binding != binding || r.result != result)
        {
            if let Some(previous) = self.resolution.as_ref() {
                self.notice = Some(
                    if previous.binding.device_handle != binding.device_handle
                        || previous.binding.session_epoch != binding.session_epoch
                    {
                        "The device session changed or reconnected. Choose its Device Profile again."
                    } else if previous.binding.catalog != binding.catalog {
                        "The setup catalog changed. Restart EmuChef and choose a Device Profile again."
                    } else if previous.binding.runtime_generation != binding.runtime_generation {
                        "The runtime changed. Read device information and choose a Device Profile again."
                    } else if previous.binding.platform_tools_revision
                        != binding.platform_tools_revision
                    {
                        "Platform-Tools changed. Read device information and choose a Device Profile again."
                    } else if previous.binding.qualification != binding.qualification {
                        "Device capabilities changed. Read device information and choose a Device Profile again."
                    } else {
                        "Device firmware or eligibility changed. Read device information and choose a Device Profile again."
                    },
                );
            }
            self.invalidate();
            self.resolution = Some(Resolution {
                handle: format!("resolution_{}", Uuid::new_v4().simple()),
                binding,
                result,
            });
        }
        if change_allowed && self.selected.is_none() {
            if let Some(id) = self
                .resolution
                .as_ref()
                .and_then(|r| r.result["selectedProfileId"].as_str())
                .map(str::to_string)
            {
                let handle = self.resolution.as_ref().unwrap().handle.clone();
                let notice = self.notice;
                let _ = self.select(&handle, &id, false);
                self.notice = notice;
            }
        }
        self.projection(change_allowed)
    }

    pub fn select(
        &mut self,
        resolution_handle: &str,
        profile_id: &str,
        generic_acknowledged: bool,
    ) -> Result<SelectedProfile, String> {
        let resolution = self
            .resolution
            .as_ref()
            .filter(|r| r.handle == resolution_handle)
            .ok_or_else(selection_error)?;
        let profile = resolution.result["profiles"]
            .as_array()
            .and_then(|profiles| profiles.iter().find(|p| p["profileId"] == profile_id))
            .ok_or_else(selection_error)?;
        if profile["eligibility"] != "eligible"
            || (profile["generic"] == true && !generic_acknowledged)
            || (profile["generic"] != true && resolution.result["resolutionStatus"] == "incomplete")
        {
            return Err(selection_error());
        }
        if let Some(selected) = self
            .selected
            .as_ref()
            .filter(|s| s.profile_id == profile_id)
        {
            return Ok(selected.clone());
        }
        self.generation = self.generation.saturating_add(1);
        let selected = SelectedProfile {
            handle: format!("selection_{}", Uuid::new_v4().simple()),
            profile_id: profile_id.to_string(),
            binding: resolution.binding.clone(),
            plans: profile["plans"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["planId"].as_str().map(str::to_string))
                .collect(),
            setup_generation: self.generation,
        };
        self.selected = Some(selected.clone());
        self.notice = None;
        self.resolution.as_mut().unwrap().handle =
            format!("resolution_{}", Uuid::new_v4().simple());
        Ok(selected)
    }

    pub fn require(
        &self,
        binding: &ProfileBinding,
        handle: Option<&str>,
        plan: Option<&str>,
    ) -> Result<SelectedProfile, String> {
        self.selected
            .as_ref()
            .filter(|s| {
                &s.binding == binding
                    && handle.is_none_or(|h| h == s.handle)
                    && plan.is_none_or(|p| s.plans.iter().any(|allowed| allowed == p))
            })
            .cloned()
            .ok_or_else(selection_error)
    }

    pub fn require_resolution_binding(&self, binding: &ProfileBinding) -> Result<(), String> {
        if self
            .resolution
            .as_ref()
            .is_some_and(|r| &r.binding == binding)
        {
            Ok(())
        } else {
            Err(selection_error())
        }
    }

    pub fn require_match_set(&self, result: &Value) -> Result<(), String> {
        if self
            .resolution
            .as_ref()
            .is_some_and(|r| &r.result == result)
        {
            Ok(())
        } else {
            Err(selection_error())
        }
    }

    pub fn projection(&self, change_allowed: bool) -> Value {
        json!({ "profileSelectionNotice": self.notice, "resolutionHandle": self.resolution.as_ref().map(|r| &r.handle),
            "selectionHandle": self.selected.as_ref().map(|s| &s.handle),
            "selectedProfileId": self.selected.as_ref().map(|s| &s.profile_id),
            "setupGeneration": self.generation, "profileChangeAllowed": change_allowed,
            "profileChangeReason": (!change_allowed).then_some("An execution is active or its outcome is unresolved. Finish reconciliation before choosing a profile.") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> ProfileBinding {
        ProfileBinding {
            device_handle: "device".into(),
            session_epoch: 1,
            runtime_generation: 1,
            platform_tools_revision: 1,
            facts: json!({"firmware_build":"one"}),
            catalog: json!({"digest":"one"}),
            qualification: None,
        }
    }
    fn result() -> Value {
        json!({"profiles":[{"profileId":"a","eligibility":"eligible","generic":false,"plans":[{"planId":"a.plan"}]},
        {"profileId":"b","eligibility":"eligible","generic":false,"plans":[]}],"resolutionStatus":"choice_required"})
    }
    #[test]
    fn explicit_selection_requires_current_resolution_and_plan_family() {
        let mut state = ProfileSelectionState::default();
        let view = state.observe(binding(), result(), true);
        assert!(state.require(&binding(), None, Some("a.plan")).is_err());
        assert!(state.select("forged", "a", false).is_err());
        let selected = state
            .select(view["resolutionHandle"].as_str().unwrap(), "a", false)
            .unwrap();
        assert!(state
            .require(&binding(), Some(&selected.handle), Some("a.plan"))
            .is_ok());
        assert!(state
            .require(&binding(), Some(&selected.handle), Some("b.plan"))
            .is_err());
    }
    #[test]
    fn equivalent_refresh_preserves_selection_but_material_changes_retire_it() {
        let mut state = ProfileSelectionState::default();
        let view = state.observe(binding(), result(), true);
        let selected = state
            .select(view["resolutionHandle"].as_str().unwrap(), "a", false)
            .unwrap();
        assert_eq!(
            state.observe(binding(), result(), true)["selectionHandle"],
            selected.handle
        );
        let mut changed = binding();
        changed.session_epoch += 1;
        assert!(state.observe(changed, result(), true)["selectionHandle"].is_null());
    }
    #[test]
    fn firmware_qualification_and_match_set_changes_irreversibly_retire_selection() {
        for change in ["firmware", "qualification", "match_set"] {
            let mut state = ProfileSelectionState::default();
            let original_binding = binding();
            let original_result = result();
            let view = state.observe(original_binding.clone(), original_result.clone(), true);
            let selected = state
                .select(view["resolutionHandle"].as_str().unwrap(), "a", false)
                .unwrap();
            let resolution = state.projection(true)["resolutionHandle"].clone();
            let mut changed_binding = original_binding.clone();
            let mut changed_result = original_result.clone();
            match change {
                "firmware" => changed_binding.facts["firmware_build"] = json!("two"),
                "qualification" => changed_binding.qualification = Some("qualified".into()),
                "match_set" => changed_result["profiles"][1]["eligibility"] = json!("incompatible"),
                _ => unreachable!(),
            }
            let changed = state.observe(changed_binding.clone(), changed_result, true);
            assert!(changed["selectionHandle"].is_null(), "{change}");
            assert!(changed["setupGeneration"].as_u64().unwrap() > selected.setup_generation);
            assert!(state
                .require(&changed_binding, Some(&selected.handle), None)
                .is_err());
            assert!(state
                .require(&original_binding, Some(&selected.handle), None)
                .is_err());
            assert!(state
                .select(resolution.as_str().unwrap(), "a", false)
                .is_err());

            let reverted = state.observe(original_binding.clone(), original_result, true);
            assert!(reverted["selectionHandle"].is_null(), "{change}");
            assert!(state
                .require(&original_binding, Some(&selected.handle), None)
                .is_err());
            assert!(state
                .select(resolution.as_str().unwrap(), "a", false)
                .is_err());
            let replacement = state
                .select(reverted["resolutionHandle"].as_str().unwrap(), "a", false)
                .unwrap();
            assert_ne!(replacement.handle, selected.handle, "{change}");
            assert!(replacement.setup_generation > selected.setup_generation);
            assert!(state
                .require(&original_binding, Some(&selected.handle), None)
                .is_err());
            assert!(state
                .require(&original_binding, Some(&replacement.handle), Some("a.plan"))
                .is_ok());
        }
    }
    #[test]
    fn reordered_choices_cannot_reuse_a_consumed_resolution() {
        let mut state = ProfileSelectionState::default();
        let view = state.observe(binding(), result(), true);
        let old = view["resolutionHandle"].as_str().unwrap();
        let first = state.select(old, "a", false).unwrap();
        assert!(state.select(old, "b", false).is_err());
        let fresh = state.projection(true)["resolutionHandle"]
            .as_str()
            .unwrap()
            .to_string();
        state.select(&fresh, "b", false).unwrap();
        let fresh = state.projection(true)["resolutionHandle"]
            .as_str()
            .unwrap()
            .to_string();
        let last = state.select(&fresh, "a", false).unwrap();
        assert_ne!(first.handle, last.handle);
        assert!(state
            .require(&binding(), Some(&first.handle), Some("a.plan"))
            .is_err());
    }
    #[test]
    fn incomplete_competitor_blocks_model_selection_but_not_eligible_generic() {
        let mut state = ProfileSelectionState::default();
        let mut result = result();
        result["resolutionStatus"] = json!("incomplete");
        result["profiles"].as_array_mut().unwrap().push(
            json!({"profileId":"generic","generic":true,"eligibility":"eligible","plans":[]}),
        );
        let view = state.observe(binding(), result, true);
        let handle = view["resolutionHandle"].as_str().unwrap();
        assert!(state.select(handle, "a", false).is_err());
        assert!(state.select(handle, "generic", false).is_err());
        assert!(state.select(handle, "generic", true).is_ok());
    }
    #[test]
    fn material_session_and_catalog_changes_have_actionable_explanations() {
        for catalog_change in [false, true] {
            let mut state = ProfileSelectionState::default();
            state.observe(binding(), result(), true);
            let mut changed = binding();
            if catalog_change {
                changed.catalog = json!({"digest":"two"});
            } else {
                changed.session_epoch += 1;
            }
            let view = state.observe(changed, result(), true);
            let notice = view["profileSelectionNotice"].as_str().unwrap();
            assert!(notice.contains(if catalog_change {
                "catalog changed"
            } else {
                "reconnected"
            }));
        }
    }
    #[test]
    fn unrelated_device_invalidation_preserves_current_selection() {
        let mut state = ProfileSelectionState::default();
        let view = state.observe(binding(), result(), true);
        let selected = state
            .select(view["resolutionHandle"].as_str().unwrap(), "a", false)
            .unwrap();
        state.invalidate_device("another-device", "Another device changed.");
        assert!(state
            .require(&binding(), Some(&selected.handle), None)
            .is_ok());
    }
    #[test]
    fn equivalent_fact_ordering_and_incidental_counters_do_not_change_binding() {
        assert_eq!(
            normalized_facts(
                &json!({"brand":" ARBOR ","abis":["arm64-v8a","armeabi-v7a"],"counter":1})
            ),
            normalized_facts(
                &json!({"brand":"arbor","abis":["armeabi-v7a","arm64-v8a"],"counter":2})
            )
        );
    }
    #[test]
    fn automatic_selection_cannot_cross_execution_guard() {
        let mut state = ProfileSelectionState::default();
        let mut result = result();
        result["selectedProfileId"] = json!("a");
        assert!(state.observe(binding(), result.clone(), false)["selectionHandle"].is_null());
        assert!(state.observe(binding(), result, true)["selectionHandle"].is_string());
    }
}

/// Canonical eligibility and identity facts exclude observation counters and
/// sort unordered capability lists. Firmware identity remains case-sensitive.
pub(crate) fn normalized_facts(source: &Value) -> Value {
    let mut facts = serde_json::Map::new();
    for key in [
        "manufacturer",
        "brand",
        "model",
        "product",
        "device",
        "board",
        "hardware",
        "abis",
        "android_version",
        "android_api_level",
        "firmware_build",
        "device_tags",
    ] {
        let value = source.get(key).cloned().unwrap_or(Value::Null);
        let value = match value {
            Value::String(s) if matches!(key, "manufacturer" | "brand") => {
                let s = s.trim().to_lowercase();
                if s.is_empty() {
                    Value::Null
                } else {
                    json!(s)
                }
            }
            Value::Array(mut values) => {
                values.sort_by_key(Value::to_string);
                values.dedup();
                Value::Array(values)
            }
            other => other,
        };
        facts.insert(key.to_string(), value);
    }
    Value::Object(facts)
}
