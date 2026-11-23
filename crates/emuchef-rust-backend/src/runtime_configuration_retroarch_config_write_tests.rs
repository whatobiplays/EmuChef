use super::{
    describe_configuration, freeze_uploaded_retroarch_configuration, frozen_configuration_value,
    plan_configuration, select_retroarch_configuration_source, ConfigurationContextRequest,
    FrozenRetroArchConfiguration,
};
use crate::catalog_source::CatalogSnapshot;
use crate::executor::{
    CopyCommitConfigurationWriteOutcome, DeviceOperationError, ExecutorAdapters, ExecutorDevice,
    ExecutorRunner, FakeDryRunDevice, PermissionOperation,
};
use crate::model::OrderedMap;
use crate::planner::{BindingSource, ExecutionPlan, ResolvedInputBinding};
use crate::user_configuration::UserConfiguration;
use indexmap::IndexMap;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn replace_backup_mode_follows_the_native_persisted_preference() {
    let frozen = FrozenRetroArchConfiguration {
        bytes: b"config".to_vec(),
        sha256: crate::runtime_configuration::sha256_hex(b"config"),
        source_identity: serde_json::json!({ "kind": "generated" }),
        operation: "replace".to_string(),
    };

    let enabled = frozen_configuration_value(&frozen, serde_json::json!({}), true, None);
    let disabled = frozen_configuration_value(&frozen, serde_json::json!({}), false, None);

    assert_eq!(enabled["backupMode"], "verified");
    assert_eq!(disabled["backupMode"], "disabled");
}

#[test]
fn one_time_backup_waiver_is_bound_into_the_replace_artifact() {
    let frozen = FrozenRetroArchConfiguration {
        bytes: b"config".to_vec(),
        sha256: crate::runtime_configuration::sha256_hex(b"config"),
        source_identity: serde_json::json!({ "kind": "generated" }),
        operation: "replace".to_string(),
    };
    let waiver_digest = "b".repeat(64);

    let waived = frozen_configuration_value(
        &frozen,
        serde_json::json!({ "targetId": "config" }),
        true,
        Some(&waiver_digest),
    );

    assert_eq!(waived["backupMode"], "waived");
    assert_eq!(waived["backupWaiverDigest"], waiver_digest);
}

fn binding(
    input_id: &str,
    type_name: &str,
    value: serde_json::Value,
    source: BindingSource,
) -> ResolvedInputBinding {
    binding_for_recipe(
        "app.retroarch.provision",
        input_id,
        type_name,
        value,
        source,
    )
}

fn binding_for_recipe(
    recipe_id: &str,
    input_id: &str,
    type_name: &str,
    value: serde_json::Value,
    source: BindingSource,
) -> ResolvedInputBinding {
    ResolvedInputBinding {
        key: format!("{recipe_id}/{input_id}"),
        recipe_id: recipe_id.to_string(),
        input_id: input_id.to_string(),
        type_name: type_name.to_string(),
        value: Some(value),
        source: Some(source),
    }
}

fn mode(value: &str) -> ResolvedInputBinding {
    binding(
        "retroarch_cfg_source",
        "enum",
        serde_json::json!(value),
        BindingSource::Explicit,
    )
}

fn runtime_availability(
    root_shell: bool,
    app_data_write: bool,
    root_shell_route: Option<crate::planner::RootShellRoute>,
) -> super::RuntimeCapabilityAvailability {
    super::RuntimeCapabilityAvailability {
        root_shell,
        app_data_write,
        root_shell_route,
    }
}

fn plan_request(explicit_bindings: OrderedMap<Value>) -> ConfigurationContextRequest {
    let authored_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
    // Host-planning fixtures model a currently qualified root-shell route.
    ConfigurationContextRequest {
        catalog: CatalogSnapshot::legacy_local(&authored_root).unwrap(),
        configuration_root: None,
        user_configuration: None,
        device_plan: Some("ayaneo.konkr_pocket_fit.base".to_string()),
        selected_recipes: Some(vec!["app.retroarch.provision".to_string()]),
        explicit_bindings,
        device_context: Some(super::DeviceContextOverride {
            android_api_level: Some(33),
            ..Default::default()
        }),
        target_device: None,
        runtime_capability_availability: Some(super::RuntimeCapabilityAvailability {
            root_shell: true,
            app_data_write: true,
            root_shell_route: Some(crate::planner::RootShellRoute::SuCommand),
        }),
    }
}

fn copy_authored_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_authored_tree(&source_path, &destination_path);
        } else {
            std::fs::copy(source_path, destination_path).unwrap();
        }
    }
}

fn config_create_plan_request(
    authored_root: &Path,
    recipe_id: &str,
    explicit_bindings: OrderedMap<Value>,
) -> ConfigurationContextRequest {
    let runtime_capability_availability = super::RuntimeCapabilityAvailability {
        root_shell: true,
        app_data_write: true,
        root_shell_route: Some(crate::planner::RootShellRoute::SuCommand),
    };
    ConfigurationContextRequest {
        catalog: CatalogSnapshot::legacy_local(authored_root).unwrap(),
        configuration_root: None,
        user_configuration: None,
        device_plan: Some("ayaneo.konkr_pocket_fit.base".to_string()),
        selected_recipes: Some(vec![recipe_id.to_string()]),
        explicit_bindings,
        device_context: Some(super::DeviceContextOverride {
            android_api_level: Some(31),
            ..Default::default()
        }),
        target_device: None,
        runtime_capability_availability: Some(runtime_capability_availability),
    }
}

#[test]
fn konkr_product_plan_supports_managed_configuration_only_override() {
    let authored_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.configure/retroarch_cfg_source".to_string(),
        serde_json::json!("managed"),
    );
    let mut request =
        config_create_plan_request(&authored_root, "app.retroarch.configure", bindings);
    request.device_plan = Some("konkr.pocket_advance.base".to_string());
    // Runtime availability and write evidence are test fixtures, not profile defaults.
    request.runtime_capability_availability = Some(super::RuntimeCapabilityAvailability {
        root_shell: true,
        app_data_write: true,
        root_shell_route: Some(crate::planner::RootShellRoute::SuCommand),
    });

    let response = plan_configuration(request).unwrap();
    assert!(
        response.plan.is_some(),
        "test-only scoped evidence should admit the explicit config-create recipe: {:?}",
        response.diagnostics
    );
    let plan = response.plan.unwrap();
    let plan_value = serde_json::to_value(&plan).unwrap();

    assert_eq!(
        plan_value["source"]["device_plan_ref"],
        "konkr.pocket_advance.base"
    );
    assert_eq!(
        plan_value["source"]["device_profile_ref"],
        "konkr.pocket_advance"
    );
    assert_eq!(
        plan_value["source"]["selected_recipe_refs"],
        serde_json::json!(["app.retroarch.configure"])
    );
    assert!(!plan.runtime_capabilities.app_data_write);
    assert_eq!(plan.steps.len(), 1);
    let step = &plan.steps[0];
    assert_eq!(step.id, "app.retroarch.configure/seed_retroarch_cfg");
    let Some(crate::planner::ExecutionParamValue::Literal { value }) =
        step.params.get("copy_policy")
    else {
        panic!("configuration creation should retain its authored copy policy");
    };
    assert_eq!(value, "create_only");
    let Some(crate::planner::ExecutionParamValue::Literal { value }) = step.params.get("source")
    else {
        panic!("configuration creation should freeze its target identity");
    };
    assert_eq!(
        value
            .pointer("/targetIdentity/targetId")
            .and_then(Value::as_str),
        Some("config")
    );
    assert_eq!(
        value
            .pointer("/targetIdentity/target/path")
            .and_then(Value::as_str),
        Some("files/retroarch.cfg")
    );
}

#[test]
fn retired_probe_field_is_rejected_from_runtime_capability_request() {
    let parsed =
        serde_json::from_value::<super::RuntimeCapabilityAvailabilityRequest>(serde_json::json!({
            "rootShell": true,
            "appDataWrite": true,
            "configCreateWriteEvidence": true,
            "rootShellRoute": "suCommand"
        }));

    assert!(
        parsed.is_err(),
        "legacy probe claims are not accepted as runtime capabilities"
    );
}

#[test]
fn konkr_configuration_target_does_not_broaden_app_data_write() {
    let temp = tempfile::tempdir().unwrap();
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
    let authored_root = temp.path().join("authored");
    copy_authored_tree(&source_root, &authored_root);

    let plan_path = authored_root.join("device_plans/ayaneo.konkr_pocket_fit.base.yaml");
    let plan_yaml = std::fs::read_to_string(&plan_path).unwrap();
    std::fs::write(
        &plan_path,
        plan_yaml.replace(
            "device_profile_ref: ayaneo.konkr_pocket_fit",
            "device_profile_ref: konkr.pocket_advance",
        ),
    )
    .unwrap();

    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.configure/retroarch_cfg_source".to_string(),
        serde_json::json!("managed"),
    );
    let request = config_create_plan_request(&authored_root, "app.retroarch.configure", bindings);
    let response = plan_configuration(request).unwrap();
    assert!(
        response.plan.is_some(),
        "qualified route should admit the exact configuration target: {:#?}",
        response.diagnostics
    );
    let plan = response.plan.unwrap();

    assert!(!plan.runtime_capabilities.app_data_write);
    assert!(plan
        .steps
        .iter()
        .any(|step| step.id.ends_with("/seed_retroarch_cfg")));
    let config_step = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with("/seed_retroarch_cfg"))
        .unwrap();
    let source_artifact = match config_step.params.get("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => panic!("configuration source should be frozen in the reviewed plan"),
    };
    assert_eq!(
        source_artifact
            .pointer("/targetIdentity/target/path")
            .and_then(Value::as_str),
        Some("files/retroarch.cfg")
    );
    assert!(crate::executor::runtime_capability_enabled(
        &plan,
        config_step,
        "app_data_write"
    ));
    assert!(!crate::executor::reviewed_root_authorized(&plan));
    let mut wrong_destination = plan.clone();
    let wrong_destination_step = wrong_destination
        .steps
        .iter_mut()
        .find(|step| step.id.ends_with("/seed_retroarch_cfg"))
        .unwrap();
    if let Some(crate::planner::ExecutionParamValue::Literal { value }) =
        wrong_destination_step.params.get_mut("source")
    {
        value["targetIdentity"]["target"]["path"] = serde_json::json!("files/other.cfg");
    }
    let wrong_destination_step = wrong_destination
        .steps
        .iter()
        .find(|step| step.id.ends_with("/seed_retroarch_cfg"))
        .unwrap();
    assert!(!crate::executor::runtime_capability_enabled(
        &wrong_destination,
        wrong_destination_step,
        "app_data_write"
    ));
    assert!(!crate::executor::reviewed_root_authorized(
        &wrong_destination
    ));
    let mut unrelated_step = config_step.clone();
    unrelated_step.id = "app.retroarch.provision/copy_cores".to_string();
    assert!(!crate::executor::runtime_capability_enabled(
        &plan,
        &unrelated_step,
        "app_data_write"
    ));
    let mut generic_copy = config_step.clone();
    generic_copy.id = "feature.copy_roms/import_config".to_string();
    generic_copy.recipe_ref = "feature.copy_roms".to_string();
    generic_copy.app_id = None;
    generic_copy.params.insert(
        "source".to_string(),
        crate::planner::ExecutionParamValue::Literal {
            value: serde_json::to_value(crate::planner::RuntimeValue {
                type_name: "file_path".to_string(),
                value: serde_json::json!("/tmp/retroarch.cfg"),
                location: Some("host".to_string()),
            })
            .unwrap(),
        },
    );
    generic_copy.params.insert(
        "dest".to_string(),
        crate::planner::ExecutionParamValue::Literal {
            value: serde_json::json!(
                "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg"
            ),
        },
    );
    generic_copy.params.insert(
        "copy_policy".to_string(),
        crate::planner::ExecutionParamValue::Literal {
            value: serde_json::json!("merge"),
        },
    );
    let mut plan_with_generic_writer = plan.clone();
    plan_with_generic_writer
        .source
        .selected_recipe_refs
        .push("feature.copy_roms".to_string());
    plan_with_generic_writer.steps.push(generic_copy);
    let generic_writer_error =
        crate::executor::validate_supported_copy_plan(&plan_with_generic_writer)
            .expect_err("scoped configuration evidence must not authorize a generic writer");
    assert!(
        generic_writer_error.contains("protected RetroArch configuration target"),
        "{generic_writer_error}"
    );
    assert!(
        !plan.runtime_capabilities.app_data_write,
        "the plan receives only the configuration-target capability"
    );
    assert!(plan.steps.iter().all(|step| {
        !step
            .constraints
            .capabilities
            .iter()
            .any(|capability| capability == "app_data_write")
            || step.id.ends_with("/seed_retroarch_cfg")
    }));
}

