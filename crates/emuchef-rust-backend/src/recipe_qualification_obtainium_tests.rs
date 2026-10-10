//! Automated qualification for the authored Obtainium installation workflow.
//!
//! These tests bind expectations to the authored recipe bytes, plan the real
//! catalog through runtime configuration, inspect the production review, and
//! execute the unchanged generated plan through deterministic sandbox-root
//! adapters. No test in this module invokes ADB, a physical device, a live
//! network request, or an ignored physical-qualification harness.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::artifact_resolver::artifact_local_filename;
use crate::executor::{
    DeviceOperationError, ExecutorAdapters, ExecutorDevice, ExecutorRunner, FakeDryRunDevice,
    StepRunStatus,
};
use crate::planner::{
    ExecutionArtifact, ExecutionArtifactSource, ExecutionParamValue, ExecutionPlan, ExecutionStep,
};
use crate::runtime_configuration::PlanConfigurationResult;

const TARGET_RECIPE: &str = "app.obtainium.install";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.generic.base";
const TARGET_PACKAGE: &str = "dev.imranr.obtainium";
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObtainiumQualificationContract {
    schema_version: i64,
    target_recipe: String,
    planning_device_plan: String,
    authored_source: AuthoredSourceContract,
    selected_recipes: Vec<String>,
    expanded_recipes: Vec<String>,
    recipe_constraint_capabilities: Vec<String>,
    qualification_context_capabilities: Vec<String>,
    required_inputs: Vec<String>,
    optional_inputs: Vec<String>,
    required_operation_families: Vec<String>,
    material_dependency_edges: Vec<Vec<String>>,
    app_definition: AppDefinitionContract,
    release: ReleaseContract,
    artifact: ArtifactContract,
    install: InstallContract,
    live_network_required_for_automated_qualification: bool,
    automated_status: String,
    physical_status: String,
    physical_cleanup_authority: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppDefinitionContract {
    id: String,
    package_name: String,
    artifact_id: String,
    source_strategy: String,
    provider: String,
    base_url: String,
    repository: String,
    asset_pattern: String,
    prerelease: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseContract {
    metadata_fixture: String,
    cached_old_tag: String,
    selected_tag: String,
    asset_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthoredSourceContract {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactContract {
    id_suffix: String,
    #[serde(rename = "type")]
    type_name: String,
    app_ref: String,
    app_artifact: String,
    cache: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallContract {
    step_id_suffix: String,
    app_ref: String,
    replace_existing: bool,
    skip_condition_type: String,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate should live beneath the repository root")
        .to_path_buf()
}

fn authored_root() -> PathBuf {
    repository_root().join("authored")
}

fn contract_path() -> PathBuf {
    repository_root()
        .join("tests/fixtures/recipe-qualification/obtainium/qualification-contract.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_contract() -> ObtainiumQualificationContract {
    let text = fs::read_to_string(contract_path())
        .expect("Obtainium qualification contract should be readable");
    let contract: ObtainiumQualificationContract = serde_json::from_str(&text)
        .expect("Obtainium qualification contract should deserialize strictly");
    assert_eq!(
        contract.schema_version, 1,
        "contract schema version must be 1"
    );
    assert_eq!(contract.target_recipe, TARGET_RECIPE);
    assert_eq!(contract.planning_device_plan, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(contract.selected_recipes, vec![TARGET_RECIPE]);
    assert_eq!(contract.expanded_recipes, vec![TARGET_RECIPE]);
    assert_eq!(contract.recipe_constraint_capabilities, vec!["apk_install"]);
    assert_eq!(
        contract.qualification_context_capabilities,
        vec!["apk_install"]
    );
    assert!(contract.required_inputs.is_empty());
    assert!(contract.optional_inputs.is_empty());
    assert_eq!(
        contract.required_operation_families,
        vec!["resolve_artifacts", "install_apk"]
    );
    assert_eq!(
        contract.material_dependency_edges,
        vec![vec![
            "resolve_artifacts".to_string(),
            "install_obtainium".to_string()
        ]]
    );
    assert_eq!(contract.app_definition.id, "obtainium");
    assert_eq!(contract.app_definition.package_name, TARGET_PACKAGE);
    assert_eq!(contract.app_definition.artifact_id, "apk");
    assert_eq!(contract.app_definition.source_strategy, "latest_release");
    assert_eq!(contract.app_definition.provider, "github");
    assert_eq!(contract.app_definition.base_url, "https://github.com");
    assert_eq!(contract.app_definition.repository, "ImranR98/Obtainium");
    assert_eq!(contract.app_definition.asset_pattern, r"^app-release\.apk$");
    assert!(!contract.app_definition.prerelease);
    assert_eq!(contract.release.cached_old_tag, "v1.0.0");
    assert_eq!(contract.release.selected_tag, "v2.0.0");
    assert_eq!(contract.release.asset_name, "app-release.apk");
    assert_eq!(contract.artifact.id_suffix, "obtainium_apk");
    assert_eq!(contract.artifact.type_name, "remote_file");
    assert_eq!(contract.artifact.app_ref, contract.app_definition.id);
    assert_eq!(
        contract.artifact.app_artifact,
        contract.app_definition.artifact_id
    );
    assert_eq!(contract.artifact.cache, "default");
    assert_eq!(contract.install.step_id_suffix, "install_obtainium");
    assert_eq!(contract.install.app_ref, contract.app_definition.id);
    assert!(!contract.install.replace_existing);
    assert_eq!(contract.install.skip_condition_type, "package_installed");
    assert!(!contract.live_network_required_for_automated_qualification);
    assert_eq!(contract.automated_status, "qualified");
    assert_eq!(contract.physical_status, "deferred");
    assert_eq!(
        contract.physical_cleanup_authority,
        "not_authorized_for_recipe_qualification"
    );
    contract
}

/// Prepare the real authored Obtainium configuration through the production
/// runtime-configuration path. No target serial, device probe, or host input
/// binding is used.
fn plan_obtainium() -> PlanConfigurationResult {
    use crate::catalog_source::CatalogSnapshot;
    use crate::model::OrderedMap;
    use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    plan_configuration(ConfigurationContextRequest {
        catalog,
        configuration_root: None,
        user_configuration: None,
        device_plan: Some(QUALIFICATION_DEVICE_PLAN.to_string()),
        selected_recipes: Some(vec![TARGET_RECIPE.to_string()]),
        explicit_bindings: OrderedMap::new(),
        device_context: None,
        target_device: None,
        runtime_capability_availability: None,
    })
    .expect("real authored Obtainium configuration should prepare")
}

/// Map an authored step suffix to exactly one generated step for Obtainium.
fn generated_step<'a>(plan: &'a ExecutionPlan, authored_step_id: &str) -> &'a ExecutionStep {
    let suffix = format!("/{authored_step_id}");
    let mut matches = plan
        .steps
        .iter()
        .filter(|step| step.recipe_ref == TARGET_RECIPE && step.id.ends_with(&suffix));
    let step = matches
        .next()
        .unwrap_or_else(|| panic!("no generated step for authored id {authored_step_id}"));
    assert!(
        matches.next().is_none(),
        "multiple generated steps for authored id {authored_step_id}"
    );
    step
}

fn runtime_capability_enabled(plan: &ExecutionPlan, capability: &str) -> bool {
    match capability {
        "apk_install" => plan.runtime_capabilities.apk_install,
        other => panic!("unexpected Obtainium runtime capability {other}"),
    }
}

#[test]
fn obtainium_contract_binds_current_source_and_deferred_physical_status() {
    let contract = load_contract();
    let source_path = Path::new(&contract.authored_source.path);
    assert!(
        source_path.is_relative(),
        "contract source path must be repository-relative"
    );
    assert_eq!(
        source_path,
        Path::new("authored/recipes/app.obtainium.install.yaml")
    );
    let resolved = repository_root().join(source_path);
    let canonical_root = repository_root()
        .canonicalize()
        .expect("repo root should canonicalize");
    let canonical_source = resolved
        .canonicalize()
        .expect("authored Obtainium recipe should resolve");
    assert!(
        canonical_source.starts_with(canonical_root),
        "contract source path must not escape the repository root"
    );
    let raw = fs::read(&resolved).expect("authored Obtainium recipe should be readable");
    assert_eq!(
        sha256_hex(&raw),
        contract.authored_source.sha256,
        "authored Obtainium recipe changed; qualification expectations must be reviewed"
    );
    assert_eq!(contract.app_definition.source_strategy, "latest_release");
    assert_eq!(contract.app_definition.provider, "github");
    assert_eq!(contract.app_definition.base_url, "https://github.com");
    assert_eq!(contract.app_definition.repository, "ImranR98/Obtainium");
    assert_eq!(contract.app_definition.asset_pattern, r"^app-release\.apk$");
    assert!(!contract.app_definition.prerelease);
}

#[test]
fn obtainium_real_authored_plan_and_review_match_qualification_contract() {
    let contract = load_contract();
    let result = plan_obtainium();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != "error"),
        "planning must produce no error diagnostics"
    );
    let plan = result
        .plan
        .as_ref()
        .expect("Obtainium plan should be generated");
    assert!(
        result
            .plan_digest
            .as_deref()
            .is_some_and(|digest| !digest.is_empty()),
        "plan digest should be present"
    );
    assert_eq!(plan.source.selected_recipe_refs, contract.selected_recipes);
    assert_eq!(plan.source.expanded_recipe_refs, contract.expanded_recipes);
    assert_eq!(plan.source.device_plan_ref, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(plan.source.device_profile_ref, "ayaneo.generic");
    assert!(plan.recipes.iter().all(|recipe| recipe.id == TARGET_RECIPE));
    assert_eq!(plan.steps.len(), 2);
    assert!(plan
        .steps
        .iter()
        .all(|step| step.recipe_ref == TARGET_RECIPE));

    let mut families = plan
        .steps
        .iter()
        .map(|step| step.type_name.clone())
        .collect::<Vec<_>>();
    families.sort();
    families.dedup();
    let mut expected_families = contract.required_operation_families.clone();
    expected_families.sort();
    assert_eq!(families, expected_families);

    let install_step = plan
        .steps
        .iter()
        .find(|step| step.type_name == "install_apk")
        .expect("Obtainium plan should contain an install_apk step");
    assert_eq!(
        install_step.constraints.capabilities, contract.recipe_constraint_capabilities,
        "the Obtainium install step must retain the authored apk_install requirement"
    );
    let resolve_step = plan
        .steps
        .iter()
        .find(|step| step.type_name == "resolve_artifacts")
        .expect("Obtainium plan should contain a resolve_artifacts step");
    assert!(resolve_step.constraints.capabilities.is_empty());
    for capability in &contract.qualification_context_capabilities {
        assert!(runtime_capability_enabled(plan, capability));
    }
    assert!(!plan.runtime_capabilities.root_shell);
    assert!(result.resolved_inputs.is_empty());

    let artifact = plan
        .artifacts
        .iter()
        .find(|artifact| {
            artifact
                .id
                .ends_with(&format!("/{}", contract.artifact.id_suffix))
        })
        .expect("Obtainium APK artifact should be emitted");
    assert_eq!(plan.artifacts.len(), 1);
    assert_eq!(artifact.type_name, contract.artifact.type_name);
    assert!(matches!(
        &artifact.source,
        ExecutionArtifactSource::RemoteRelease {
            provider,
            service_origin,
            repository,
            include_prereleases,
            asset_pattern,
            invert_asset_pattern: false,
            ..
        }
            if provider == &contract.app_definition.provider
                && service_origin == &contract.app_definition.base_url
                && repository == &contract.app_definition.repository
                && include_prereleases == &contract.app_definition.prerelease
                && asset_pattern.as_deref() == Some(contract.app_definition.asset_pattern.as_str())
    ));
    assert_eq!(
        artifact
            .app_provenance
            .as_ref()
            .map(|provenance| provenance.app_id.as_str()),
        Some(contract.app_definition.id.as_str())
    );
    assert_eq!(
        artifact
            .app_provenance
            .as_ref()
            .map(|provenance| provenance.artifact_id.as_str()),
        Some(contract.app_definition.artifact_id.as_str())
    );
    assert_eq!(artifact.cache, contract.artifact.cache);

    let serialized_plan = serde_json::to_value(plan).expect("plan should serialize");
    let parsed_plan = crate::cli::parse_execution_plan_json(&serialized_plan)
        .expect("serialized Obtainium plan should parse");
    let parsed_artifact = parsed_plan
        .artifacts
        .iter()
        .find(|candidate| candidate.id == artifact.id)
        .expect("parsed plan should retain the Obtainium APK artifact");
    assert!(matches!(
        &parsed_artifact.source,
        ExecutionArtifactSource::RemoteRelease {
            provider,
            service_origin,
            repository,
            include_prereleases,
            asset_pattern,
            invert_asset_pattern: false,
            ..
        }
            if provider == &contract.app_definition.provider
                && service_origin == &contract.app_definition.base_url
                && repository == &contract.app_definition.repository
                && include_prereleases == &contract.app_definition.prerelease
                && asset_pattern.as_deref() == Some(contract.app_definition.asset_pattern.as_str())
    ));
    assert_eq!(parsed_artifact.cache, contract.artifact.cache);
    assert_eq!(
        parsed_artifact.app_provenance.as_ref().map(|provenance| (
            provenance.app_id.as_str(),
            provenance.artifact_id.as_str(),
            provenance.kind.as_str(),
        )),
        Some((
            contract.app_definition.id.as_str(),
            contract.app_definition.artifact_id.as_str(),
            "apk",
        ))
    );

    let resolve = generated_step(plan, "resolve_artifacts");
    assert_eq!(resolve.type_name, "resolve_artifacts");
    let install = generated_step(plan, &contract.install.step_id_suffix);
    assert_eq!(install.type_name, "install_apk");
    assert_eq!(
        install.app_id.as_deref(),
        Some(contract.install.app_ref.as_str())
    );
    assert!(install.dependencies.contains(&resolve.id));
    assert_eq!(
        install.params.get("app"),
        Some(&ExecutionParamValue::Ref {
            ref_value: format!("artifacts.{TARGET_RECIPE}/obtainium_apk.local_path"),
        })
    );
    assert_eq!(
        install.params.get("replace_existing"),
        Some(&ExecutionParamValue::Literal {
            value: Value::Bool(contract.install.replace_existing),
        })
    );
    assert_eq!(install.skip_if.len(), 1);
    assert_eq!(
        install.skip_if[0].type_name,
        contract.install.skip_condition_type
    );
    assert_eq!(
        install.skip_if[0].app_id.as_deref(),
        Some(contract.install.app_ref.as_str())
    );
    assert!(install.skip_if[0].params.is_empty());
    assert_eq!(plan.apps[0].id, contract.app_definition.id);
    assert_eq!(
        plan.apps[0].package_id,
        contract.app_definition.package_name
    );

    let review = result
        .review
        .as_ref()
        .expect("production review should exist");
    assert!(review.can_execute);
    assert_eq!(review.features.len(), 1);
    assert_eq!(review.features[0].name, "Install Obtainium");
    assert!(!review.features[0].automatically_added);
    let install_action = review.features[0]
        .sections
        .iter()
        .find(|section| section.kind == "installs")
        .and_then(|section| section.actions.first())
        .expect("review should describe the Obtainium install action");
    let description = install_action
        .description
        .as_deref()
        .expect("review should explain the stable GitHub release source");
    assert_eq!(
        description,
        "The latest eligible stable GitHub APK for Obtainium is resolved during execution after matching filename pattern ^app-release\\.apk$."
    );
    assert!(!description.contains("direct HTTPS"));
    let section_kinds = review.features[0]
        .sections
        .iter()
        .map(|section| section.kind)
        .collect::<Vec<_>>();
    assert!(section_kinds.contains(&"downloads"));
    assert!(section_kinds.contains(&"installs"));
    assert!(review.inputs.is_empty());
    assert_eq!(review.work.action_count, plan.steps.len());
    assert!(review
        .notices
        .iter()
        .all(|notice| notice.severity != "blocker"));

    let temp = tempfile::tempdir().expect("review sanitization tempdir should be created");
    let serialized = serde_json::to_string(review).expect("review should serialize");
    assert!(!serialized.contains(&temp.path().to_string_lossy().to_string()));
    assert!(!serialized.contains("serial"));
    assert!(!serialized.contains("unauthorized"));
}

struct QualificationWorkspace {
    _temp: tempfile::TempDir,
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    host_input_root: PathBuf,
}

impl QualificationWorkspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("qualification tempdir should be created");
        Self {
            runtime_root: temp.path().join("runtime"),
            cache_root: temp.path().join("cache"),
            fake_device_root: temp.path().join("fake-device"),
            host_input_root: temp.path().join("host-input"),
            _temp: temp,
        }
    }
}

fn release_metadata_fixture(contract: &ObtainiumQualificationContract) -> Value {
    let path = repository_root().join(&contract.release.metadata_fixture);
    let bytes = fs::read(path).expect("Obtainium release metadata fixture should be readable");
    serde_json::from_slice(&bytes).expect("Obtainium release metadata fixture should be valid")
}

fn release_asset_url(contract: &ObtainiumQualificationContract, tag: &str) -> String {
    format!(
        "{}/{}/releases/download/{}/{}",
        contract.app_definition.base_url.trim_end_matches('/'),
        contract.app_definition.repository,
        tag,
        contract.release.asset_name
    )
}

fn artifact_cache_path(cache_root: &Path, artifact: &ExecutionArtifact, url: &str) -> PathBuf {
    cache_root.join(artifact_local_filename(&artifact.id, url, &artifact.cache))
}

fn valid_apk_fixture_bytes(package_name: &'static str) -> Vec<u8> {
    let fixture_workspace = tempfile::tempdir().expect("APK fixture root should be created");
    let fixture_apk = crate::apk_manifest::tests::write_valid_test_apk_with_package(
        &fixture_workspace,
        package_name,
    );
    fs::read(fixture_apk).expect("Obtainium APK fixture should be readable")
}

fn seed_artifact_cache(
    cache_root: &Path,
    plan: &ExecutionPlan,
    contract: &ObtainiumQualificationContract,
    selected_package_name: &'static str,
) {
    fs::create_dir_all(cache_root).expect("cache root should be created");
    assert_eq!(plan.artifacts.len(), 1);
    assert_eq!(contract.app_definition.package_name, TARGET_PACKAGE);
    let releases = release_metadata_fixture(contract);
    let release_tags = releases
        .as_array()
        .expect("release metadata fixture should contain a list")
        .iter()
        .filter_map(|release| release.get("tag_name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(release_tags.contains(&contract.release.cached_old_tag.as_str()));
    assert!(release_tags.contains(&contract.release.selected_tag.as_str()));
    let selected_url = release_asset_url(contract, &contract.release.selected_tag);
    let older_url = release_asset_url(contract, &contract.release.cached_old_tag);
    let selected_bytes = valid_apk_fixture_bytes(selected_package_name);
    let older_bytes = valid_apk_fixture_bytes("dev.example.stale.obtainium");
    for artifact in &plan.artifacts {
        assert!(artifact
            .id
            .ends_with(&format!("/{}", contract.artifact.id_suffix)));
        assert_eq!(artifact.type_name, contract.artifact.type_name);
        assert!(matches!(
            &artifact.source,
            ExecutionArtifactSource::RemoteRelease {
                provider,
                service_origin,
                repository,
                include_prereleases,
                asset_pattern,
                invert_asset_pattern: false,
                ..
            }
                if provider == &contract.app_definition.provider
                    && service_origin == &contract.app_definition.base_url
                    && repository == &contract.app_definition.repository
                    && include_prereleases == &contract.app_definition.prerelease
                    && asset_pattern.as_deref() == Some(contract.app_definition.asset_pattern.as_str())
        ));
        assert_eq!(artifact.cache, "default");
        fs::write(
            artifact_cache_path(cache_root, artifact, &selected_url),
            &selected_bytes,
        )
        .expect("selected Obtainium APK fixture should be written");
        fs::write(
            artifact_cache_path(cache_root, artifact, &older_url),
            &older_bytes,
        )
        .expect("older cached Obtainium APK fixture should be written");
    }
}

fn dry_run_adapters(workspace: &QualificationWorkspace) -> ExecutorAdapters {
    ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
    )
}

fn install_status(result: &crate::executor::ExecutionRunResult) -> StepRunStatus {
    result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/install_obtainium"))
        .expect("Obtainium install record should exist")
        .status
        .clone()
}

#[test]
fn obtainium_generated_plan_executes_successfully_without_network_or_adb() {
    let workspace = QualificationWorkspace::new();
    let contract = load_contract();
    let prepared = plan_obtainium();
    let plan = prepared.plan.expect("Obtainium plan should be generated");
    seed_artifact_cache(&workspace.cache_root, &plan, &contract, TARGET_PACKAGE);

    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));
    runner.set_github_release_metadata_fixtures(vec![release_metadata_fixture(&contract)]);
    let result = runner.run(&plan);

    assert!(
        result.success,
        "generated Obtainium plan should execute successfully"
    );
    assert_eq!(result.total_steps, plan.steps.len());
    assert_eq!(result.steps.len(), result.total_steps);
    assert_eq!(result.resolved_releases.len(), 1);
    let release = &result.resolved_releases[0];
    assert_eq!(release.app_id, contract.app_definition.id);
    assert_eq!(release.artifact_id, contract.app_definition.artifact_id);
    assert_eq!(release.release_tag, contract.release.selected_tag);
    assert_eq!(release.asset_name, contract.release.asset_name);
    assert!(result.resolved_artifacts.is_empty());
    let selected_cache = artifact_cache_path(
        &workspace.cache_root,
        &plan.artifacts[0],
        &release_asset_url(&contract, &contract.release.selected_tag),
    );
    let older_cache = artifact_cache_path(
        &workspace.cache_root,
        &plan.artifacts[0],
        &release_asset_url(&contract, &contract.release.cached_old_tag),
    );
    assert_ne!(
        selected_cache, older_cache,
        "release asset URLs use distinct cache keys"
    );
    assert!(selected_cache.is_file());
    assert!(older_cache.is_file());
    assert_ne!(
        sha256_hex(&fs::read(&selected_cache).expect("selected release cache fixture should read")),
        sha256_hex(&fs::read(&older_cache).expect("older cache fixture should read")),
        "the newer selected APK and retained older cached APK must be distinct"
    );
    let serialized_result = serde_json::to_value(&result).expect("result should serialize");
    assert!(serialized_result["resolved_releases"].is_array());
    assert!(!serialized_result
        .to_string()
        .contains("browser_download_url"));
    for record in &result.steps {
        assert!(
            !matches!(
                record.status,
                StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
            ),
            "step {} must not fail: {:?}",
            record.step_id,
            record.status
        );
    }
    for suffix in ["/resolve_artifacts", "/install_obtainium"] {
        let record = result
            .steps
            .iter()
            .find(|record| record.step_id.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing execution record for {suffix}"));
        assert_eq!(record.status, StepRunStatus::Executed);
    }
}

#[test]
fn obtainium_install_skips_on_repeated_deterministic_run() {
    let workspace = QualificationWorkspace::new();
    let contract = load_contract();
    let prepared = plan_obtainium();
    let plan = prepared.plan.expect("Obtainium plan should be generated");
    seed_artifact_cache(&workspace.cache_root, &plan, &contract, TARGET_PACKAGE);
    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));
    let fixture = release_metadata_fixture(&contract);
    runner.set_github_release_metadata_fixtures(vec![fixture.clone(), fixture]);

    let first = runner.run(&plan);
    assert!(first.success, "first deterministic run should succeed");
    assert_eq!(install_status(&first), StepRunStatus::Executed);

    let second = runner.run(&plan);
    assert!(
        second.success,
        "repeated deterministic run should remain successful"
    );
    assert_eq!(
        install_status(&second),
        StepRunStatus::Skipped,
        "the authored package_installed predicate must skip the repeated install"
    );
    for record in &second.steps {
        assert!(
            !matches!(
                record.status,
                StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
            ),
            "repeated run step {} must not fail: {:?}",
            record.step_id,
            record.status
        );
    }
}

