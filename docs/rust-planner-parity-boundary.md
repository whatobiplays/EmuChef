# Rust Planner Parity Boundary

This document defines the current planner ownership boundary for the
Python-to-Rust/Tauri migration. Python planner behavior is not deleted,
deprecated, or ready for cutover. Rust planner coverage remains crate-internal
and fixture-scoped.

Rust planner tests include an intentional fixture inventory/parsing guard for
the existing Phase 6M/6N planner parity evidence. The guard consumes checked-in
files only; it does not invoke Python or regenerate fixtures. The Rust tests
also include focused execution-plan DTO shape and normalization assertions for
the supported fixture-scoped surface, plus authored-corpus coverage for the
checked-in recipe files under `authored/recipes`, `authored/device_plans`, and
`authored/device_profiles`.

## Current Owners And Evidence

Python remains the CLI/reference owner for planner behavior:

- `src/emuchef/cli.py`: current `draft` and `plan` command path, device-context
  resolution, and planner invocation.
- `src/emuchef/planner/service.py`, `draft_builder.py`, `dependencies.py`,
  `emitter.py`, `contracts.py`, `bindings.py`, `conflicts.py`,
  `operations.py`, `profile_matching.py`: planner session, dependency,
  binding, normalization, validation, and emission behavior.
- `src/emuchef/io/loader.py`, `validation.py`, `serde.py`: authored YAML,
  catalog validation, and serialized output support used by planner-facing
  workflows.
- `src/emuchef/domain/execution_plan.py`, `planning_result.py`,
  `param_values.py`, `recipe.py`, `step.py`, `input_declaration.py`,
  `artifacts.py`: Python planner input/output model definitions.
- `src/emuchef/steps/planner_hooks.py`: step-owned planner validation and
  normalization hooks.
- `tests/test_planner_core.py`, `tests/test_cli.py`, `tests/test_validation.py`,
  `tests/test_step_plugins.py`: Python planner/CLI/reference test coverage.

Rust planner-adjacent coverage is internal and fixture-scoped:

- `crates/emuchef-rust-backend/src/planner.rs`: private planner skeleton for
  focused `PlanningResult`/`ExecutionPlan` fixture parity and internal
  permission-intent construction.
- `crates/emuchef-rust-backend/src/planner_device_plan.rs`: private
  checked-in device-plan/profile ingestion for crate-internal planner inputs.
  It parses only the current authored profile/plan YAML surface, maps selected
  recipe refs in authored order, derives profile capability defaults and tags,
  and builds explicitly synthetic planner-only device context.
- `crates/emuchef-rust-backend/src/planner_tests.rs`: Rust unit tests comparing
  Phase 6M/6N planning output to checked-in Python planner goldens, including
  the P7A fixture inventory/parsing guard, focused planner-only param contract
  coverage for selected emitted step types, and P7G execution-plan DTO
  shape/normalization assertions. P7H tests discover the checked-in authored
  recipe corpus, parse each recipe through the Rust domain model, exercise
  manually supplied selected-recipe contexts, assert optional-input pruning and
  bound-input inclusion for RetroArch, classify required-input gaps as
  `binding_missing`, and assert checked-in authored recipe/golden evidence is
  not rewritten. P7I tests discover checked-in repo device profiles and plans,
  assert explicit path/id/profile/selected-recipe inventory, build private
  `PlannerInput` values from repo device plans/profiles, accept supplied test
  bindings, and emit at least one deterministic plan from a checked-in
  profile/plan context without invoking Python, ADB, executor/apply, network,
  or artifact materialization. P7F permission-intent tests serialize the
  internal Rust helper output only for assertions; that serialized helper output
  is not an execution-plan DTO surface.
- `crates/emuchef-rust-backend/tests/phase6m_planner.rs`: protocol guard that
  keeps planner requests unrouted and capabilities editor-scoped.
- `crates/emuchef-rust-backend/tests/fixtures/authored_root/planner_*`;
  `crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6m_planner_*.json`;
  `crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6n_planner_*.json`:
  frozen authored inputs and Python planner evidence consumed by Rust tests.

