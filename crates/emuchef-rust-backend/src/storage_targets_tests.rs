use indexmap::IndexMap;
use serde_json::{json, Value};
use std::path::PathBuf;

use super::storage_targets::*;
use crate::model::{
    InputDeclaration, InputOption, InputValidation, OrderedMap, ParamValue, Recipe, RecipeProvides,
    Step, StepConstraints,
};
use crate::planner::{
    plan_execution_with_app_definitions, DeviceContext, PlannerInput, PlanningStatus,
    RuntimeCapabilities,
};

#[test]
fn setup_location_reference_is_a_single_string_input() {
    let declaration = InputDeclaration {
        type_name: "setup_location_ref".to_string(),
        role: "destination".to_string(),
        label: "ROM destination".to_string(),
        description: None,
        required: true,
        multiple: false,
        validation: InputValidation {
            must_exist: false,
            allowed_extensions: Vec::new(),
            path_kind: None,
            allowed_prefixes: Vec::new(),
        },
        default: Value::Null,
        options: Vec::<InputOption>::new(),
        sensitive: false,
        advanced: false,
        metadata: IndexMap::new(),
    };

    assert!(declaration.value_matches_type(&json!("loc-123")));
    assert!(!declaration.value_matches_type(&json!(["loc-123"])));
}

#[test]
fn setup_location_reference_is_accepted_as_copy_files_destination() {
    let mut recipe = Recipe {
        schema_version: 1,
        kind: "recipe".to_string(),
        id: "test.setup_location".to_string(),
        name: "test.setup_location".to_string(),
        description: None,
        recipe_dependencies: Vec::new(),
        provides: RecipeProvides {
            features: Vec::new(),
        },
        inputs: OrderedMap::new(),
        config: None,
        artifacts: OrderedMap::new(),
        artifact_groups: OrderedMap::new(),
        steps: Vec::new(),
    };
    let input = |type_name: &str, label: &str| InputDeclaration {
        type_name: type_name.to_string(),
        role: "generic".to_string(),
        label: label.to_string(),
        description: None,
        required: true,
        multiple: false,
        validation: InputValidation {
            must_exist: false,
            allowed_extensions: Vec::new(),
            path_kind: None,
            allowed_prefixes: Vec::new(),
        },
        default: Value::Null,
        options: Vec::new(),
        sensitive: false,
        advanced: false,
        metadata: OrderedMap::new(),
    };
    recipe
        .inputs
        .insert("rom_source".to_string(), input("directory", "ROM source"));
    recipe.inputs.insert(
        "rom_destination".to_string(),
        input("setup_location_ref", "ROM destination"),
    );

    let mut params = OrderedMap::new();
    params.insert(
        "source".to_string(),
        ParamValue::Ref("inputs.rom_source".to_string()),
    );
    params.insert(
        "dest".to_string(),
        ParamValue::Ref("inputs.rom_destination".to_string()),
    );
    params.insert(
        "copy_policy".to_string(),
        ParamValue::Literal(json!("merge")),
    );
    recipe.steps = vec![Step {
        id: "copy_roms".to_string(),
        type_name: "copy_files".to_string(),
        name: "Copy ROMs".to_string(),
        description: None,
        progress_note: None,
        user_toggleable: false,
        app_ref: None,
        dependencies: Vec::new(),
        constraints: StepConstraints {
            capabilities: Vec::new(),
            conflicts_with: Vec::new(),
        },
        skip_if: Vec::new(),
        params,
        verify: Vec::new(),
    }];
    let mut planner_input = PlannerInput {
        device_plan_presentation: None,
        recipes: vec![recipe],
        selected_recipe_refs: vec!["test.setup_location".to_string()],
        explicit_input_bindings: OrderedMap::new(),
        user_configuration_bindings: OrderedMap::new(),
        device_plan_input_bindings: OrderedMap::new(),
        plan_id: "plan.recipe_expansion.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: DeviceContext {
            manufacturer: "Example".to_string(),
            model: "Device".to_string(),
            android_version: 13,
            android_api_level: Some(33),
            device_tags: Vec::new(),
            root_shell_mode: None,
        },
        runtime_capabilities: RuntimeCapabilities {
            adb_available: true,
            apk_install: false,
            shared_storage_write: true,
            app_launch: false,
            shell_command: false,
            package_remove_for_user: false,
            root_shell: false,
            app_data_write: false,
            root_shell_route: None,
        },
        catalog_identity: None,
        target_device: None,
    };
    planner_input
        .explicit_input_bindings
        .insert("test.setup_location/rom_source".to_string(), json!("/tmp"));
    planner_input.explicit_input_bindings.insert(
        "test.setup_location/rom_destination".to_string(),
        json!("location-roms"),
    );

    let result = plan_execution_with_app_definitions(planner_input, &[]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
}

#[test]
fn setup_location_names_are_trimmed_bounded_and_unambiguous() {
    assert_eq!(validate_setup_location_name("  BIOS  ").unwrap(), "BIOS");
    assert!(validate_setup_location_name(" \t ").is_err());
    assert!(validate_setup_location_name("a\nb").is_err());
    assert!(validate_setup_location_name(&"x".repeat(MAX_SETUP_LOCATION_NAME_BYTES + 1)).is_err());
}

#[test]
fn relative_location_paths_reject_escape_and_ambiguous_syntax() {
    assert_eq!(
        validate_relative_location_path("Games/ROMs/名作").unwrap(),
        "Games/ROMs/名作"
    );
    for invalid in [
        "",
        "/storage/emulated/0",
        "../escape",
        "Games/../escape",
        "Games//ROMs",
        "Games/ROMs/",
        "Games\\ROMs",
        "C:/Games",
        "Games/./ROMs",
        "Games/ROM\0s",
    ] {
        assert!(
            validate_relative_location_path(invalid).is_err(),
            "{invalid:?}"
        );
    }
}

#[test]
fn only_supported_reported_volume_roots_are_eligible() {
    assert_eq!(
        validate_volume_root(
            StorageTargetVolumeKind::InternalShared,
            "/storage/emulated/0"
        )
        .unwrap(),
        "/storage/emulated/0"
    );
    assert_eq!(
        validate_volume_root(StorageTargetVolumeKind::RemovableSd, "/storage/1234-ABCD").unwrap(),
        "/storage/1234-ABCD"
    );
    for (kind, root) in [
        (
            StorageTargetVolumeKind::InternalShared,
            "/storage/1234-ABCD",
        ),
        (StorageTargetVolumeKind::RemovableSd, "/storage/emulated/0"),
        (
            StorageTargetVolumeKind::RemovableSd,
            "/mnt/media_rw/1234-ABCD",
        ),
        (StorageTargetVolumeKind::RemovableSd, "/storage/../data"),
    ] {
        assert!(validate_volume_root(kind, root).is_err(), "{root:?}");
    }
}

#[test]
fn writer_collision_uses_effects_and_allows_shared_parent_creation() {
    let parent = PathBuf::from("/storage/card/Games");
    let child = PathBuf::from("/storage/card/Games/BIOS");
    assert!(!writer_effects_conflict(
        &[WriterEffect::EnsureDirectory(parent.clone())],
        &[WriterEffect::EnsureDirectory(child.clone())],
    ));
    assert!(writer_effects_conflict(
        &[WriterEffect::WriteFile(parent.join("game.zip"))],
        &[WriterEffect::WriteFile(parent.join("game.zip"))],
    ));
    assert!(writer_effects_conflict(
        &[WriterEffect::DeleteDirectory(parent)],
        &[WriterEffect::WriteFile(child.join("rom.zip"))],
    ));
}