#[test]
fn explicit_none_leaves_unrelated_rooted_provisioning_on_legacy_authority() {
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg_source".to_string(),
        serde_json::json!("none"),
    );
    let mut request = plan_request(bindings);
    request.runtime_capability_availability = Some(runtime_availability(
        true,
        true,
        Some(crate::planner::RootShellRoute::SuCommand),
    ));

    let plan = plan_configuration(request)
        .unwrap()
        .plan
        .expect("explicit none should leave unrelated provisioning available");
    assert!(plan.runtime_capabilities.app_data_write);
    assert!(!plan
        .steps
        .iter()
        .any(|step| step.id.ends_with("/seed_retroarch_cfg")));
    assert!(plan.steps.iter().any(|step| {
        step.constraints
            .capabilities
            .iter()
            .any(|capability| capability == "app_data_write")
    }));
}

#[test]
fn configuration_create_fails_closed_when_authored_app_target_changes() {
    let temp = tempfile::tempdir().unwrap();
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
    let authored_root = temp.path().join("authored");
    copy_authored_tree(&source_root, &authored_root);

    let app_path = authored_root.join("apps/retroarch.yaml");
    let app_yaml = std::fs::read_to_string(&app_path).unwrap();
    let expected_target =
        "  config:\n    kind: file\n    location: external_app_data\n    path: files/retroarch.cfg";
    assert_eq!(app_yaml.matches(expected_target).count(), 1);
    std::fs::write(
        app_path,
        app_yaml.replace(
            expected_target,
            "  config:\n    kind: file\n    location: app_data\n    path: files/retroarch.cfg",
        ),
    )
    .unwrap();

    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg_source".to_string(),
        serde_json::json!("managed"),
    );
    let result = plan_configuration(config_create_plan_request(
        &authored_root,
        "app.retroarch.provision",
        bindings,
    ))
    .unwrap();
    assert!(result.plan.is_none());
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "retroarch_config_create_target_invalid"));
}

#[test]
fn configuration_write_requires_a_qualified_root_route() {
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg_source".to_string(),
        serde_json::json!("managed"),
    );

    let mut request = plan_request(bindings.clone());
    request.runtime_capability_availability = Some(runtime_availability(
        true,
        true,
        Some(crate::planner::RootShellRoute::SuCommand),
    ));
    let admitted = plan_configuration(request).unwrap();
    assert!(
        admitted.plan.is_some(),
        "native authorization is a later execution gate"
    );

    let mut unsupported_route = plan_request(bindings);
    unsupported_route.runtime_capability_availability =
        Some(runtime_availability(true, true, None));
    let blocked = plan_configuration(unsupported_route).unwrap();
    assert!(blocked.plan.is_none());
    assert!(blocked
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.code == "configuration_write_route_unavailable" }));
}

fn saved_configuration_source(
    recipe_id: &str,
    input_id: &str,
    value: Value,
) -> super::UserConfigurationSource {
    let mut bindings = IndexMap::new();
    bindings.insert(format!("{recipe_id}/{input_id}"), value);
    super::UserConfigurationSource::Inline(Box::new(UserConfiguration {
        schema_version: 1,
        kind: "user_configuration".to_string(),
        id: "saved-retroarch-source".to_string(),
        name: "Saved RetroArch setup".to_string(),
        device_plan: "ayaneo.konkr_pocket_fit.base".to_string(),
        selected_recipes: vec![recipe_id.to_string()],
        bindings,
        compatibility: None,
        extensions: std::collections::BTreeMap::new(),
        unsupported_extensions: Vec::new(),
    }))
}

fn generated_create_plan() -> ExecutionPlan {
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg_source".to_string(),
        serde_json::json!("managed"),
    );
    let mut plan = plan_configuration(plan_request(bindings))
        .unwrap()
        .plan
        .unwrap();
    plan.steps
        .retain(|step| step.id.ends_with("seed_retroarch_cfg"));
    assert_eq!(plan.steps.len(), 1);
    plan.steps[0].dependencies.clear();
    plan
}

#[test]
fn configuration_only_recipe_is_available_in_the_backend_description() {
    let description = describe_configuration(plan_request(OrderedMap::new())).unwrap();
    let recipe = description
        .recipe_options
        .iter()
        .find(|recipe| recipe["id"] == "app.retroarch.configure")
        .expect("configuration-only recipe should be offered by the ordinary catalog");

    assert_eq!(recipe["available"], true);
    assert_eq!(recipe["selected"], false);
}

#[test]
#[cfg(unix)]
fn selected_configuration_only_recipe_reaches_reviewed_adapter_report() {
    const CONFIGURATION_RECIPE_ID: &str = "app.retroarch.configure";

    let mut bindings = OrderedMap::new();
    bindings.insert(
        format!("{CONFIGURATION_RECIPE_ID}/retroarch_cfg_source"),
        serde_json::json!("managed"),
    );
    let mut request = plan_request(bindings);
    request.selected_recipes = Some(vec![CONFIGURATION_RECIPE_ID.to_string()]);

    let result = plan_configuration(request).unwrap();
    assert!(
        result.plan.is_some(),
        "selected recipe produced no plan: inputs={:?}, diagnostics={:?}",
        result.resolved_inputs,
        result.diagnostics
    );
    assert!(result.review.is_none());
    assert!(result.copy_commit_preparation_required);
    let mut plan = result
        .plan
        .expect("configuration-only selection should plan");
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].recipe_ref, CONFIGURATION_RECIPE_ID);
    assert!(plan.steps[0].id.ends_with("/seed_retroarch_cfg"));
    assert!(!plan
        .steps
        .iter()
        .any(|step| step.id.ends_with("copy_assets")));
    let artifact = match plan.steps[0].params.get("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => panic!("configuration-only plan must contain its frozen artifact"),
    };
    assert_eq!(
        artifact["sourceIdentity"]["recipeId"],
        CONFIGURATION_RECIPE_ID
    );

    let directory = tempfile::tempdir().unwrap();
    let mut runner = adapter_runner(directory.path(), AdapterCreateResult::Created, false);
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("ordinary review preparation should bind create-only authority");
    bind_operation_fence(&mut runner, &plan, directory.path());
    let artifact = match plan.steps[0].params.get("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => panic!("reviewed configuration must remain a frozen artifact"),
    };
    assert!(!artifact["targetIdentity"].is_null());
    assert!(!artifact["absencePrecondition"].is_null());
    let reviewed_digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
    assert_eq!(reviewed_digest.len(), 64);
    let run = runner.run(&plan);

    assert!(run.success, "{run:?}");
    let report = configuration_report(&run);
    assert_eq!(report["outcome"], "created_verified");
    assert_eq!(report["verified"], true);
    assert_eq!(report["byteLength"], 21);
    assert_eq!(
        report["sha256"],
        super::sha256_hex(b"video_threaded=false\n")
    );
}

#[cfg(unix)]
#[test]
fn reviewed_retroarch_config_identity_binds_android_user_zero() {
    let plan = generated_create_plan();
    let artifact = crate::executor::retroarch_config_write_artifact_for_review(&plan.steps[0])
        .expect("reviewed configuration should retain its target identity");

    assert_eq!(artifact["targetIdentity"]["androidUserId"], 0);
}

#[cfg(unix)]
#[test]
fn create_stops_running_retroarch_for_the_reviewed_package_and_user() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, observations) =
        running_create_adapter_runner(directory.path(), AdapterCreateResult::Created);
    let mut plan = generated_create_plan();
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should bind the create precondition");
    bind_operation_fence(&mut runner, &plan, directory.path());

    let run = runner.run(&plan);

    assert!(run.success, "{run:?}");
    assert_eq!(observations.force_stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(observations.force_stop_user_id.load(Ordering::SeqCst), 0);
    assert!(observations
        .force_stop_package_matches
        .load(Ordering::SeqCst));
    assert_eq!(observations.process_checks.load(Ordering::SeqCst), 3);
    assert_eq!(observations.create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        configuration_report(&run)["retroArchStopStatus"],
        "stopped_by_emuchef"
    );
}

#[cfg(unix)]
#[test]
fn create_requires_manual_closure_when_stopping_retroarch_fails() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, observations) =
        running_create_adapter_runner(directory.path(), AdapterCreateResult::Created);
    observations.force_stop_fails.store(true, Ordering::SeqCst);
    let mut plan = generated_create_plan();
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should bind the create precondition");
    bind_operation_fence(&mut runner, &plan, directory.path());

    let run = runner.run(&plan);

    assert!(!run.success);
    assert_eq!(configuration_report(&run)["outcome"], "app_stop_failed");
    assert_eq!(configuration_report(&run)["mayHaveChanged"], false);
    assert_eq!(observations.force_stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(observations.create_calls.load(Ordering::SeqCst), 0);
    assert!(
        run.steps[0]
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("Close RetroArch manually"),
        "{run:?}"
    );
}

#[cfg(unix)]
#[test]
fn create_requires_manual_closure_when_retroarch_process_state_is_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, observations) =
        running_create_adapter_runner(directory.path(), AdapterCreateResult::Created);
    observations
        .process_query_fails
        .store(true, Ordering::SeqCst);
    let mut plan = generated_create_plan();
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should bind the create precondition");
    bind_operation_fence(&mut runner, &plan, directory.path());

    let run = runner.run(&plan);

    assert!(!run.success);
    assert_eq!(
        configuration_report(&run)["outcome"],
        "app_state_unverified"
    );
    assert_eq!(observations.force_stop_calls.load(Ordering::SeqCst), 0);
    assert_eq!(observations.create_calls.load(Ordering::SeqCst), 0);
    assert!(run.steps[0]
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("Close RetroArch manually"));
}