## Planner Boundary Matrix

| Planner area | Current Python owner | Rust evidence | Current status | Before Python planner deletion |
| --- | --- | --- | --- | --- |
| Authored YAML loading/parsing | `src/emuchef/io/loader.py`; `src/emuchef/io/validation.py` | `crates/emuchef-rust-backend/src/yaml.rs`; `phase6e_yaml.rs`; `planner_tests.rs` loading fixture roots and checked-in authored corpus | Fixture-scoped Rust parsing exists for selected authored roots and the checked-in `authored/recipes` corpus. | Prove all required authored corpus and planner inputs parse with matching semantics. |
| Schema/domain validation | `src/emuchef/io/validation.py`; `src/emuchef/planner/contracts.py`; domain modules under `src/emuchef/domain/` | `validation.rs`; `phase6k_validation.rs`; `phase6l_catalog_validation.rs`; planner golden comparisons | Rust validates selected editor/planner rules only. | Port or retire Python planner-owned diagnostics and validation rules with parity tests. |
| Device plan/profile ingestion | `src/emuchef/io/loader.py`; `src/emuchef/planner/service.py`; `src/emuchef/cli.py`; `profile_matching.py` | `planner_device_plan.rs`; `repo_device_profile_inventory_is_explicit_by_path_and_id`; `repo_device_plan_inventory_is_explicit_by_path_id_profile_and_selected_order`; `repo_device_plan_profile_context_can_plan_successfully_without_python_or_devices` | Rust has private checked-in repo device-plan/profile ingestion coverage. It parses current YAML profiles/plans, requires `selected_by_default` where current Python parsing requires it, maps selected recipe refs in authored order, maps profile `capability_defaults` and `device_tags`, and builds synthetic planner-only context from profile fields. Defaults, config variants, planner override semantics, profile matching, real detected facts, CLI operation replay, and device probing remain out of scope. | Prove or deliberately retire full Python planner plan/profile behavior, including CLI context resolution, profile matching, defaults/overrides, and diagnostics. |
| Input binding/default handling | `src/emuchef/planner/bindings.py`; `draft_builder.py`; `emitter.py` | `required_input_bindings_match_python_success_and_missing_error`; `phase6n_optional_inputs_prune_and_rebind_like_python`; `phase6n_input_defaults_and_multiple_values_match_python` | Covered for selected required, optional, default, and multiple input fixtures. | Cover device plans, overrides, operation replay, validation errors, and broader input declarations. |
| Selected recipe expansion | `src/emuchef/planner/service.py`; `dependencies.py`; `draft_builder.py` | `recipe_and_step_dependencies_match_python_ordering`; `phase6n_dependency_expansion_and_namespacing_match_python` | Covered for focused recipe dependency fixtures. | Prove selected recipe expansion across the required authored catalog and CLI workflows. |
| Dependency validation | `src/emuchef/planner/dependencies.py`; `src/emuchef/io/validation.py` | `phase6l_catalog_validation.rs`; planner dependency tests; `dependency_validation_*` planner tests | Rust validates selected/emitted step dependencies for unknown or non-emitted targets, self-dependencies, and static cycles. Duplicate authored dependencies remain allowed and are preserved in emitted execution steps. Runtime dependency outcomes remain executor behavior. | Match Python planner diagnostics, ordering, and failure behavior for broader planner-facing dependency cases. |
| Artifact group expansion | `src/emuchef/steps/planner_hooks.py`; `src/emuchef/planner/emitter.py` | `refs_artifacts_defaults_and_conditions_match_python_execution_plan` | Covered for focused `resolve_artifacts` and `extract_archive` fixture behavior. | Cover all planner hook cases and invalid artifact/group diagnostics. |
| Duplicate detection | `src/emuchef/io/validation.py`; `src/emuchef/planner/catalog.py` | `phase6l_catalog_validation.rs` duplicate fixture cases | Catalog duplicate diagnostics are fixture-scoped; planner replacement is not complete. | Prove duplicate behavior for all catalog/planner inputs or explicitly retire unsupported shapes. |
| Param normalization and focused contract checks | `src/emuchef/planner/contracts.py`; `src/emuchef/steps/planner_hooks.py`; `emitter.py` | `phase6n_builtin_planner_mappings_match_python_goldens`; `phase6n_step_data_refs_conditions_and_constraints_match_python`; `dto_normalized_params_ref_defaults_and_permission_surface_are_stable`; focused P7E/P7F param contract tests in `planner_tests.rs` | Covered for selected built-in StepSpec defaults and hook outputs. The Rust planner also validates focused emitted-step param contracts for `copy_files`, `extract_artifacts`, `extract_archive`, `install_apk`, `wait`, and `grant_permissions`, including required params, ref/literal mode, selected enum/bool/scalar values, group-only artifact selection, and unknown params only for those focused step types. DTO tests assert normalized artifacts lists, default-injected params, rewritten shorthand refs, semantic list ordering, and internal-only permission surface for the supported fixture slice. | Port full StepSpec planner hooks and normalization behavior for all supported steps. |
| Literal/ref param representation | `src/emuchef/domain/param_values.py`; `src/emuchef/planner/emitter.py` | Phase 6M/6N planner goldens parsed by `planner_parity_fixture_inventory_consumes_checked_in_evidence_only` | Rust emits Python-shaped literal/ref JSON for selected fixtures. | Prove all required literal/runtime/ref values match Python output and plan IO expectations. |
| Shorthand step ref rewriting | `src/emuchef/planner/emitter.py`; `src/emuchef/planner/contracts.py` | `refs_artifacts_defaults_and_conditions_match_python_execution_plan`; `phase6n_step_data_refs_conditions_and_constraints_match_python`; `step_ref_validation_preserves_explicit_refs_and_rewrites_shorthand_refs` | Covered for selected `steps.<id>` and `steps.<id>.outputs.<field>` refs in emitted Rust planner steps. | Cover cross-recipe refs, all planner-supported ref contexts, and broader command paths. |
| Static ref target validation | `src/emuchef/planner/contracts.py`; `src/emuchef/io/validation.py` | `phase6k_validation.rs`; `phase6l_catalog_validation.rs`; selected planner tests | Rust validates selected emitted step refs for malformed `steps.*`, unknown selected step targets, unknown outputs, and shorthand refs to steps without a primary output. Non-step refs remain outside this slice. | Port planner-owned static ref diagnostics and ensure CLI planning paths use Rust equivalents. |
| Permission intent construction | Current permission declarations are step-local authored data under selected `grant_permissions.params.runtime`, `grant_permissions.params.appops`, and `grant_permissions.params.policy`; top-level recipe `permissions:` is invalid, and manual permission declarations are unsupported. | `grant_permissions_stays_step_local_without_permission_plan`; P7F `build_permission_intent` tests; executor permission tests | Rust builds internal structured permission intent from selected `grant_permissions` authored step params. The helper preserves runtime permissions, appops, policy metadata, required flags, and `when` metadata without emitting shell/ADB command strings. Rust planner output still omits serialized `permission_plan`. | Keep step-local permission semantics unless a later approved schema/DTO slice deliberately changes the source or serialized surface. |
| Execution plan DTO/schema emission | `src/emuchef/domain/execution_plan.py`; `src/emuchef/planner/emitter.py`; `src/emuchef/io/serde.py` | `PlanningResult`/`ExecutionPlan` structs in `planner.rs`; Phase 6M/6N golden comparisons; P7G DTO shape tests | Rust emits matching DTOs for selected frozen flows only. P7G asserts exact key sets and values for successful and error planning results without treating arbitrary JSON object key order as a public contract. | Prove all required plan DTO fields, schema handling, output formats, and CLI `--output` behavior. |
| Deterministic output ordering | `src/emuchef/planner/dependencies.py`; `emitter.py`; Python ordered domain models | `recipe_and_step_dependencies_match_python_ordering`; `phase6n_dependency_expansion_and_namespacing_match_python`; `dto_success_result_shape_is_stable_for_supported_fixture_surface`; P7A inventory guard | Fixture-scoped ordering parity exists for selected dependencies, inputs, artifacts, and steps. List ordering is semantic in the DTO tests; object key ordering is not broadened into a contract unless the Rust surface intentionally models ordering. | Prove deterministic ordering for all required catalog/plan combinations. |
| Diagnostics/error model | `src/emuchef/domain/errors.py`; `src/emuchef/planner/*`; `src/emuchef/io/validation.py` | Required binding missing, dependency cycle, catalog validation tests, focused P7E param contract diagnostics, P7G DTO error-result tests | Rust covers selected error/status shapes. Focused Rust planner param contract diagnostics include recipe, step, step type, param, expected value/mode, and actual value context. P7G asserts deterministic error-result shape for the current focused step-param slice and deterministic ordering only for an existing multi-error unknown-param path; it does not broaden planner error accumulation. | Port or retire all planner-facing diagnostics, details fields, CLI output, and exit-code expectations. |
| Corpus/golden coverage | `tests/test_planner_core.py`; `tests/test_cli.py`; dev-only/reference-only golden generation in `crates/emuchef-rust-backend/README.md` | `planner_tests.rs`; `phase6t_authored_corpus.rs`; `phase6m_planner.rs`; `phase6m_planner_*.json`; `phase6n_planner_*.json` | P7A guards the current frozen inventory and parses every planner golden. P7H guards the checked-in authored recipe inventory and exercises the internal Rust planner against manually supplied corpus contexts. | Broaden corpus coverage or reclassify historical tests before deleting Python planner code. |