#[test]
fn obtainium_missing_release_fixture_fails_without_network_fallback() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_obtainium();
    let plan = prepared.plan.expect("Obtainium plan should be generated");
    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));
    runner.set_github_release_metadata_fixtures(Vec::new());

    let result = runner.run(&plan);

    assert!(!result.success);
    assert_eq!(
        result
            .steps
            .iter()
            .find(|record| record.step_id.ends_with("/resolve_artifacts"))
            .expect("artifact resolution record should exist")
            .status,
        StepRunStatus::Failed
    );
    assert_eq!(install_status(&result), StepRunStatus::Blocked);
}

#[test]
fn obtainium_rejects_wrong_package_before_device_install_mutation() {
    let workspace = QualificationWorkspace::new();
    let contract = load_contract();
    let prepared = plan_obtainium();
    let plan = prepared.plan.expect("Obtainium plan should be generated");
    seed_artifact_cache(
        &workspace.cache_root,
        &plan,
        &contract,
        "dev.example.unexpected.obtainium",
    );

    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));
    runner.set_github_release_metadata_fixtures(vec![release_metadata_fixture(&contract)]);
    let result = runner.run(&plan);

    assert!(!result.success);
    assert_eq!(
        result
            .steps
            .iter()
            .find(|record| record.step_id.ends_with("/resolve_artifacts"))
            .expect("artifact resolution record should exist")
            .status,
        StepRunStatus::Executed
    );
    assert_eq!(install_status(&result), StepRunStatus::Failed);
    assert!(runner
        .adapters()
        .device()
        .commands()
        .iter()
        .all(|command| command.first().map(String::as_str) != Some("install_apk")));
}