#[cfg(unix)]
#[test]
fn create_pauses_for_manual_closure_if_retroarch_relaunches_after_stop() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, observations) =
        running_create_adapter_runner(directory.path(), AdapterCreateResult::Created);
    observations
        .process_relaunches_after_stop
        .store(true, Ordering::SeqCst);
    let mut plan = generated_create_plan();
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should bind the create precondition");
    bind_operation_fence(&mut runner, &plan, directory.path());

    let run = runner.run(&plan);

    assert!(!run.success);
    assert_eq!(
        configuration_report(&run)["outcome"],
        "app_relaunched_before_write"
    );
    assert_eq!(observations.force_stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(observations.create_calls.load(Ordering::SeqCst), 0);
    assert!(run.steps[0]
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("Close RetroArch manually"));
}

#[cfg(unix)]
#[test]
fn stopped_retroarch_status_survives_a_later_unverified_write() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, observations) =
        running_create_adapter_runner(directory.path(), AdapterCreateResult::PartialWrite);
    let mut plan = generated_create_plan();
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should bind the create precondition");
    bind_operation_fence(&mut runner, &plan, directory.path());

    let run = runner.run(&plan);

    assert_eq!(
        configuration_report(&run)["retroArchStopStatus"],
        "stopped_by_emuchef"
    );
    assert_eq!(observations.force_stop_calls.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[test]
fn selected_replace_stops_app_and_verifies_backup_before_replacement() {
    const ORIGINAL_CONFIG: &[u8] = b"previous RetroArch settings\n";
    const RECIPE_ID: &str = "app.retroarch.configure";

    let directory = tempfile::tempdir().unwrap();
    let mut bindings = OrderedMap::new();
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_cfg_source"),
        serde_json::json!("managed"),
    );
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_cfg_write_operation"),
        serde_json::json!("replace"),
    );
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_video_threaded"),
        serde_json::json!(false),
    );
    let mut request = plan_request(bindings);
    request.selected_recipes = Some(vec![RECIPE_ID.to_string()]);
    let mut plan = plan_configuration(request)
        .unwrap()
        .plan
        .expect("RetroArch replacement should be planable");
    assert_eq!(plan.steps.len(), 1);

    let (mut runner, observations) =
        replace_adapter_runner(directory.path(), ORIGINAL_CONFIG, true);
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should capture the original file precondition");
    let artifact = match plan.steps[0].params.get("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => panic!("configuration replacement should retain frozen artifact"),
    };
    assert_eq!(
        artifact["originalFilePrecondition"]["sha256"],
        super::sha256_hex(ORIGINAL_CONFIG)
    );
    assert_eq!(
        artifact["originalFilePrecondition"]["byteLength"],
        ORIGINAL_CONFIG.len() as u64
    );
    assert!(artifact["originalFilePrecondition"]
        .get("originalSha256")
        .is_none());
    bind_operation_fence(&mut runner, &plan, directory.path());
    let run = runner.run(&plan);

    assert!(run.success, "{run:?}");
    let report = configuration_report(&run);
    assert_eq!(report["backupStatus"], "verified");
    assert_eq!(report["backupSha256"], super::sha256_hex(ORIGINAL_CONFIG));
    let backup_path = report["backupPath"].as_str().unwrap();
    assert_eq!(std::fs::read(backup_path).unwrap(), ORIGINAL_CONFIG);
    assert_eq!(observations.force_stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(observations.force_stop_user_id.load(Ordering::SeqCst), 0);
    assert!(observations
        .force_stop_package_matches
        .load(Ordering::SeqCst));
    assert_eq!(observations.process_checks.load(Ordering::SeqCst), 3);
    assert_eq!(observations.replace_calls.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[test]
fn waived_replace_skips_backup_and_still_verifies_the_exact_replacement() {
    const ORIGINAL_CONFIG: &[u8] = b"previous RetroArch settings\n";
    const RECIPE_ID: &str = "app.retroarch.configure";
    let waiver_digest = "c".repeat(64);

    let directory = tempfile::tempdir().unwrap();
    let mut bindings = OrderedMap::new();
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_cfg_source"),
        serde_json::json!("managed"),
    );
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_cfg_write_operation"),
        serde_json::json!("replace"),
    );
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_video_threaded"),
        serde_json::json!(false),
    );
    let mut request = plan_request(bindings);
    request.selected_recipes = Some(vec![RECIPE_ID.to_string()]);
    let mut plan = plan_configuration(request)
        .unwrap()
        .plan
        .expect("RetroArch replacement should be planable");

    let (mut runner, observations) =
        replace_adapter_runner(directory.path(), ORIGINAL_CONFIG, false);
    let Some(crate::planner::ExecutionParamValue::Literal { value }) =
        plan.steps[0].params.get_mut("source")
    else {
        panic!("configuration replacement should retain its frozen artifact");
    };
    value["backupMode"] = serde_json::json!("waived");
    value["backupWaiverDigest"] = serde_json::json!(waiver_digest);
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should capture the original file precondition");
    bind_operation_fence(&mut runner, &plan, directory.path());

    let run = runner.run(&plan);

    assert!(run.success, "{run:?}");
    let report = configuration_report(&run);
    assert_eq!(report["outcome"], "replaced_verified");
    assert_eq!(report["backupStatus"], "waived");
    assert_eq!(report["backupWaiverDigest"], waiver_digest);
    assert!(report["backupPath"].is_null());
    assert!(!directory.path().join("configuration-backups").exists());
    assert_eq!(observations.force_stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(observations.force_stop_user_id.load(Ordering::SeqCst), 0);
    assert!(observations
        .force_stop_package_matches
        .load(Ordering::SeqCst));
    assert_eq!(observations.replace_calls.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[test]
fn replace_blocks_when_app_remains_running_after_force_stop() {
    const RECIPE_ID: &str = "app.retroarch.configure";

    let directory = tempfile::tempdir().unwrap();
    let mut bindings = OrderedMap::new();
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_cfg_source"),
        serde_json::json!("managed"),
    );
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_cfg_write_operation"),
        serde_json::json!("replace"),
    );
    bindings.insert(
        format!("{RECIPE_ID}/retroarch_video_threaded"),
        serde_json::json!(false),
    );
    let mut request = plan_request(bindings);
    request.selected_recipes = Some(vec![RECIPE_ID.to_string()]);
    let mut plan = plan_configuration(request)
        .unwrap()
        .plan
        .expect("RetroArch replacement should be planable");
    let (mut runner, observations) = replace_adapter_runner(directory.path(), b"old config", true);
    observations
        .process_stays_running
        .store(true, Ordering::SeqCst);
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should capture the original file precondition");
    bind_operation_fence(&mut runner, &plan, directory.path());

    let run = runner.run(&plan);

    assert!(!run.success);
    let report = configuration_report(&run);
    assert_eq!(report["outcome"], "app_still_running");
    assert_eq!(report["mayHaveChanged"], false);
    assert_eq!(observations.force_stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(observations.replace_calls.load(Ordering::SeqCst), 0);
    assert_eq!(observations.process_checks.load(Ordering::SeqCst), 2);
}

fn bind_operation_fence(
    runner: &mut ExecutorRunner<ConfigurationWriteTestDevice>,
    plan: &ExecutionPlan,
    directory: &Path,
) {
    let record = crate::operation_fence::OperationFenceRecord {
        version: 1,
        operation_id: "replace-test-operation".to_string(),
        runtime_generation: 1,
        attempt_sequence: 1,
        device_handle: "test-device".to_string(),
        plan_digest: crate::plan_digest::execution_plan_digest(plan).unwrap(),
        phase: crate::operation_fence::OperationFencePhase::Dispatching,
        outcome: None,
        host_cleanup_complete: false,
        target: None,
        backup: None,
    };
    let path = directory.join("operation-fence.v1");
    crate::operation_fence::write_initial(&path, &record).unwrap();
    runner.set_operation_fence(Some(crate::operation_fence::OperationFenceBinding {
        path,
        record,
    }));
}

fn fake_runner(directory: &Path) -> (ExecutorRunner, PathBuf) {
    let runtime = directory.join("runtime");
    let cache = directory.join("cache");
    let fake_device = directory.join("fake-device");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(&fake_device).unwrap();
    let runner = ExecutorRunner::new(ExecutorAdapters::with_sandbox_roots(
        runtime,
        cache,
        fake_device.clone(),
        Vec::new(),
    ));
    (runner, fake_device)
}

fn target_in(fake_device: &Path) -> PathBuf {
    fake_device.join("storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg")
}

fn configuration_report(run: &crate::executor::ExecutionRunResult) -> &Value {
    &run.steps[0]
        .outputs
        .get("configuration_create_report")
        .unwrap()
        .value
}

fn sandbox_runner(directory: &Path) -> ExecutorRunner<FakeDryRunDevice> {
    let runtime = directory.join("runtime");
    let cache = directory.join("cache");
    let fake_device = directory.join("fake-device");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(&fake_device).unwrap();
    ExecutorRunner::new(ExecutorAdapters::<FakeDryRunDevice>::with_sandbox_roots(
        runtime,
        cache,
        fake_device,
        Vec::new(),
    ))
}

#[test]
#[cfg(unix)]
fn review_and_execution_sandbox_root_mismatch_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let mut plan = generated_create_plan();
    sandbox_runner(&directory.path().join("review"))
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("review should capture the selected sandbox root");

    let execution_root = directory.path().join("execution");
    let run = sandbox_runner(&execution_root).run(&plan);
    let report = configuration_report(&run);

    assert_eq!(report["outcome"], "review_parent_changed");
    assert_eq!(report["mayHaveChanged"], false);
    assert_eq!(report["verified"], false);
    assert!(
        !target_in(&execution_root.join("fake-device")).exists(),
        "a different execution root must remain untouched"
    );
}

#[derive(Clone, Copy, Debug)]
enum AdapterCreateResult {
    Created,
    Existing,
    HashMismatch,
    PartialWrite,
    LostResponse,
    PreDispatchFailure,
    Replaced,
}

#[derive(Debug, Default)]
struct ReplaceObservations {
    force_stop_calls: AtomicUsize,
    force_stop_user_id: AtomicUsize,
    force_stop_package_matches: AtomicBool,
    process_checks: AtomicUsize,
    process_user_id: AtomicUsize,
    process_package_matches: AtomicBool,
    replace_calls: AtomicUsize,
    create_calls: AtomicUsize,
    force_stop_fails: AtomicBool,
    process_query_fails: AtomicBool,
    process_relaunches_after_stop: AtomicBool,
    process_stays_running: AtomicBool,
}

#[derive(Debug)]
struct ConfigurationWriteTestDevice {
    create_result: AdapterCreateResult,
    expect_backup: bool,
    staged_bytes: Vec<u8>,
    fail_cleanup: bool,
    created: bool,
    stage_creation_response_lost: bool,
    stage_created: bool,
    stage_cleanup_attempted: bool,
    original_config_bytes: Vec<u8>,
    app_data_root: Option<PathBuf>,
    replace_observations: Option<Arc<ReplaceObservations>>,
}

