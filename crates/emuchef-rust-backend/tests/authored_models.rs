use std::fs;
use std::path::{Path, PathBuf};

use emuchef_rust_backend::authored_models::{
    emit_app_definition_yaml, emit_device_profile_yaml, parse_app_definition_yaml,
    parse_device_profile_yaml, validate_app_definition, validate_device_profile,
};
use emuchef_rust_backend::{catalog, validation};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn yaml_files(directory: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("yaml" | "yml")
                )
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn every_checked_in_app_definition_is_valid_and_canonicalizes_idempotently() {
    let paths = yaml_files(&repo_root().join("authored/apps"));
    assert_eq!(paths.len(), 3);

    for path in paths {
        let source = fs::read_to_string(&path).unwrap();
        let parsed = parse_app_definition_yaml(&source)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(validate_app_definition(&parsed), [], "{}", path.display());
        let canonical = emit_app_definition_yaml(&parsed).unwrap();
        let reparsed = parse_app_definition_yaml(&canonical).unwrap();
        assert_eq!(reparsed, parsed, "{}", path.display());
        assert_eq!(emit_app_definition_yaml(&reparsed).unwrap(), canonical);
    }
}

#[test]
fn every_checked_in_device_profile_is_valid_and_canonicalizes_idempotently() {
    let paths = yaml_files(&repo_root().join("authored/device_profiles"));
    assert_eq!(paths.len(), 5);

    for path in paths {
        let source = fs::read_to_string(&path).unwrap();
        let parsed = parse_device_profile_yaml(&source)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(validate_device_profile(&parsed), [], "{}", path.display());
        let canonical = emit_device_profile_yaml(&parsed).unwrap();
        let reparsed = parse_device_profile_yaml(&canonical).unwrap();
        assert_eq!(reparsed, parsed, "{}", path.display());
        assert_eq!(emit_device_profile_yaml(&reparsed).unwrap(), canonical);
    }
}

#[test]
fn catalog_model_validation_reports_all_files_in_stable_relative_path_order() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("apps")).unwrap();
    fs::create_dir_all(root.join("device_profiles")).unwrap();
    fs::write(root.join("apps/broken.yaml"), "kind: app_definition\n").unwrap();
    fs::write(
        root.join("device_profiles/broken.yaml"),
        include_str!("../../../authored/device_profiles/ayaneo.pocket_s2.yaml")
            .replace("    - 'Pocket S2'", "    - '['"),
    )
    .unwrap();

    let diagnostics = catalog::validate_authored_catalog_models(root);
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0]["code"], "app_definition_yaml_invalid");
    assert_eq!(diagnostics[0]["file"], "apps/broken.yaml");
    assert_eq!(diagnostics[1]["code"], "device_model_pattern_invalid");
    assert_eq!(diagnostics[1]["file"], "device_profiles/broken.yaml");
    assert_eq!(diagnostics[1]["field"], "match.model_patterns[0]");
}

#[test]
fn malformed_models_do_not_suppress_valid_recipe_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for directory in ["apps", "recipes", "device_profiles", "device_plans"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    fs::write(
        root.join("apps/invalid-id.yaml"),
        include_str!("../../../authored/apps/retroarch.yaml")
            .replace("id: retroarch", "id: RetroArch"),
    )
    .unwrap();
    fs::write(
        root.join("device_profiles/invalid-regex.yaml"),
        include_str!("../../../authored/device_profiles/ayaneo.pocket_s2.yaml")
            .replace("    - 'Pocket S2'", "    - '['"),
    )
    .unwrap();
    let recipe_path = root.join("recipes/example.yaml");
    fs::write(
        &recipe_path,
        r#"schema_version: 1
kind: recipe
id: recipe.example
name: Example
recipe_dependencies: [recipe.missing]
provides: {features: []}
inputs: {}
artifacts: {}
artifact_groups: {}
steps: []
"#,
    )
    .unwrap();

    let result = validation::validate_recipe_path_result(&recipe_path, Some(root));
    let diagnostics = result["diagnostics"].as_array().unwrap();
    let codes = diagnostics
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        [
            "authored_id_invalid",
            "device_model_pattern_invalid",
            "recipe_not_found"
        ]
    );
}