#[derive(Debug, Default)]
struct InstallFailureDevice {
    inner: FakeDryRunDevice,
}

impl InstallFailureDevice {
    fn commands(&self) -> &[Vec<String>] {
        self.inner.commands()
    }
}

impl ExecutorDevice for InstallFailureDevice {
    fn uses_fake_device_filesystem(&self) -> bool {
        true
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        let _ = <FakeDryRunDevice as ExecutorDevice>::install_apk(
            &mut self.inner,
            apk_path,
            replace_existing,
        );
        Err(DeviceOperationError::other(
            "deterministic Obtainium install failure",
        ))
    }

    fn record_installed_package(&mut self, package_name: &str) {
        <FakeDryRunDevice as ExecutorDevice>::record_installed_package(
            &mut self.inner,
            package_name,
        );
    }

    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::push(&mut self.inner, source, dest, sync)
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::mkdir_p(&mut self.inner, path)
    }

    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::remove_file(&mut self.inner, path)
    }

    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::remove_tree(&mut self.inner, path)
    }

    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::copy_on_device(
            &mut self.inner,
            source,
            dest,
            recursive,
            privileged,
        )
    }

    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::package_installed(&mut self.inner, package_name)
    }

    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::path_exists(&mut self.inner, path)
    }

    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::path_is_dir(&mut self.inner, path)
    }

    fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::run_plan_command(&mut self.inner, command)
    }

    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::launch_app(&mut self.inner, package_name, activity)
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::force_stop_app(&mut self.inner, package_name)
    }
}