impl ExecutorDevice for ConfigurationWriteTestDevice {
    fn install_apk(
        &mut self,
        _apk_path: &Path,
        _replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn push(
        &mut self,
        source: &Path,
        _dest: &str,
        _sync: bool,
    ) -> Result<(), DeviceOperationError> {
        self.staged_bytes = std::fs::read(source)
            .map_err(|error| DeviceOperationError::other(error.to_string()))?;
        Ok(())
    }

    fn mkdir_p(&mut self, _path: &str) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn remove_file(&mut self, _path: &str) -> Result<(), DeviceOperationError> {
        if self.fail_cleanup {
            Err(DeviceOperationError::other(
                "injected staging file cleanup failure",
            ))
        } else {
            Ok(())
        }
    }

    fn remove_tree(&mut self, _path: &str) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn copy_on_device(
        &mut self,
        _source: &str,
        _dest: &str,
        _recursive: bool,
        _privileged: bool,
    ) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn package_installed(&mut self, _package_name: &str) -> Result<bool, DeviceOperationError> {
        Ok(false)
    }

    fn path_exists(&mut self, _path: &str) -> Result<bool, DeviceOperationError> {
        Ok(self.created)
    }

    fn path_is_dir(&mut self, _path: &str) -> Result<bool, DeviceOperationError> {
        Ok(false)
    }

    fn apply_permission_operation(
        &mut self,
        _operation: PermissionOperation,
    ) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn launch_app(
        &mut self,
        _package_name: &str,
        _activity: Option<&str>,
    ) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn force_stop_app(&mut self, _package_name: &str) -> Result<(), DeviceOperationError> {
        if let Some(observations) = &self.replace_observations {
            observations.force_stop_calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
    fn force_stop_app_for_user(
        &mut self,
        package_name: &str,
        user_id: u32,
    ) -> Result<(), DeviceOperationError> {
        if let Some(observations) = &self.replace_observations {
            observations.force_stop_calls.fetch_add(1, Ordering::SeqCst);
            observations
                .force_stop_user_id
                .store(user_id as usize, Ordering::SeqCst);
            observations
                .force_stop_package_matches
                .store(package_name == "com.retroarch.aarch64", Ordering::SeqCst);
            if observations.force_stop_fails.load(Ordering::SeqCst) {
                return Err(DeviceOperationError::other(
                    "injected RetroArch stop failure",
                ));
            }
        }
        Ok(())
    }
    fn app_process_running(&mut self, _package_name: &str) -> Result<bool, DeviceOperationError> {
        if let Some(observations) = &self.replace_observations {
            let check = observations.process_checks.fetch_add(1, Ordering::SeqCst);
            if observations.process_query_fails.load(Ordering::SeqCst) {
                return Err(DeviceOperationError::other(
                    "injected process query failure",
                ));
            }
            if observations.process_stays_running.load(Ordering::SeqCst) {
                return Ok(true);
            }
            if observations
                .process_relaunches_after_stop
                .load(Ordering::SeqCst)
                && observations.force_stop_calls.load(Ordering::SeqCst) > 0
                && check >= 2
            {
                return Ok(true);
            }
            return Ok(observations.force_stop_calls.load(Ordering::SeqCst) == 0);
        }
        Ok(false)
    }

    fn app_process_running_for_user(
        &mut self,
        package_name: &str,
        user_id: u32,
    ) -> Result<bool, DeviceOperationError> {
        if let Some(observations) = &self.replace_observations {
            observations
                .process_user_id
                .store(user_id as usize, Ordering::SeqCst);
            observations
                .process_package_matches
                .store(package_name == "com.retroarch.aarch64", Ordering::SeqCst);
        }
        self.app_process_running(package_name)
    }

    fn copy_commit_path_binding(&mut self, _path: &str) -> Result<String, DeviceOperationError> {
        Ok("reviewed-parent-identity".to_string())
    }

    fn copy_commit_path_is_symlink(&mut self, _path: &str) -> Result<bool, DeviceOperationError> {
        Ok(false)
    }

    fn copy_commit_create_directory(&mut self, _path: &str) -> Result<(), DeviceOperationError> {
        self.stage_created = true;
        if self.stage_creation_response_lost {
            Err(DeviceOperationError::other(
                "injected lost response after staging directory creation",
            ))
        } else {
            Ok(())
        }
    }

    fn copy_commit_prepare_mutation_helper(
        &mut self,
        _stage_root: &str,
        _helper_path: &str,
    ) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn copy_commit_create_file_exclusive(
        &mut self,
        _selected_root: &str,
        _destination_binding: &str,
        _stage_root: &str,
        _helper_path: &str,
        _relative_path: &str,
        package_name: &str,
        user_id: u32,
        _expected_absence: &str,
        expected_source_sha256: &str,
        expected_source_length: u64,
        operation_id: &str,
        before_helper_dispatch: &mut dyn FnMut() -> Result<(), String>,
        after_remote_quiescent: &mut dyn FnMut(
            crate::executor::CopyCommitRemoteOutcome,
        ) -> Result<(), String>,
    ) -> Result<CopyCommitConfigurationWriteOutcome, DeviceOperationError> {
        if operation_id.is_empty() {
            return Err(DeviceOperationError::other("missing operation identity"));
        }
        match self.app_process_running_for_user(package_name, user_id) {
            Ok(false) => {}
            Ok(true) => return Ok(CopyCommitConfigurationWriteOutcome::AppStillRunning),
            Err(_) => return Ok(CopyCommitConfigurationWriteOutcome::AppStateUnverified),
        }
        if matches!(self.create_result, AdapterCreateResult::PreDispatchFailure) {
            return Err(DeviceOperationError::other(
                "injected helper resolution failure before dispatch",
            ));
        }
        before_helper_dispatch().map_err(DeviceOperationError::other)?;
        if let Some(observations) = &self.replace_observations {
            observations.create_calls.fetch_add(1, Ordering::SeqCst);
        }
        match self.create_result {
            AdapterCreateResult::Created => {
                self.created = true;
                after_remote_quiescent(crate::executor::CopyCommitRemoteOutcome::VerifiedWrite)
                    .map_err(DeviceOperationError::other)?;
                Ok(CopyCommitConfigurationWriteOutcome::CreatedVerified {
                    sha256: expected_source_sha256.to_string(),
                    byte_length: expected_source_length,
                })
            }
            AdapterCreateResult::Existing => {
                after_remote_quiescent(
                    crate::executor::CopyCommitRemoteOutcome::AbortedBeforeWrite,
                )
                .map_err(DeviceOperationError::other)?;
                Ok(CopyCommitConfigurationWriteOutcome::TargetAlreadyExists)
            }
            AdapterCreateResult::HashMismatch => {
                after_remote_quiescent(
                    crate::executor::CopyCommitRemoteOutcome::AbortedBeforeWrite,
                )
                .map_err(DeviceOperationError::other)?;
                Ok(CopyCommitConfigurationWriteOutcome::SourceHashMismatch)
            }
            AdapterCreateResult::PartialWrite => {
                after_remote_quiescent(
                    crate::executor::CopyCommitRemoteOutcome::PartialWriteTerminal,
                )
                .map_err(DeviceOperationError::other)?;
                Ok(CopyCommitConfigurationWriteOutcome::PartialWriteTerminal)
            }
            AdapterCreateResult::LostResponse => Err(DeviceOperationError::other(
                "injected lost response after helper invocation",
            )),
            AdapterCreateResult::Replaced => Err(DeviceOperationError::other(
                "replacement outcome used through create adapter",
            )),
            AdapterCreateResult::PreDispatchFailure => {
                unreachable!("pre-dispatch failure must return before the helper dispatch boundary")
            }
        }
    }
    fn copy_commit_read_retroarch_config(
        &mut self,
        _parent: &str,
        _parent_binding: &str,
    ) -> Result<Vec<u8>, DeviceOperationError> {
        Ok(self.original_config_bytes.clone())
    }
    fn copy_commit_replace_retroarch_config(
        &mut self,
        _selected_root: &str,
        _destination_binding: &str,
        _stage_root: &str,
        _helper_path: &str,
        _relative_path: &str,
        package_name: &str,
        user_id: u32,
        expected_original_sha256: &str,
        expected_original_length: u64,
        expected_source_sha256: &str,
        expected_source_length: u64,
        operation_id: &str,
        before_helper_dispatch: &mut dyn FnMut() -> Result<(), String>,
        after_remote_quiescent: &mut dyn FnMut(
            crate::executor::CopyCommitRemoteOutcome,
        ) -> Result<(), String>,
    ) -> Result<CopyCommitConfigurationWriteOutcome, DeviceOperationError> {
        let observations = self
            .replace_observations
            .as_ref()
            .ok_or_else(|| DeviceOperationError::other("replacement test state is missing"))?;
        observations.replace_calls.fetch_add(1, Ordering::SeqCst);
        if observations.force_stop_calls.load(Ordering::SeqCst) != 1
            || observations.process_checks.load(Ordering::SeqCst) != 2
        {
            return Err(DeviceOperationError::other(
                "RetroArch was not stopped and checked before replacement",
            ));
        }
        if crate::runtime_configuration::sha256_hex(&self.original_config_bytes)
            != expected_original_sha256
            || self.original_config_bytes.len() as u64 != expected_original_length
        {
            return Err(DeviceOperationError::other(
                "replacement precondition did not match the original config",
            ));
        }
        let app_data_root = self
            .app_data_root
            .as_ref()
            .ok_or_else(|| DeviceOperationError::other("replacement app-data root is missing"))?;
        if self.expect_backup {
            let backup_directory = crate::configuration_backups::configuration_backup_directory(
                app_data_root,
                operation_id,
            )
            .map_err(|error| DeviceOperationError::other(error.to_string()))?;
            let backup = std::fs::read(backup_directory.join("original.cfg"))
                .map_err(|error| DeviceOperationError::other(error.to_string()))?;
            if backup != self.original_config_bytes {
                return Err(DeviceOperationError::other(
                    "verified original backup was missing before replacement",
                ));
            }
        }
        if self.app_process_running_for_user(package_name, user_id)? {
            return Ok(CopyCommitConfigurationWriteOutcome::AppStillRunning);
        }
        before_helper_dispatch().map_err(DeviceOperationError::other)?;
        after_remote_quiescent(crate::executor::CopyCommitRemoteOutcome::VerifiedWrite)
            .map_err(DeviceOperationError::other)?;
        Ok(CopyCommitConfigurationWriteOutcome::ReplacedVerified {
            sha256: expected_source_sha256.to_string(),
            byte_length: expected_source_length,
        })
    }

    fn copy_commit_cleanup_mutation_helper(
        &mut self,
        _helper_path: &str,
    ) -> Result<(), DeviceOperationError> {
        if self.fail_cleanup {
            Err(DeviceOperationError::other(
                "injected helper cleanup failure",
            ))
        } else {
            Ok(())
        }
    }

    fn copy_commit_remove_empty_directory(
        &mut self,
        _path: &str,
    ) -> Result<(), DeviceOperationError> {
        self.stage_cleanup_attempted = true;
        if self.fail_cleanup {
            Err(DeviceOperationError::other(
                "injected staging directory cleanup failure",
            ))
        } else {
            Ok(())
        }
    }
}

fn adapter_runner(
    directory: &Path,
    create_result: AdapterCreateResult,
    fail_cleanup: bool,
) -> ExecutorRunner<ConfigurationWriteTestDevice> {
    adapter_runner_with_stage_creation_response(directory, create_result, fail_cleanup, false)
}

fn running_create_adapter_runner(
    directory: &Path,
    create_result: AdapterCreateResult,
) -> (
    ExecutorRunner<ConfigurationWriteTestDevice>,
    Arc<ReplaceObservations>,
) {
    let runtime = directory.join("runtime");
    let cache = directory.join("cache");
    let fake_device = directory.join("fake-device");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(&fake_device).unwrap();
    let observations = Arc::new(ReplaceObservations::default());
    let device = ConfigurationWriteTestDevice {
        create_result,
        expect_backup: true,
        staged_bytes: Vec::new(),
        fail_cleanup: false,
        created: false,
        stage_creation_response_lost: false,
        stage_created: false,
        stage_cleanup_attempted: false,
        original_config_bytes: Vec::new(),
        app_data_root: Some(directory.to_path_buf()),
        replace_observations: Some(Arc::clone(&observations)),
    };
    let runner = ExecutorRunner::new(ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        runtime,
        cache,
        fake_device,
        Vec::new(),
        false,
    ));
    (runner, observations)
}

fn adapter_runner_with_stage_creation_response(
    directory: &Path,
    create_result: AdapterCreateResult,
    fail_cleanup: bool,
    stage_creation_response_lost: bool,
) -> ExecutorRunner<ConfigurationWriteTestDevice> {
    let runtime = directory.join("runtime");
    let cache = directory.join("cache");
    let fake_device = directory.join("fake-device");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(&fake_device).unwrap();
    let device = ConfigurationWriteTestDevice {
        create_result,
        expect_backup: true,
        staged_bytes: Vec::new(),
        fail_cleanup,
        created: false,
        stage_creation_response_lost,
        stage_created: false,
        stage_cleanup_attempted: false,
        original_config_bytes: Vec::new(),
        app_data_root: None,
        replace_observations: None,
    };
    ExecutorRunner::new(ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        runtime,
        cache,
        fake_device,
        Vec::new(),
        false,
    ))
}

fn replace_adapter_runner(
    directory: &Path,
    original_config_bytes: &[u8],
    expect_backup: bool,
) -> (
    ExecutorRunner<ConfigurationWriteTestDevice>,
    Arc<ReplaceObservations>,
) {
    let runtime = directory.join("runtime");
    let cache = directory.join("artifact-cache");
    let fake_device = directory.join("fake-device");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(&fake_device).unwrap();
    let observations = Arc::new(ReplaceObservations::default());
    let device = ConfigurationWriteTestDevice {
        create_result: AdapterCreateResult::Replaced,
        expect_backup,
        staged_bytes: Vec::new(),
        fail_cleanup: false,
        created: true,
        stage_creation_response_lost: false,
        stage_created: false,
        stage_cleanup_attempted: false,
        original_config_bytes: original_config_bytes.to_vec(),
        app_data_root: Some(directory.to_path_buf()),
        replace_observations: Some(Arc::clone(&observations)),
    };
    let runner = ExecutorRunner::new(ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        runtime,
        cache,
        fake_device,
        Vec::new(),
        false,
    ));
    (runner, observations)
}