## Authored-Corpus Support Matrix

P7H authored-corpus planning uses `PlannerInput::from_authored_root("authored")`
with manually supplied selected recipe refs, fixture device context, and fixture
runtime capabilities. P7I adds private
`PlannerInput::from_authored_device_plan(...)` coverage for the checked-in repo
`device_plans` and `device_profiles`: selected recipes come from
`recipes[].recipe_ref` entries whose required `selected_by_default` value is
true, in authored order; runtime capabilities come from profile
`capability_defaults`; tags come from profile `device_tags`; synthetic
planner-only context uses the first `match.manufacturer_contains` value or
`profile:<profile_id>`, profile `name` or `profile:<profile_id>`, profile
`match.android_version.min` or `0`, and `android_api_level: null`.

This coverage does not resolve remote URLs, download or materialize artifacts,
invoke ADB, call executor/apply paths, run profile matching against detected
facts, apply device-plan defaults, apply config variants, or implement
device-plan override binding semantics.

| Path | Recipe id | Parse status | Planner/emission status | Context source | Unsupported gap | Test/evidence path |
| --- | --- | --- | --- | --- | --- | --- |
| `authored/recipes/app.obtainium.install.yaml` | `app.obtainium.install` | Success through Rust domain model. | Success for synthetic selected-recipe context. | Manual selected ref plus synthetic fixture device context/capabilities. | Repo device plans do not currently select this recipe; remote APK URL remains declarative planner data only. | `planner_tests::authored_corpus_recipes_parse_through_rust_domain_model`; `planner_tests::authored_corpus_supported_synthetic_context_emits_execution_plan` |
| `authored/recipes/app.retroarch.provision.yaml` | `app.retroarch.provision` | Success through Rust domain model. | Success with optional `retroarch_cfg` omitted; `seed_retroarch_cfg` is pruned. Success with placeholder `retroarch_cfg` binding; `seed_retroarch_cfg` is included. At least one checked-in repo device plan/profile context emits successfully through private P7I ingestion with a supplied placeholder `retroarch_cfg` binding. | Manual selected ref plus synthetic fixture context in P7H; checked-in repo device plan/profile ingestion in P7I with synthetic planner-only context derived from profile fields. | Placeholder binding is planner-only; remote artifact URLs remain declarative planner data only. | `planner_tests::authored_corpus_recipes_parse_through_rust_domain_model`; `planner_tests::authored_corpus_retroarch_optional_cfg_omitted_and_bound_are_deterministic`; `planner_tests::repo_device_plan_profile_context_can_plan_successfully_without_python_or_devices` |
| `authored/recipes/app.xaniteog.install.yaml` | `app.xaniteog.install` | Success through Rust domain model. | Unbound synthetic context emits deterministic `binding_missing` for `app.xaniteog.install/xaniteog_apk`; placeholder `.apk` binding emits successfully in the bound synthetic corpus context. P7I verifies supplied binding ingestion for a checked-in device plan that selects this recipe, but full emitted success for that plan is not broadened because the current selected profile lacks app-data capability required by the selected RetroArch flow. | Manual selected ref plus synthetic fixture context in P7H; checked-in repo device plan/profile selected-ref ingestion in P7I. | Placeholder binding is planner-only and does not validate APK payloads; broader current-plan success depends on profile capability coverage. | `planner_tests::authored_corpus_unbound_required_inputs_emit_classified_errors`; `planner_tests::authored_corpus_supported_synthetic_context_emits_execution_plan`; `planner_tests::repo_device_plan_ingestion_accepts_supplied_bindings_without_applying_overrides` |
| `authored/recipes/feature.copy_bios.yaml` | `feature.copy_bios` | Success through Rust domain model. | Unbound synthetic context emits deterministic `binding_missing` for `feature.copy_bios/bios_source_dir`; placeholder directory binding emits successfully in the bound synthetic corpus context. P7I verifies supplied binding ingestion for checked-in device plans that select this recipe, but full emitted success for those plans is not broadened when the selected profile lacks app-data capability required by the selected RetroArch flow. | Manual selected ref plus synthetic fixture context in P7H; checked-in repo device plan/profile selected-ref ingestion in P7I. | Placeholder binding is planner-only and does not inspect BIOS contents; broader current-plan success depends on profile capability coverage. | `planner_tests::authored_corpus_unbound_required_inputs_emit_classified_errors`; `planner_tests::authored_corpus_supported_synthetic_context_emits_execution_plan`; `planner_tests::repo_device_plan_ingestion_accepts_supplied_bindings_without_applying_overrides` |

