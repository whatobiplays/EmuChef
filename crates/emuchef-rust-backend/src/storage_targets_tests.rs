use indexmap::IndexMap;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::storage_targets::*;
use crate::model::{
    InputDeclaration, InputOption, InputValidation, OrderedMap, ParamValue, Recipe, RecipeProvides,
    Step, StepConstraints,
};
use crate::planner::{
    plan_execution_with_app_definitions, DeviceContext, PlannerInput, PlanningStatus,
    RuntimeCapabilities,
};

const SETUP_LOCATION_RECIPE_YAML: &str = "schema_version: 1\nkind: recipe\nid: test.setup_location\nname: Setup Location\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs:\n  rom_source:\n    type: directory\n    role: source\n    label: ROM source\n    required: true\n  rom_destination:\n    type: setup_location_ref\n    role: destination\n    label: ROM destination\n    required: true\nartifacts: {}\nsteps:\n  - id: copy_roms\n    type: copy_files\n    name: Copy ROMs\n    user_toggleable: false\n    dependencies: []\n    constraints:\n      capabilities: []\n      conflicts_with: []\n    skip_if: []\n    params:\n      source: {ref: inputs.rom_source}\n      dest: {ref: inputs.rom_destination}\n      copy_policy: merge\n    verify: []\n";

fn write_setup_location_recipe(root: &Path) -> PathBuf {
    for directory in ["apps", "device_profiles", "device_plans"] {
        std::fs::create_dir_all(root.join(directory)).expect("create catalog directory");
    }
    let recipes = root.join("recipes");
    std::fs::create_dir_all(&recipes).expect("create recipe catalog");
    let path = recipes.join("setup-location.yaml");
    std::fs::write(&path, SETUP_LOCATION_RECIPE_YAML).expect("write Setup Location Recipe");
    path
}

fn load_setup_location_recipe() -> Recipe {
    crate::yaml::load_recipe_from_bytes(
        SETUP_LOCATION_RECIPE_YAML.as_bytes(),
        Path::new("recipes/setup-location.yaml"),
    )
    .expect("Setup Location Recipe fixture should parse")
}

#[test]
fn copy_files_destination_contract_lists_setup_location_ref_without_changing_legacy_contract() {
    let destination = &crate::step_specs::step_spec_for("copy_files")
        .expect("copy_files StepSpec")
        .params["dest"];

    assert_eq!(
        destination.accepted_sources,
        ["literal", "input_ref", "app_target"]
    );
    assert_eq!(
        destination.accepted_value_types,
        ["device_path", "setup_location_ref"]
    );
}

#[test]
fn setup_location_ref_passes_shared_recipe_validation() {
    let root = tempfile::tempdir().expect("temporary catalog");
    let path = write_setup_location_recipe(root.path());
    let validation = crate::validation::validate_recipe_path_result(&path, None);
    assert_eq!(
        validation["diagnostics"]
            .as_array()
            .expect("diagnostic list")
            .iter()
            .filter(|diagnostic| diagnostic["severity"] == "error")
            .count(),
        0,
        "shared Recipe validation: {validation}"
    );
}

#[test]
fn setup_location_ref_passes_catalog_validation() {
    let root = tempfile::tempdir().expect("temporary catalog");
    let path = write_setup_location_recipe(root.path());
    let (validated_paths, diagnostics) = crate::catalog::validate_authored_root(root.path());
    assert!(
        validated_paths.contains(&crate::yaml::resolved_path_string(&path)),
        "valid Recipe should remain in catalog validation results: {validated_paths:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic["severity"] == "error"),
        "catalog diagnostics: {diagnostics:?}"
    );
}

#[test]
fn setup_location_ref_passes_recipe_validation_through_the_cli() {
    let root = tempfile::tempdir().expect("temporary recipe root");
    let path = write_setup_location_recipe(root.path());
    let output = crate::cli::run(&[
        "validate".to_string(),
        "--authored-root".to_string(),
        root.path().to_string_lossy().into_owned(),
        path.to_string_lossy().into_owned(),
    ]);

    assert_eq!(output.exit_code, 0, "{}{}", output.stdout, output.stderr);
    assert!(output.stdout.contains("Validation status: success"));
}

#[test]
fn setup_location_ref_remains_rejected_for_unsupported_copy_parameters_and_input_shapes() {
    for parameter in ["source", "copy_policy"] {
        let mut recipe = load_setup_location_recipe();
        recipe.steps[0].params.insert(
            parameter.to_string(),
            ParamValue::Ref("inputs.rom_destination".to_string()),
        );
        let errors = crate::planner::validate_recipe_static_contracts(&recipe);
        assert!(
            errors
                .iter()
                .any(|error| error.code == "invalid_param_value_type"),
            "Setup Location must not become copy_files.{parameter}: {errors:?}"
        );
    }

    let mut unsupported_operation = load_setup_location_recipe();
    unsupported_operation.steps[0].type_name = "wait".to_string();
    unsupported_operation.steps[0].params.clear();
    unsupported_operation.steps[0].params.insert(
        "duration_ms".to_string(),
        ParamValue::Ref("inputs.rom_destination".to_string()),
    );
    let errors = crate::planner::validate_recipe_static_contracts(&unsupported_operation);
    assert!(
        errors
            .iter()
            .any(|error| error.code == "invalid_param_source"),
        "Setup Location must not become a wait input: {errors:?}"
    );

    let invalid_shapes = [
        "    multiple: true\n",
        "    default: location-roms\n",
        "    options:\n      - value: location-roms\n        label: ROMs\n",
    ];
    for extra in invalid_shapes {
        let yaml = SETUP_LOCATION_RECIPE_YAML.replace(
            "    label: ROM destination\n    required: true\n",
            &format!("    label: ROM destination\n    required: true\n{extra}"),
        );
        assert!(
            crate::yaml::load_recipe_from_bytes(yaml.as_bytes(), Path::new("recipe.yaml")).is_err(),
            "Setup Location input must reject {extra:?}"
        );
    }
}

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
    assert!(!declaration.value_matches_type(&json!(7)));
    assert!(!declaration.value_matches_type(&json!(true)));
    assert!(!declaration.value_matches_type(&json!({ "id": "loc-123" })));
    assert!(!declaration.value_matches_type(&Value::Null));
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