fn default_mode(value: &str) -> ResolvedInputBinding {
    binding(
        "retroarch_cfg_source",
        "enum",
        serde_json::json!(value),
        BindingSource::RecipeDefault,
    )
}

fn default_mode_for_recipe(recipe_id: &str, value: &str) -> ResolvedInputBinding {
    binding_for_recipe(
        recipe_id,
        "retroarch_cfg_source",
        "enum",
        serde_json::json!(value),
        BindingSource::RecipeDefault,
    )
}

fn write_operation(value: &str, source: BindingSource) -> ResolvedInputBinding {
    binding(
        "retroarch_cfg_write_operation",
        "enum",
        serde_json::json!(value),
        source,
    )
}

#[test]
fn no_selected_source_preserves_optional_configuration_behavior() {
    let default_setting = binding(
        "retroarch_video_threaded",
        "boolean",
        serde_json::json!(false),
        BindingSource::RecipeDefault,
    );

    assert_eq!(
        select_retroarch_configuration_source(&[default_setting]).unwrap(),
        None
    );
}

#[test]
fn default_none_mode_preserves_optional_configuration_behavior() {
    assert_eq!(
        select_retroarch_configuration_source(&[default_mode("none")]).unwrap(),
        None
    );
}

#[test]
fn selected_write_operation_is_bound_to_the_managed_or_uploaded_source() {
    let managed = select_retroarch_configuration_source(&[
        default_mode("managed"),
        binding(
            "retroarch_video_threaded",
            "boolean",
            serde_json::json!(true),
            BindingSource::Explicit,
        ),
        write_operation("replace", BindingSource::Explicit),
    ])
    .unwrap()
    .unwrap();
    assert!(matches!(
        managed,
        super::RetroArchConfigurationSource::Generated { operation, .. }
            if operation == "replace"
    ));

    let uploaded = select_retroarch_configuration_source(&[
        default_mode("uploaded"),
        binding(
            "retroarch_cfg",
            "file",
            serde_json::json!("/tmp/retroarch.cfg"),
            BindingSource::Explicit,
        ),
        write_operation("create", BindingSource::RecipeDefault),
    ])
    .unwrap()
    .unwrap();
    assert!(matches!(
        uploaded,
        super::RetroArchConfigurationSource::Uploaded { operation, .. }
            if operation == "create"
    ));

    assert_eq!(
        select_retroarch_configuration_source(&[
            default_mode("none"),
            write_operation("replace", BindingSource::Explicit),
        ])
        .unwrap(),
        None
    );
}

#[test]
fn default_none_projects_legacy_saved_sources_for_both_retroarch_recipes() {
    for recipe_id in ["app.retroarch.provision", "app.retroarch.configure"] {
        let uploaded = [
            default_mode_for_recipe(recipe_id, "none"),
            binding_for_recipe(
                recipe_id,
                "retroarch_cfg",
                "file",
                serde_json::json!("/tmp/saved-retroarch.cfg"),
                BindingSource::UserConfiguration,
            ),
        ];
        assert_eq!(
            super::effective_retroarch_configuration_source(&uploaded, recipe_id),
            "uploaded",
            "legacy file-only bindings must display as uploaded for {recipe_id}"
        );
        assert!(matches!(
            select_retroarch_configuration_source(&uploaded).unwrap(),
            Some(super::RetroArchConfigurationSource::Uploaded { .. })
        ));

        let managed = [
            default_mode_for_recipe(recipe_id, "none"),
            binding_for_recipe(
                recipe_id,
                "retroarch_video_threaded",
                "boolean",
                serde_json::json!(false),
                BindingSource::UserConfiguration,
            ),
        ];
        assert_eq!(
            super::effective_retroarch_configuration_source(&managed, recipe_id),
            "managed",
            "saved managed settings must display as managed for {recipe_id}"
        );
        assert!(matches!(
            select_retroarch_configuration_source(&managed).unwrap(),
            Some(super::RetroArchConfigurationSource::Generated { .. })
        ));
    }
}

#[test]
fn configuration_description_projects_saved_sources_for_both_retroarch_recipes() {
    let directory = tempfile::tempdir().unwrap();
    for recipe_id in ["app.retroarch.provision", "app.retroarch.configure"] {
        let config_path = directory.path().join(format!("{recipe_id}.cfg"));
        std::fs::write(&config_path, b"custom_setting=true\n").unwrap();
        for (input_id, value, expected_source) in [
            (
                "retroarch_cfg",
                serde_json::json!(config_path.to_string_lossy()),
                "uploaded",
            ),
            (
                "retroarch_video_threaded",
                serde_json::json!(false),
                "managed",
            ),
        ] {
            let mut request = plan_request(OrderedMap::new());
            request.selected_recipes = Some(vec![recipe_id.to_string()]);
            request.user_configuration =
                Some(saved_configuration_source(recipe_id, input_id, value));
            let description = describe_configuration(request).unwrap();
            let source = description
                .inputs
                .iter()
                .find(|input| {
                    input["recipeId"] == recipe_id && input["inputId"] == "retroarch_cfg_source"
                })
                .expect("RetroArch source selector should be described");

            assert_eq!(source["value"], "none", "recipe {recipe_id}");
            assert_eq!(
                source["valueSource"], "recipe_default",
                "recipe {recipe_id}"
            );
            assert_eq!(
                source["effectiveConfigurationSource"], expected_source,
                "recipe {recipe_id} should project its saved {expected_source} source"
            );
            let saved_input = description
                .inputs
                .iter()
                .find(|input| input["recipeId"] == recipe_id && input["inputId"] == input_id)
                .unwrap();
            assert_eq!(saved_input["valueSource"], "user_configuration");
        }
    }
}

#[test]
fn explicit_none_with_a_saved_upload_does_not_produce_a_plan() {
    const CONFIGURATION_RECIPE_ID: &str = "app.retroarch.configure";
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("saved-retroarch.cfg");
    std::fs::write(&config_path, b"existing user config\n").unwrap();

    let mut bindings = OrderedMap::new();
    bindings.insert(
        format!("{CONFIGURATION_RECIPE_ID}/retroarch_cfg_source"),
        serde_json::json!("none"),
    );
    bindings.insert(
        format!("{CONFIGURATION_RECIPE_ID}/retroarch_cfg"),
        serde_json::json!(config_path),
    );
    let mut request = plan_request(bindings);
    request.selected_recipes = Some(vec![CONFIGURATION_RECIPE_ID.to_string()]);

    let result = plan_configuration(request).unwrap();
    assert!(result.plan.is_none());
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.code == "configuration_source_conflict" }));
}

#[test]
fn an_explicit_setting_selects_generated_mode_without_a_mode_binding() {
    let selected = select_retroarch_configuration_source(&[
        default_mode("none"),
        binding(
            "retroarch_video_threaded",
            "boolean",
            serde_json::json!(false),
            BindingSource::Explicit,
        ),
    ])
    .unwrap();

    assert!(matches!(
        selected,
        Some(super::RetroArchConfigurationSource::Generated {
            video_threaded: false,
            ..
        })
    ));
}

#[test]
fn explicit_none_rejects_stale_source_bindings() {
    let inputs = [
        mode("none"),
        binding(
            "retroarch_cfg",
            "file",
            serde_json::json!("/tmp/old.cfg"),
            BindingSource::UserConfiguration,
        ),
    ];

    assert_eq!(
        select_retroarch_configuration_source(&inputs).unwrap_err(),
        "configuration_source_conflict"
    );
    assert_eq!(
        super::effective_retroarch_configuration_source(&inputs, "app.retroarch.provision"),
        "conflict",
        "explicit none with a stale saved source must not display as a publishable mode"
    );
}