## Python Planner Paths Required Today

The Python planner remains required for current user-facing CLI/reference
behavior. The `emuchef` console script still routes through `src/emuchef/cli.py`
and the Python planner modules for `draft` and `plan`. Python planner tests
remain active reference coverage. Dev-only/reference-only regeneration commands
for Phase 6M/6N planner goldens remain documented in
`crates/emuchef-rust-backend/README.md` and classified in
`docs/python-fixture-golden-ownership.md`.

The Rust planner skeleton is not a replacement command path. It is not exposed
as a Tauri command, sidecar protocol request, production CLI command, backend
selector, or runtime fallback. Its internal permission-intent helper is not a
serialized execution-plan field and is not consumed by executor/apply behavior.
P7G DTO coverage does not change those boundaries.

## Deletion-Readiness Ladder

Python planner deletion requires, at minimum:

1. Rust parses all required authored corpus and frozen planner goldens.
2. Rust validates planner-owned rules with parity coverage.
3. Rust emits execution-plan DTOs matching frozen goldens for supported flows.
4. A Rust CLI or replacement command path can produce equivalent plans.
5. Python planner no longer owns any user-facing CLI/reference path.
6. Python planner tests are ported, retired, or explicitly reclassified as
   historical reference.

The current Rust planner coverage does not complete this ladder. The current
boundary docs, frozen planner evidence guard, selected emitted step-output ref
and dependency validation coverage, and focused emitted-step param contract
coverage, including P7F internal permission-intent construction coverage and
P7G DTO shape/normalization coverage plus P7I private repo device-plan/profile
ingestion coverage, are incremental parity evidence only.