#[test]
fn obtainium_install_failure_stops_the_unchanged_generated_plan_truthfully() {
    let workspace = QualificationWorkspace::new();
    let contract = load_contract();
    let prepared = plan_obtainium();
    let plan = prepared.plan.expect("Obtainium plan should be generated");
    let plan_before = plan.clone();
    seed_artifact_cache(&workspace.cache_root, &plan, &contract, TARGET_PACKAGE);

    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        InstallFailureDevice::default(),
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
        false,
    );
    let mut runner = ExecutorRunner::new(adapters);
    runner.set_github_release_metadata_fixtures(vec![release_metadata_fixture(&contract)]);
    let result = runner.run(&plan);

    assert_eq!(
        plan, plan_before,
        "executor qualification must not mutate the plan"
    );
    assert!(!result.success);
    let resolve = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/resolve_artifacts"))
        .expect("artifact resolution record should exist");
    assert_eq!(resolve.status, StepRunStatus::Executed);
    let install_index = result
        .steps
        .iter()
        .position(|record| record.step_id.ends_with("/install_obtainium"))
        .expect("Obtainium install record should exist");
    let install = &result.steps[install_index];
    assert_eq!(install.status, StepRunStatus::Failed);
    assert!(install
        .message
        .as_deref()
        .is_some_and(|message| !message.is_empty()));
    assert!(result.steps[install_index + 1..]
        .iter()
        .all(|record| record.status != StepRunStatus::Executed));
    assert!(runner
        .adapters()
        .device()
        .commands()
        .iter()
        .any(|command| command.first().map(String::as_str) == Some("install_apk")));
}