#[test]
fn configuration_only_recipe_rejects_stale_and_conflicting_sources() {
    const CONFIGURATION_RECIPE_ID: &str = "app.retroarch.configure";

    let conflicting = select_retroarch_configuration_source(&[
        binding_for_recipe(
            CONFIGURATION_RECIPE_ID,
            "retroarch_cfg_source",
            "enum",
            serde_json::json!("uploaded"),
            BindingSource::Explicit,
        ),
        binding_for_recipe(
            CONFIGURATION_RECIPE_ID,
            "retroarch_video_threaded",
            "boolean",
            serde_json::json!(true),
            BindingSource::UserConfiguration,
        ),
        binding_for_recipe(
            CONFIGURATION_RECIPE_ID,
            "retroarch_cfg",
            "file",
            serde_json::json!("/tmp/saved-retroarch.cfg"),
            BindingSource::UserConfiguration,
        ),
    ]);
    assert_eq!(conflicting.unwrap_err(), "configuration_source_conflict");

    let stale_none = select_retroarch_configuration_source(&[
        binding_for_recipe(
            CONFIGURATION_RECIPE_ID,
            "retroarch_cfg_source",
            "enum",
            serde_json::json!("none"),
            BindingSource::Explicit,
        ),
        binding_for_recipe(
            CONFIGURATION_RECIPE_ID,
            "retroarch_cfg",
            "file",
            serde_json::json!("/tmp/saved-retroarch.cfg"),
            BindingSource::UserConfiguration,
        ),
    ]);
    assert_eq!(stale_none.unwrap_err(), "configuration_source_conflict");

    let duplicate_sources = select_retroarch_configuration_source(&[
        mode("managed"),
        binding_for_recipe(
            CONFIGURATION_RECIPE_ID,
            "retroarch_cfg_source",
            "enum",
            serde_json::json!("managed"),
            BindingSource::Explicit,
        ),
    ]);
    assert_eq!(
        duplicate_sources.unwrap_err(),
        "configuration_input_ambiguous"
    );
}

#[test]
fn upload_capture_keeps_exact_bytes_after_source_changes_and_hides_path() {
    let mut source = tempfile::NamedTempFile::new().unwrap();
    let original = b"video_threaded = false\ncustom = \"retained exactly\"\n";
    source.write_all(original).unwrap();
    source.as_file().sync_all().unwrap();
    let path = source.path().to_string_lossy().into_owned();

    let frozen = freeze_uploaded_retroarch_configuration(
        "app.retroarch.provision",
        &path,
        BindingSource::Explicit,
        "create".to_string(),
    )
    .unwrap();
    source.as_file().set_len(0).unwrap();
    source.write_all(b"changed after review\n").unwrap();

    assert_eq!(frozen.bytes, original);
    assert_eq!(frozen.sha256, super::sha256_hex(original));
    assert_eq!(frozen.source_identity["kind"], "uploaded");
    assert_eq!(frozen.source_identity["byteLength"], original.len());
    assert!(!frozen.source_identity.to_string().contains(&path));
}

#[test]
fn generated_plan_binds_frozen_bytes_and_conditionally_orders_final_launch() {
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg_source".to_string(),
        serde_json::json!("managed"),
    );

    let result = plan_configuration(plan_request(bindings)).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != "error"),
        "{:?}",
        result.diagnostics
    );
    assert!(result.copy_commit_preparation_required);
    let plan = result.plan.unwrap();
    let seed = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with("seed_retroarch_cfg"))
        .unwrap();
    let crate::planner::ExecutionParamValue::Literal { value: artifact } =
        seed.params.get("source").unwrap()
    else {
        panic!("configuration source must be frozen in the plan");
    };
    let bytes = artifact["bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|byte| byte.as_u64().unwrap() as u8)
        .collect::<Vec<_>>();
    assert_eq!(bytes, b"video_threaded=false\n");
    assert_eq!(artifact["sha256"], super::sha256_hex(&bytes));
    assert_eq!(artifact["sourceIdentity"]["kind"], "generated");
    assert_eq!(artifact["targetIdentity"]["targetId"], "config");
    assert!(artifact["absencePrecondition"].is_null());
    assert!(plan.resolved_inputs.iter().any(|input| {
        input.key == "app.retroarch.provision/retroarch_cfg" && input.value.is_none()
    }));
    let launch = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with("launch_retroarch"))
        .unwrap();
    assert!(launch.dependencies.contains(&seed.id));
}

#[test]
fn no_configuration_keeps_the_existing_optional_launch_graph() {
    let result = plan_configuration(plan_request(OrderedMap::new())).unwrap();
    let plan = result.plan.unwrap();
    assert!(!plan
        .steps
        .iter()
        .any(|step| step.id.ends_with("seed_retroarch_cfg")));
    let launch = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with("launch_retroarch"))
        .unwrap();
    assert!(!launch
        .dependencies
        .iter()
        .any(|dependency| dependency.ends_with("seed_retroarch_cfg")));
}

#[test]
fn uploaded_plan_freezes_supplied_bytes_without_rewriting_them() {
    let mut source = tempfile::NamedTempFile::new().unwrap();
    let supplied = b"video_threaded = true\ncomment = \"as supplied\"\n";
    source.write_all(supplied).unwrap();
    source.as_file().sync_all().unwrap();
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg_source".to_string(),
        serde_json::json!("uploaded"),
    );
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        serde_json::json!(source.path().to_string_lossy()),
    );

    let result = plan_configuration(plan_request(bindings)).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != "error"),
        "{:?}",
        result.diagnostics
    );
    let plan = result.plan.unwrap();
    let seed = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with("seed_retroarch_cfg"))
        .unwrap();
    let crate::planner::ExecutionParamValue::Literal { value: artifact } =
        seed.params.get("source").unwrap()
    else {
        panic!("configuration source must be frozen in the plan");
    };
    let bytes = artifact["bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|byte| byte.as_u64().unwrap() as u8)
        .collect::<Vec<_>>();
    assert_eq!(bytes, supplied);
    assert_eq!(artifact["sha256"], super::sha256_hex(supplied));
    assert_eq!(artifact["sourceIdentity"]["kind"], "uploaded");
    assert!(plan.inputs.iter().all(|input| {
        input.id != "app.retroarch.provision/retroarch_cfg" || input.value.value.is_null()
    }));
}

#[test]
fn generated_source_uses_the_typed_app_setting() {
    let selected = select_retroarch_configuration_source(&[
        mode("managed"),
        binding(
            "retroarch_video_threaded",
            "boolean",
            serde_json::json!(false),
            BindingSource::RecipeDefault,
        ),
    ])
    .unwrap();

    assert_eq!(
        selected,
        Some(super::RetroArchConfigurationSource::Generated {
            recipe_id: "app.retroarch.provision".to_string(),
            video_threaded: false,
            binding_source: BindingSource::RecipeDefault,
            operation: "create".to_string(),
        })
    );
}

#[test]
fn uploaded_source_preserves_legacy_file_input_without_a_mode_binding() {
    let selected = select_retroarch_configuration_source(&[binding(
        "retroarch_cfg",
        "file",
        serde_json::json!("/tmp/retroarch.cfg"),
        BindingSource::Explicit,
    )])
    .unwrap();

    assert_eq!(
        selected,
        Some(super::RetroArchConfigurationSource::Uploaded {
            recipe_id: "app.retroarch.provision".to_string(),
            path: "/tmp/retroarch.cfg".to_string(),
            binding_source: BindingSource::Explicit,
            operation: "create".to_string(),
        })
    );
}

#[test]
fn explicit_uploaded_mode_ignores_only_the_recipe_default_setting() {
    let selected = select_retroarch_configuration_source(&[
        mode("uploaded"),
        binding(
            "retroarch_video_threaded",
            "boolean",
            serde_json::json!(false),
            BindingSource::RecipeDefault,
        ),
        binding(
            "retroarch_cfg",
            "file",
            serde_json::json!("/tmp/retroarch.cfg"),
            BindingSource::Explicit,
        ),
    ])
    .unwrap();

    assert!(matches!(
        selected,
        Some(super::RetroArchConfigurationSource::Uploaded { .. })
    ));
}

#[test]
fn contradictory_mode_and_inactive_source_bindings_are_rejected() {
    let stale_upload = select_retroarch_configuration_source(&[
        mode("managed"),
        binding(
            "retroarch_video_threaded",
            "boolean",
            serde_json::json!(true),
            BindingSource::Explicit,
        ),
        binding(
            "retroarch_cfg",
            "file",
            serde_json::json!("/tmp/old.cfg"),
            BindingSource::UserConfiguration,
        ),
    ]);
    assert_eq!(stale_upload.unwrap_err(), "configuration_source_conflict");

    let stale_setting = select_retroarch_configuration_source(&[
        mode("uploaded"),
        binding(
            "retroarch_video_threaded",
            "boolean",
            serde_json::json!(false),
            BindingSource::UserConfiguration,
        ),
        binding(
            "retroarch_cfg",
            "file",
            serde_json::json!("/tmp/new.cfg"),
            BindingSource::Explicit,
        ),
    ]);
    assert_eq!(stale_setting.unwrap_err(), "configuration_source_conflict");
}

#[test]
fn selected_source_requires_its_matching_input_and_known_mode() {
    assert_eq!(
        select_retroarch_configuration_source(&[mode("uploaded")]).unwrap_err(),
        "configuration_upload_missing"
    );
    assert_eq!(
        select_retroarch_configuration_source(&[mode("managed")]).unwrap_err(),
        "configuration_managed_value_missing"
    );
    assert_eq!(
        select_retroarch_configuration_source(&[mode("another")]).unwrap_err(),
        "configuration_source_invalid"
    );
}

#[test]
#[cfg(unix)]
fn generated_configuration_runs_from_the_precondition_bound_review() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, fake_device) = fake_runner(directory.path());
    let mut plan = generated_create_plan();
    let before_precondition = crate::plan_digest::execution_plan_digest(&plan).unwrap();

    runner.prepare_copy_commit_snapshots(&mut plan).unwrap();

    let reviewed_digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
    assert_ne!(before_precondition, reviewed_digest);
    let artifact = match plan.steps[0].params.get("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => panic!("configuration artifact must be literal frozen plan data"),
    };
    assert_eq!(
        artifact["absencePrecondition"]["status"],
        "confirmed_absent"
    );
    assert_eq!(
        artifact["absencePrecondition"]["authority"],
        "dry_run_sandbox"
    );

    let run = runner.run(&plan);

    assert!(run.success);
    assert_eq!(
        std::fs::read(target_in(&fake_device)).unwrap(),
        b"video_threaded=false\n"
    );
    let report = configuration_report(&run);
    assert_eq!(report["outcome"], "simulated_create_verified");
    assert_eq!(
        report["sha256"],
        super::sha256_hex(b"video_threaded=false\n")
    );
    assert_eq!(report["byteLength"], 21);
    assert_eq!(report["simulated"], true);
    assert_eq!(report["appliedToDevice"], false);
}

#[test]
#[cfg(unix)]
fn device_reviewed_generated_configuration_runs_in_the_simulation_sandbox() {
    let directory = tempfile::tempdir().unwrap();
    let mut plan = generated_create_plan();
    let mut review_runner = adapter_runner(directory.path(), AdapterCreateResult::Created, false);
    review_runner
        .prepare_copy_commit_snapshots(&mut plan)
        .unwrap();
    let artifact = match plan.steps[0].params.get("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => panic!("configuration artifact must be literal frozen plan data"),
    };
    assert_eq!(artifact["absencePrecondition"]["authority"], "device");

    let simulation_directory = tempfile::tempdir().unwrap();
    let (mut simulation_runner, fake_device) = fake_runner(simulation_directory.path());
    let run = simulation_runner.run(&plan);

    assert!(run.success, "{run:?}");
    assert_eq!(
        std::fs::read(target_in(&fake_device)).unwrap(),
        b"video_threaded=false\n"
    );
    let report = configuration_report(&run);
    assert_eq!(report["outcome"], "simulated_create_verified");
    assert_eq!(report["simulated"], true);
    assert_eq!(report["appliedToDevice"], false);
}

#[test]
#[cfg(unix)]
fn uploaded_configuration_execution_uses_the_frozen_supplied_bytes() {
    let mut source = tempfile::NamedTempFile::new().unwrap();
    let supplied = b"video_threaded = true\ncustom = \"unchanged\"\n";
    source.write_all(supplied).unwrap();
    source.as_file().sync_all().unwrap();
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg_source".to_string(),
        serde_json::json!("uploaded"),
    );
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        serde_json::json!(source.path().to_string_lossy()),
    );
    let mut plan = plan_configuration(plan_request(bindings))
        .unwrap()
        .plan
        .unwrap();
    plan.steps
        .retain(|step| step.id.ends_with("seed_retroarch_cfg"));
    plan.steps[0].dependencies.clear();
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, fake_device) = fake_runner(directory.path());
    runner.prepare_copy_commit_snapshots(&mut plan).unwrap();

    source.as_file().set_len(0).unwrap();
    source.write_all(b"source changed after review\n").unwrap();
    let run = runner.run(&plan);

    assert!(run.success);
    assert_eq!(std::fs::read(target_in(&fake_device)).unwrap(), supplied);
    let report = configuration_report(&run);
    assert_eq!(report["sourceKind"], "uploaded");
    assert_eq!(report["sha256"], super::sha256_hex(supplied));
    assert_eq!(report["byteLength"], supplied.len());
}

#[test]
#[cfg(unix)]
fn device_reviewed_uploaded_configuration_runs_in_the_simulation_sandbox() {
    let mut source = tempfile::NamedTempFile::new().unwrap();
    let supplied = b"video_threaded = true\ncustom = \"simulated exactly\"\n";
    source.write_all(supplied).unwrap();
    source.as_file().sync_all().unwrap();
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg_source".to_string(),
        serde_json::json!("uploaded"),
    );
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        serde_json::json!(source.path().to_string_lossy()),
    );
    let mut plan = plan_configuration(plan_request(bindings))
        .unwrap()
        .plan
        .unwrap();
    plan.steps
        .retain(|step| step.id.ends_with("seed_retroarch_cfg"));
    plan.steps[0].dependencies.clear();

    let review_directory = tempfile::tempdir().unwrap();
    let mut review_runner =
        adapter_runner(review_directory.path(), AdapterCreateResult::Created, false);
    review_runner
        .prepare_copy_commit_snapshots(&mut plan)
        .unwrap();
    let artifact = match plan.steps[0].params.get("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => panic!("configuration artifact must be literal frozen plan data"),
    };
    assert_eq!(artifact["absencePrecondition"]["authority"], "device");

    source.as_file().set_len(0).unwrap();
    source.write_all(b"source changed after review\n").unwrap();
    let simulation_directory = tempfile::tempdir().unwrap();
    let (mut simulation_runner, fake_device) = fake_runner(simulation_directory.path());
    let run = simulation_runner.run(&plan);

    assert!(run.success, "{run:?}");
    assert_eq!(std::fs::read(target_in(&fake_device)).unwrap(), supplied);
    let report = configuration_report(&run);
    assert_eq!(report["outcome"], "simulated_create_verified");
    assert_eq!(report["sourceKind"], "uploaded");
    assert_eq!(report["sha256"], super::sha256_hex(supplied));
    assert_eq!(report["simulated"], true);
    assert_eq!(report["appliedToDevice"], false);
}

#[test]
#[cfg(unix)]
fn initially_present_target_blocks_review_without_changing_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, fake_device) = fake_runner(directory.path());
    let target = target_in(&fake_device);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let existing = b"existing RetroArch configuration";
    std::fs::write(&target, existing).unwrap();
    let mut plan = generated_create_plan();

    let result = runner.prepare_copy_commit_snapshots(&mut plan);

    assert!(result.unwrap_err().contains("already exists"));
    assert_eq!(std::fs::read(target).unwrap(), existing);
}

#[test]
#[cfg(unix)]
fn target_appearing_after_review_is_rejected_and_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, fake_device) = fake_runner(directory.path());
    let mut plan = generated_create_plan();
    runner.prepare_copy_commit_snapshots(&mut plan).unwrap();
    let target = target_in(&fake_device);
    let appeared = b"created after immutable review";
    std::fs::write(&target, appeared).unwrap();

    let run = runner.run(&plan);

    assert!(!run.success);
    assert_eq!(
        configuration_report(&run)["outcome"],
        "blocked_existing_target"
    );
    assert_eq!(std::fs::read(target).unwrap(), appeared);
}

#[cfg(unix)]
#[test]
fn parent_replaced_with_symlink_after_review_is_rejected() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let (mut runner, fake_device) = fake_runner(directory.path());
    let mut plan = generated_create_plan();
    runner.prepare_copy_commit_snapshots(&mut plan).unwrap();
    let target = target_in(&fake_device);
    let parent = target.parent().unwrap();
    let replacement = directory.path().join("replacement");
    std::fs::create_dir(&replacement).unwrap();
    std::fs::remove_dir(parent).unwrap();
    symlink(&replacement, parent).unwrap();

    let run = runner.run(&plan);

    assert!(!run.success);
    assert_eq!(
        configuration_report(&run)["outcome"],
        "review_parent_changed"
    );
    assert!(!replacement.join("retroarch.cfg").exists());
}

#[cfg(unix)]
#[test]
fn device_review_simulation_rejects_symlinked_sandbox_root_without_creating_outside() {
    use std::os::unix::fs::symlink;

    let review_directory = tempfile::tempdir().unwrap();
    let mut plan = generated_create_plan();
    let mut review_runner =
        adapter_runner(review_directory.path(), AdapterCreateResult::Created, false);
    review_runner
        .prepare_copy_commit_snapshots(&mut plan)
        .unwrap();

    let simulation_directory = tempfile::tempdir().unwrap();
    let (mut simulation_runner, fake_device) = fake_runner(simulation_directory.path());
    let outside = simulation_directory.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::remove_dir(&fake_device).unwrap();
    symlink(&outside, &fake_device).unwrap();

    let run = simulation_runner.run(&plan);

    assert!(!run.success);
    assert!(!outside.join("storage").exists());
    assert_eq!(configuration_report(&run)["mayHaveChanged"], false);
}

#[cfg(unix)]
#[test]
fn fake_review_rejects_symlinked_sandbox_root_without_creating_outside() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let (mut runner, fake_device) = fake_runner(directory.path());
    let outside = directory.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::remove_dir(&fake_device).unwrap();
    symlink(&outside, &fake_device).unwrap();

    let mut plan = generated_create_plan();
    let result = runner.prepare_copy_commit_snapshots(&mut plan);

    assert!(result.unwrap_err().contains("safely"));
    assert!(!outside.join("storage").exists());
}

#[test]
#[cfg(unix)]
fn symlinked_sandbox_root_ancestor_after_review_cannot_redirect_creation() {
    use std::os::unix::fs::symlink;

    let review_directory = tempfile::tempdir().unwrap();
    let mut plan = generated_create_plan();
    adapter_runner(review_directory.path(), AdapterCreateResult::Created, false)
        .prepare_copy_commit_snapshots(&mut plan)
        .unwrap();

    let simulation_directory = tempfile::tempdir().unwrap();
    let selected_root = simulation_directory.path().join("selected-root");
    std::fs::create_dir(&selected_root).unwrap();
    let (mut runner, fake_device) = fake_runner(&selected_root);
    let target = target_in(&fake_device);

    let displaced_root = simulation_directory.path().join("displaced-root");
    std::fs::rename(&selected_root, &displaced_root).unwrap();
    let outside = simulation_directory.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::create_dir(outside.join("fake-device")).unwrap();
    symlink(&outside, &selected_root).unwrap();

    let run = runner.run(&plan);

    assert!(!run.success);
    assert!(!outside.join("fake-device/storage").exists());
    assert_eq!(configuration_report(&run)["mayHaveChanged"], false);
    assert!(!target.exists());
}

#[test]
#[cfg(unix)]
fn stale_authority_after_review_blocks_creation_and_changes_the_digest() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runner, fake_device) = fake_runner(directory.path());
    let mut plan = generated_create_plan();
    runner.prepare_copy_commit_snapshots(&mut plan).unwrap();
    let reviewed_digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
    let crate::planner::ExecutionParamValue::Literal { value } =
        plan.steps[0].params.get_mut("source").unwrap()
    else {
        panic!("configuration artifact must be literal frozen plan data");
    };
    value["absencePrecondition"]["authority"] = serde_json::json!("unrecognized");
    let tampered_digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();

    let run = runner.run(&plan);

    assert_ne!(reviewed_digest, tampered_digest);
    assert!(!run.success);
    assert_eq!(
        configuration_report(&run)["outcome"],
        "review_authority_stale"
    );
    assert!(!target_in(&fake_device).exists());
}

#[test]
fn adapter_outcomes_keep_partial_hash_and_lost_response_distinct() {
    for (adapter_outcome, expected, may_have_changed) in [
        (
            AdapterCreateResult::Existing,
            "blocked_existing_target",
            false,
        ),
        (
            AdapterCreateResult::HashMismatch,
            "staged_source_hash_mismatch",
            false,
        ),
        (
            AdapterCreateResult::PartialWrite,
            "partial_write_terminal",
            true,
        ),
        (
            AdapterCreateResult::LostResponse,
            "completion_uncertain",
            true,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut runner = adapter_runner(directory.path(), adapter_outcome, false);
        let mut plan = generated_create_plan();
        runner.prepare_copy_commit_snapshots(&mut plan).unwrap();
        bind_operation_fence(&mut runner, &plan, directory.path());

        let run = runner.run(&plan);

        assert!(!run.success, "{adapter_outcome:?} must fail the step");
        let report = configuration_report(&run);
        assert_eq!(report["outcome"], expected);
        assert_eq!(report["mayHaveChanged"], may_have_changed);
        assert_eq!(report["verified"], false);
        if matches!(adapter_outcome, AdapterCreateResult::LostResponse) {
            assert_eq!(
                report["cleanup"]["status"],
                "deferred_due_uncertain_outcome"
            );
        }
    }
}

#[test]
fn pre_dispatch_failure_cleans_staging_before_recording_no_dispatch() {
    for fail_cleanup in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut runner = adapter_runner(
            directory.path(),
            AdapterCreateResult::PreDispatchFailure,
            fail_cleanup,
        );
        let mut plan = generated_create_plan();
        runner.prepare_copy_commit_snapshots(&mut plan).unwrap();
        bind_operation_fence(&mut runner, &plan, directory.path());

        let run = runner.run(&plan);

        assert!(!run.success);
        assert_eq!(
            configuration_report(&run)["outcome"],
            "pre_dispatch_failed_no_target_write"
        );
        let report = configuration_report(&run);
        assert_eq!(report["mayHaveChanged"], false);
        assert_eq!(report["verified"], false);
        assert_eq!(
            report["cleanup"]["status"],
            if fail_cleanup { "warning" } else { "complete" }
        );
        assert!(runner.adapters().device().stage_cleanup_attempted);

        let fence =
            crate::operation_fence::read(&directory.path().join("operation-fence.v1")).unwrap();
        assert_eq!(
            fence.phase,
            crate::operation_fence::OperationFencePhase::RemoteQuiescent
        );
        assert_eq!(
            fence.outcome,
            Some(crate::operation_fence::OperationFenceOutcome::NotDispatched)
        );
        assert_eq!(fence.host_cleanup_complete, !fail_cleanup);
    }
}

#[test]
fn verified_creation_keeps_cleanup_warnings_separate_from_file_outcome() {
    let directory = tempfile::tempdir().unwrap();
    let mut runner = adapter_runner(directory.path(), AdapterCreateResult::Created, true);
    let mut plan = generated_create_plan();
    runner.prepare_copy_commit_snapshots(&mut plan).unwrap();
    bind_operation_fence(&mut runner, &plan, directory.path());

    let run = runner.run(&plan);

    assert!(run.success, "{run:?}");
    let report = configuration_report(&run);
    assert_eq!(report["outcome"], "created_verified");
    assert_eq!(report["verified"], true);
    assert_eq!(report["cleanup"]["status"], "warning");
    assert_eq!(
        report["cleanup"]["warnings"],
        serde_json::json!([
            "helper_cleanup_failed",
            "staged_source_cleanup_failed",
            "staging_directory_cleanup_failed"
        ])
    );
}

#[test]
fn lost_staging_directory_creation_response_defers_cleanup_truthfully() {
    let directory = tempfile::tempdir().unwrap();
    let mut runner = adapter_runner_with_stage_creation_response(
        directory.path(),
        AdapterCreateResult::Created,
        false,
        true,
    );
    let mut plan = generated_create_plan();
    runner.prepare_copy_commit_snapshots(&mut plan).unwrap();
    bind_operation_fence(&mut runner, &plan, directory.path());
    let result = runner.run(&plan);
    let report = configuration_report(&result);

    assert!(!result.success);
    assert_eq!(report["outcome"], "staging_failed_no_target_write");
    assert_eq!(report["cleanup"]["status"], "warning");
    assert_eq!(
        report["cleanup"]["warnings"],
        serde_json::json!(["staging_directory_cleanup_deferred_creation_uncertain"])
    );
    assert!(runner.adapters().device().stage_created);
    assert!(!runner.adapters().device().stage_cleanup_attempted);
    assert!(!runner.adapters().device().created);
}

#[test]
fn real_configuration_write_requires_native_claim_and_valid_precondition() {
    use crate::execution_session::ExecutionMode;
    use crate::planner::TargetDeviceBinding;
    use crate::trusted_control::{
        control_mac_input, NativeConfigurationWriteClaim, TrustedControlVerifier,
        VerifiedTrustedControl,
    };
    use ring::hmac;

    fn authenticated(claim: &NativeConfigurationWriteClaim) -> VerifiedTrustedControl {
        const KEY: [u8; 32] = [0x39; 32];
        let claim = serde_json::to_value(claim).unwrap();
        let payload = serde_json::json!({});
        let request_id = "config-write-test";
        let input = control_mac_input(7, 1, request_id, "startExecution", &payload, Some(&claim));
        let signature = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &KEY), &input);
        let mut request = serde_json::json!({
            "id": request_id,
            "type": "startExecution",
            "payload": payload,
            "trustedControl": {
                "version": 1,
                "runtimeGeneration": 7,
                "sequence": 1,
                "claims": claim,
                "mac": hex::encode(signature.as_ref())
            }
        });
        TrustedControlVerifier::new(KEY, 7)
            .verify_request(&mut request)
            .unwrap()
    }

    let temporary = tempfile::tempdir().unwrap();
    let mut plan = generated_create_plan();
    let target = TargetDeviceBinding {
        serial: "config-device".to_string(),
        manufacturer: None,
        model: None,
        android_api_level: None,
    };
    plan.target_device = Some(target.clone());
    let mut runner = adapter_runner(temporary.path(), AdapterCreateResult::Created, false);
    runner.prepare_copy_commit_snapshots(&mut plan).unwrap();
    let artifact = match plan.steps[0].params.get_mut("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => panic!("configuration source must be a frozen artifact"),
    };
    artifact["backupMode"] = serde_json::json!("disabled");
    artifact["backupWaiverDigest"] = serde_json::Value::Null;

    let make_claim = |plan: &ExecutionPlan| {
        let artifact = match plan.steps[0].params.get("source").unwrap() {
            crate::planner::ExecutionParamValue::Literal { value } => value,
            _ => unreachable!(),
        };
        let operation = artifact["operation"].as_str().unwrap();
        let precondition_key = match operation {
            "create" => "absencePrecondition",
            "replace" => "originalFilePrecondition",
            _ => unreachable!(),
        };
        NativeConfigurationWriteClaim {
            version: 1,
            purpose: "retroarch_configuration_write".to_string(),
            runtime_generation: 7,
            nonce: "ab".repeat(16),
            review_handle: "review-config".to_string(),
            plan_digest: crate::plan_digest::execution_plan_digest(plan).unwrap(),
            device_handle: "device-handle".to_string(),
            device_serial: "config-device".to_string(),
            session_epoch: 2,
            qualification_context_digest: "cd".repeat(32),
            profile_id: plan.source.device_profile_ref.clone(),
            target_identity: artifact["targetIdentity"].clone(),
            destination:
                "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg"
                    .to_string(),
            operation: operation.to_string(),
            source_sha256: artifact["sha256"].as_str().unwrap().to_string(),
            original_file_precondition: artifact[precondition_key].clone(),
            backup_mode: artifact["backupMode"].as_str().unwrap().to_string(),
            backup_waiver_digest: None,
        }
    };

    let digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
    let fence_directory = tempfile::tempdir().unwrap();
    let fence_path = fence_directory.path().join("active-operation.json");
    let fence_record = crate::operation_fence::OperationFenceRecord {
        version: 1,
        operation_id: "g7-a1-config-create".to_string(),
        runtime_generation: 7,
        attempt_sequence: 1,
        device_handle: "device-handle".to_string(),
        plan_digest: digest.clone(),
        phase: crate::operation_fence::OperationFencePhase::Dispatching,
        outcome: None,
        host_cleanup_complete: false,
        target: None,
        backup: None,
    };
    crate::operation_fence::write_initial(&fence_path, &fence_record).unwrap();
    let operation_fence = crate::operation_fence::OperationFenceBinding {
        path: fence_path,
        record: fence_record,
    };
    let valid_claim = make_claim(&plan);
    assert!(
        crate::request::validate_native_configuration_write_authorization(
            &plan,
            &digest,
            ExecutionMode::Real,
            Some(&target),
            &VerifiedTrustedControl::without_claims(),
            None,
        )
        .is_err()
    );
    assert!(
        crate::request::validate_native_configuration_write_authorization(
            &plan,
            &digest,
            ExecutionMode::Real,
            Some(&target),
            &authenticated(&valid_claim),
            Some(&operation_fence),
        )
        .is_ok()
    );

    let crate::planner::ExecutionParamValue::Literal { value: artifact } =
        plan.steps[0].params.get_mut("source").unwrap()
    else {
        unreachable!();
    };
    artifact["absencePrecondition"]["status"] = serde_json::json!("unrecognized");
    let malformed_digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
    let malformed_claim = make_claim(&plan);
    assert!(
        crate::request::validate_native_configuration_write_authorization(
            &plan,
            &malformed_digest,
            ExecutionMode::Real,
            Some(&target),
            &authenticated(&malformed_claim),
            Some(&operation_fence),
        )
        .is_err()
    );

    let recipe_id = "app.retroarch.configure";
    let mut replace_bindings = OrderedMap::new();
    replace_bindings.insert(
        format!("{recipe_id}/retroarch_cfg_source"),
        serde_json::json!("managed"),
    );
    replace_bindings.insert(
        format!("{recipe_id}/retroarch_cfg_write_operation"),
        serde_json::json!("replace"),
    );
    replace_bindings.insert(
        format!("{recipe_id}/retroarch_video_threaded"),
        serde_json::json!(false),
    );
    let mut replace_request = plan_request(replace_bindings);
    replace_request.selected_recipes = Some(vec![recipe_id.to_string()]);
    let mut replace_plan = plan_configuration(replace_request)
        .unwrap()
        .plan
        .expect("Replace request should produce an execution plan");
    replace_plan.target_device = Some(target.clone());
    let (mut replace_runner, _) =
        replace_adapter_runner(temporary.path(), b"original config", true);
    replace_runner
        .prepare_copy_commit_snapshots(&mut replace_plan)
        .expect("review should bind the produced original-file precondition");
    let replace_digest = crate::plan_digest::execution_plan_digest(&replace_plan).unwrap();
    let replace_artifact = match replace_plan.steps[0].params.get("source").unwrap() {
        crate::planner::ExecutionParamValue::Literal { value } => value,
        _ => unreachable!(),
    };
    assert_eq!(replace_artifact["operation"], "replace");
    assert_eq!(
        replace_artifact["originalFilePrecondition"]["sha256"],
        crate::runtime_configuration::sha256_hex(b"original config")
    );
    let replace_claim = make_claim(&replace_plan);
    let replace_fence_directory = tempfile::tempdir().unwrap();
    let replace_fence_path = replace_fence_directory
        .path()
        .join("replace-operation.json");
    let replace_fence_record = crate::operation_fence::OperationFenceRecord {
        version: 1,
        operation_id: "replace-config-test".to_string(),
        runtime_generation: 7,
        attempt_sequence: 1,
        device_handle: "device-handle".to_string(),
        plan_digest: replace_digest.clone(),
        phase: crate::operation_fence::OperationFencePhase::Dispatching,
        outcome: None,
        host_cleanup_complete: false,
        target: None,
        backup: None,
    };
    crate::operation_fence::write_initial(&replace_fence_path, &replace_fence_record).unwrap();
    let replace_fence = crate::operation_fence::OperationFenceBinding {
        path: replace_fence_path,
        record: replace_fence_record,
    };
    assert!(
        crate::request::validate_native_configuration_write_authorization(
            &replace_plan,
            &replace_digest,
            ExecutionMode::Real,
            Some(&target),
            &authenticated(&replace_claim),
            Some(&replace_fence),
        )
        .is_ok(),
        "the precondition produced by Replace review must pass native admission"
    );

    let mut legacy_shape_claim = make_claim(&replace_plan);
    legacy_shape_claim.original_file_precondition = serde_json::json!({
        "status": "confirmed_existing",
        "originalSha256": crate::runtime_configuration::sha256_hex(b"original config"),
        "byteLength": b"original config".len(),
    });
    assert!(
        crate::request::validate_native_configuration_write_authorization(
            &replace_plan,
            &replace_digest,
            ExecutionMode::Real,
            Some(&target),
            &authenticated(&legacy_shape_claim),
            Some(&replace_fence),
        )
        .is_err(),
        "legacy runtime field names must not pass the native claim contract"
    );
}
