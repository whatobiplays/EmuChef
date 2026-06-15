# Rust Planner Parity Boundary

This document defines the current planner ownership boundary for the
Python-to-Rust/Tauri migration. Python planner behavior is not deleted,
deprecated, or ready for cutover. Rust planner coverage remains crate-internal
and fixture-scoped, with a dev-only shadow Cargo command for manual emission of
the current private Rust `PlanningResult`.

Planner cutover readiness and Python planner deletion blockers are classified
in `docs/rust-planner-cutover-readiness.md`. This boundary document records
current parity evidence; the readiness document is the source for the
user-facing routing and deletion checklist. The future Rust planner CLI output
compatibility decision is recorded in
`docs/adr/0002-rust-planner-cli-output-compatibility.md`. The future default
Rust planner real-device context ownership decision is recorded in
`docs/adr/0003-rust-real-device-context-ownership.md`: Rust should own
real-device probing and detected-device profile mismatch warning parity before a
default Rust planner cutover.

Rust planner tests include an intentional fixture inventory/parsing guard for
the existing Phase 6M/6N planner parity evidence. The guard consumes checked-in
files only; it does not invoke Python or regenerate fixtures. The Rust tests
also include focused execution-plan DTO shape and normalization assertions for
the supported fixture-scoped surface, plus authored-corpus coverage for the
checked-in recipe files under `authored/recipes`, `authored/device_plans`, and
`authored/device_profiles`, plus internal repo-plan composition evidence for
the checked-in device-plan/profile contexts that currently succeed through the
private Rust planner. The dev-only P7P scenario matrix records the current
Python planner API versus Rust shadow comparison status for the checked-in
device plans without changing planner ownership.

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
- `crates/emuchef-rust-backend/src/plan_shadow.rs` and
  `crates/emuchef-rust-backend/src/bin/emuchef-plan-shadow.rs`: dev-only shadow
  planner command support for manual migration inspection. The command emits
  pretty JSON `PlanningResult` values from explicit authored-root/device-plan
  inputs. P8J adds explicit device context flags to this command for supplied
  manufacturer, model, Android version, and ordered device tags. P8R adds a
  local `--detected-facts-json <path>` fixture mode to the shadow binary only;
  the Python CLI bridge does not forward that option. P8A exposes the shadow
  binary through an explicit developer-only Python CLI bridge, but it is not the
  default planner CLI or a cutover path.
- `tools/smoke_rust_detected_facts_fixture.py`: dev-only P8S smoke for the P8R
  Rust shadow-binary detected-facts fixture harness. The smoke creates temporary
  fixture JSON files, invokes the supplied `emuchef-plan-shadow` binary directly
  with `--detected-facts-json`, and reports deterministic matching,
  mismatching, and explicit-context override case evidence. It is optional/manual
  evidence only and does not route through Python `emuchef plan`, the matrix
  smoke runner, the Python-vs-Rust comparison harness, normal runtime checks, or
  the readiness gate.
- `tools/compare_rust_python_plan.py`: dev-only comparison harness for Python
  planner API output versus Rust shadow planner output. The harness emits a
  deterministic JSON classification report or matrix report and is not part of
  normal Rust/Tauri runtime checks. P8K matrix scenarios may include optional
  explicit `device_context` values; the harness forwards supplied values to both
  the Python planner worker and Rust shadow command while reporting only stable
  context presence/key metadata.
- `tools/smoke_rust_shadow_cli_matrix.py`: dev-only P8B smoke runner for the
  explicit Python CLI Rust planner migration routes across the current scenario
  matrix. The smoke requires a supplied shadow binary, invokes the Python CLI
  route, and reports route exit-code/classification evidence only. It does not
  run the Python-vs-Rust planner-output comparison harness. P8F extends the same
  smoke runner with explicit `--rust-shadow-output python-compatible` coverage
  for `rust-shadow`. P8H extends it to `--planner-backend rust-experimental`,
  where generated commands omit `--rust-shadow-output` and successful scenarios
  must exit `0` and emit concise Python-compatible summary stdout. P8K forwards
  optional scenario `device_context` fields through generated route commands
  using the existing P8J flags.
- `tools/check_rust_planner_cutover_readiness.py`: dev-only P8I static readiness
  gate for future default Rust planner proposals. It checks static prerequisites,
  durable readiness-document references, stable CLI backend tokens, required
  artifact presence, and checked-in device-plan coverage from
  `authored/device_plans/*.yaml` / `*.yml` filenames. It emits deterministic JSON
  validates optional `device_context` shape separately from meaningful
  explicit-context coverage, and lists manual evidence commands without executing
  comparison, smoke, Cargo, npm, ADB, Tauri/protocol, executor/apply, network,
  artifact, or golden regeneration work.
- `tests/test_cli.py`: P8C and P8E CLI output compatibility contract coverage for
  the explicit `rust-shadow` bridge. The default guarded contract is Rust
  stdout/stderr/exit-code passthrough. The explicit
  `--rust-shadow-output python-compatible` mode formats usable Rust
  `PlanningResult` JSON with Python-compatible visible summary labels and
  structured YAML over the Rust JSON mapping. P8G coverage keeps Python as the
  default planner, rejects `--rust-shadow-output` for non-`rust-shadow` backends,
  and verifies that `rust-experimental` reuses Rust shadow command construction
  with Python-compatible output by default.
- `docs/adr/0002-rust-planner-cli-output-compatibility.md`: P8D decision record
  for future default Rust planner routing. The accepted target is compatibility
  with the current Python `emuchef plan` output and exit-code behavior unless a
  separate accepted breaking-change decision says otherwise.
- `docs/adr/0003-rust-real-device-context-ownership.md`: P8M decision record for
  future default Rust planner routing. The accepted future-cutover ownership
  model is Rust-owned real-device probing and detected-device profile mismatch
  warning parity. The ADR does not implement probing, deprecate Python probing,
  change default CLI behavior, or promote Rust routes beyond explicit
  non-default migration paths.
- `crates/emuchef-rust-backend/src/device_probe.rs`: P8N crate-local
  fake/test-only foundation for future Rust-owned probing. It defines detected
  facts, stable probe errors, a probe trait, a fake probe, and context layering
  helper. It is not wired into CLI routes, Python, Tauri/protocol,
  executor/apply, readiness behavior, or live ADB probing.
- `crates/emuchef-rust-backend/src/device_profile_match.rs`: P8P crate-local
  pure/test-backed foundation for future detected-device profile mismatch
  warnings. It compares supplied detected facts with authored profile match
  criteria and returns the existing Rust `PlannerMessage` shape. It is not wired
  into planner routes, CLI behavior, Python, Tauri/protocol, executor/apply,
  readiness behavior, or live ADB probing.
- `tools/plan_parity_scenarios.json`: dev-only P7P comparison scenario
  manifest for current checked-in device-plan comparisons. It is not a Python
  golden, regenerated evidence, normal Rust/Tauri check input, or user-facing
  planner behavior.
- `crates/emuchef-rust-backend/src/planner_device_plan.rs`: private
  checked-in device-plan/profile ingestion for crate-internal planner inputs.
  It parses only the current authored profile/plan YAML surface, maps selected
  recipe refs in authored order, derives profile capability defaults and tags,
  builds explicitly synthetic planner-only device context, and provides the P8Q
  fake/test-backed `PlanningResult` composition path for supplied detected
  facts and detected-device profile mismatch warnings. P8R invokes that helper
  only from a local shadow-binary fixture path. That helper and fixture harness
  are not Python route wiring, live probing, normal planner warning emission, or
  readiness gate reclassification.
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
  or artifact materialization. P8J tests cover explicit device context overrides
  for the shadow command while preserving profile-derived tags when no explicit
  tags are supplied. P8Q tests cover fake/test-backed planner-result
  composition with detected facts and mismatch warnings while preserving stable
  non-context planner output fields. P7J tests classify checked-in device-plan
  `defaults.show_advanced_steps` and `overrides.config_variants` as inactive
  metadata, and use temporary authored roots to prove strict private
  `<recipe_ref>/<input_id>` override binding merge behavior. P7F
  permission-intent tests serialize the
  internal Rust helper output only for assertions; that serialized helper output
  is not an execution-plan DTO surface. P7K tests pin private Rust selected
  recipe expansion ordering, unknown selected/dependency ref diagnostics,
  dependency-cycle error shape, and current checked-in corpus evidence that
  selected refs and expanded refs are identical only because current authored
  recipes have no non-empty `recipe_dependencies` metadata. P7L tests run
  private Rust repo-plan composition from checked-in device-plan/profile YAML
  through `PlannerInput::from_authored_device_plan(...)` and `plan_execution(...)`
  for checked-in contexts, assert deterministic selected/expanded refs and
  execution step order, assert normalized params from prior P7 slices, and keep
  no-`app_data_write` pruning private to planner selection without broadening
  CLI, Tauri, executor/apply, ADB, network, artifact, or Python behavior. P7P
  tests cover the dev-only scenario matrix parsing and deterministic matrix
  report aggregation.
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
| Device plan/profile ingestion | `src/emuchef/io/loader.py`; `src/emuchef/planner/service.py`; `src/emuchef/cli.py`; `profile_matching.py` | `planner_device_plan.rs`; `repo_device_profile_inventory_is_explicit_by_path_and_id`; `repo_device_plan_inventory_is_explicit_by_path_id_profile_and_selected_order`; `repo_device_plan_profile_context_can_plan_successfully_without_python_or_devices`; P7J defaults/override classification tests; P7L/P7O `repo_plan_e2e_*` tests; P7N comparison harness; P7P scenario matrix | Rust has private checked-in repo device-plan/profile ingestion coverage. It parses current YAML profiles/plans, requires `selected_by_default` where current Python parsing requires it, maps selected recipe refs in authored order, maps profile `capability_defaults` and `device_tags`, and builds synthetic planner-only context from profile fields. Private repo-plan success covers `ayaneo.konkr_pocket_fit.base`, `ayaneo.pocket_s_mini.base`, `ayaneo.generic.base`, `ayaneo.pocket_air_mini.base`, and `ayaneo.pocket_s2.base` when required planner-only input bindings are supplied. P7P records all five checked-in device-plan comparisons as current `match` scenarios under supplied planner-only bindings and the shared synthetic/profile-derived planner context. No-`app_data_write` profiles prune RetroArch app-data copy steps and `launch_retroarch` during selection instead of emitting `unknown_step_dependency`. P7J additionally classifies checked-in `defaults.show_advanced_steps` and `overrides.config_variants` as inactive metadata, and supports strict private `<recipe_ref>/<input_id>` override binding merge in temporary authored-root tests. Config variant selection, plan defaults as bindings, broader Python override key forms, profile matching, real detected facts, CLI operation replay, and device probing remain out of scope. | Prove or deliberately retire full Python planner plan/profile behavior, including CLI context resolution, profile matching, broader defaults/overrides, and diagnostics. |
| Input binding/default handling | `src/emuchef/planner/bindings.py`; `draft_builder.py`; `emitter.py` | `required_input_bindings_match_python_success_and_missing_error`; `phase6n_optional_inputs_prune_and_rebind_like_python`; `phase6n_input_defaults_and_multiple_values_match_python`; `temp_device_plan_override_bindings_merge_before_explicit_bindings` | Covered for selected required, optional, default, and multiple input fixtures. P7J covers private device-plan override binding merge for strict `<recipe_ref>/<input_id>` keys with explicit test bindings taking precedence. | Cover operation replay, validation errors, broader Python override key forms such as `inputs.<id>`, and broader input declarations before planner cutover. |
| Selected recipe expansion | `src/emuchef/planner/service.py`; `dependencies.py`; `draft_builder.py` | `recipe_and_step_dependencies_match_python_ordering`; `phase6n_dependency_expansion_and_namespacing_match_python`; P7K `recipe_expansion_*` tests | Private Rust planner expansion follows direct `recipe_dependencies` evidence: dependencies before dependents, sibling dependencies in authored order, selected recipe closure expansion in selected order, and duplicate suppression without moving the first occurrence. Current checked-in recipes have no non-empty dependency metadata, so current corpus selected and expanded refs match as current-state evidence only. | Prove selected recipe expansion across the required authored catalog and CLI workflows, including any future non-empty checked-in dependencies. |
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
facts, apply device-plan defaults, or apply config variants. P7J adds private
strict `<recipe_ref>/<input_id>` device-plan override binding merge behavior,
covered with temporary authored roots because current checked-in device plans
do not contain ref-shaped override binding keys.

P7L adds internal repo-plan composition evidence for checked-in
device-plan/profile contexts that currently succeed through the private Rust
planner. The successful contexts are planner-only results, not CLI, Tauri,
executor, apply, artifact-materialization, or real-device readiness evidence.
They use synthetic profile-derived planner context and do not invoke Python,
ADB, network access, executor code, or artifact downloads.

P7P adds `tools/plan_parity_scenarios.json` and matrix mode for
`tools/compare_rust_python_plan.py`. In this matrix, `match` means only that
the dev-only comparison harness found no unclassified differences for the
compared fields under the supplied planner-only bindings and shared planner
context. It does not mean Python CLI parity, real-device parity,
executor/apply parity, artifact/network/materialization parity, full schema
parity, future scenario parity, or Rust planner cutover readiness by itself.
Matrix mode exits `0` only when every scenario actual classification matches
its expected classification. This is an expectation check for the current
dev-only scenario matrix, not a full planner-correctness claim. The matrix may
require Python dependencies and Rust build artifacts. It does not execute
plans, probe devices, invoke ADB, access the network, materialize artifacts,
regenerate goldens, or participate in normal Rust/Tauri runtime checks.

P8B adds `tools/smoke_rust_shadow_cli_matrix.py` for a separate dev-only smoke
of the explicit Python CLI bridge:
`emuchef plan --planner-backend rust-shadow --rust-planner-bin <path>`. The
smoke loads the same current scenario matrix, creates planner-only placeholder
binding resources, invokes the Python CLI route for each scenario, and emits a
deterministic JSON report with stable stdout/stderr classifications and route
exit-code pass/fail status. This proves that the explicit bridge can invoke the
Rust shadow planner across the current matrix only. P7P remains the
Python-vs-Rust planner-output comparison evidence. The smoke is not a normal
Rust/Tauri runtime check and does not execute/apply plans, probe devices, invoke
ADB, access the network, materialize artifacts, regenerate goldens, or make the
Rust planner authoritative.

P8C defines the default CLI output compatibility contract for that explicit
bridge. The default Python planner route remains the CLI/reference owner for
planning output and exit-code behavior. Omitted `--rust-shadow-output` and
explicit `--rust-shadow-output passthrough` keep `rust-shadow` as a
developer-only JSON passthrough route: Rust stdout is written to Python stdout,
Rust stderr is written to Python stderr, and the Rust process exit code is
returned. That passthrough mode does not translate Rust JSON `PlanningResult`
output into Python YAML, Python concise planning summary text, or Python planner
structures. `--output` and `--verbose` are rejected unless the explicit
`--rust-shadow-output python-compatible` formatter mode is selected. P8J makes
explicit `--manufacturer`, `--model`, `--android-version`, and repeated
`--device-tag` values supported for Rust routes and forwards only supplied
values to the shadow command. `--adb`, `--serial`, `--ops`, and `--debug` remain
unsupported before invoking the Rust process. This contract is not default Rust
planner cutover readiness.

P8D records the future default-route compatibility target in
`docs/adr/0002-rust-planner-cli-output-compatibility.md`. When Rust eventually
becomes the default planner backend for `emuchef plan`, default CLI output and
exit-code behavior must remain compatible with the current Python-owned
`emuchef plan` contract unless a separate accepted breaking-change decision says
otherwise. Python concise summaries, Python `--verbose` structured YAML, and
Python `--output` YAML file behavior remain the compatibility targets. A
Rust-native JSON mode requires a future explicit structured-output option such
as `--format json`; it is not introduced by the current shadow route.

P8E adds an explicit formatter mode to the existing dev-only bridge:
`--rust-shadow-output python-compatible`. Omitted `--rust-shadow-output` and
`--rust-shadow-output passthrough` keep P8C passthrough unchanged. The
compatibility mode parses usable Rust `PlanningResult` JSON even when the Rust
process exits non-zero, preserves that non-zero exit code, and formats the
result through Python-compatible CLI summary labels or structured YAML. The YAML
surface is the Rust JSON mapping emitted through `dump_yaml(...)`, not a rebuilt
Python planner domain object. This mode is not default planner routing and does
not change executor/apply, real-device, ADB, artifact, network, Tauri, protocol,
Cargo fallback, fixture/golden, or Python planner deletion behavior.

P8F adds matrix-level smoke evidence for that explicit compatibility mode through
`tools/smoke_rust_shadow_cli_matrix.py --rust-shadow-output python-compatible`.
It uses the same current scenario matrix and Python CLI `rust-shadow` route as
P8B, but successful scenarios must classify stdout as `python_summary`. Raw Rust
JSON remains `stdout_json` and fails the compatibility-mode smoke for successful
scenarios. YAML-like output may classify as `python_yaml`, but it is not the
default P8F success expectation unless a future scenario explicitly defines that
expectation. P8F is route plus output-format smoke only; P7P remains the
Python-vs-Rust planner DTO/result comparison evidence, and P8B remains raw
passthrough route-invocation evidence.

P8G adds `emuchef plan --planner-backend rust-experimental --rust-planner-bin
<path>` as an explicit non-default migration route. It reuses the same supplied
Rust shadow planner binary invocation as `rust-shadow`, defaults to
Python-compatible CLI formatting, and supports `--verbose` and `--output` through
that formatter path. `--rust-shadow-output` is only valid with
`--planner-backend rust-shadow`; Python and `rust-experimental` reject it before
ADB resolution or Python planner/session construction. `rust-experimental` is a
cutover rehearsal route, not the default planner, not a stable final public
contract, and not Python planner deletion. Its name and behavior may change
before Rust becomes the default planner backend. P8G does not change
executor/apply, real-device, ADB, artifact, network, Tauri, protocol, Cargo
fallback, fixture/golden, or normal runtime-check behavior.

P8H adds matrix-level smoke evidence for that explicit non-default route through
`tools/smoke_rust_shadow_cli_matrix.py --planner-backend rust-experimental`.
The smoke reuses the existing backend and effective-output-mode logic, omits
`--rust-shadow-output` from generated commands, reports the effective
Python-compatible output mode, and requires successful scenarios to exit `0` and
classify stdout as `python_summary`. Raw Rust JSON remains `stdout_json` and
fails the P8H smoke for successful scenarios. P8H is route plus output-shape
smoke only; P7P remains the Python-vs-Rust planner DTO/result comparison
evidence, P8B remains raw passthrough route-invocation evidence, and P8F remains
explicit `rust-shadow` Python-compatible output-mode smoke.

P8I adds `tools/check_rust_planner_cutover_readiness.py` as a static,
developer-only gate for future default Rust planner proposals. The gate checks
that the current scenario matrix is structurally valid, every checked-in device
plan filename has a matrix scenario, required docs/tools/tests exist, the
readiness document contains durable references to the current evidence and
blockers, and the Python CLI still exposes the expected backend tokens. The
report's manual evidence commands are advisory and are not executed by the gate.
The top-level report status remains `blocked` even when static checks pass
because Python remains the default planner owner, `rust-experimental` remains an
explicit non-default rehearsal route, and executor/apply, real-device probing,
Tauri/protocol behavior, Python planner deletion, and normal runtime checks are
unchanged.

P8J adds explicit device context support to the explicit Rust routes only.
`rust-shadow`, `rust-experimental`, and `emuchef-plan-shadow` accept supplied
`--manufacturer`, `--model`, `--android-version`, and repeated `--device-tag`
values. Supplied scalar values override the synthetic/profile-derived Rust
planner context, and supplied tags replace profile-derived tags in order. When no
explicit tags are supplied, profile-derived tags remain unchanged. P8J does not
probe devices, invoke ADB, create detected-device facts, emit detected-device
profile mismatch warnings, change the default Python planner route, or change
executor/apply/Tauri/protocol behavior.

P8K adds explicit-context evidence to the dev-only matrix tooling. Optional
`device_context` objects in `tools/plan_parity_scenarios.json` are validated with
only `manufacturer`, `model`, `android_version`, and non-empty ordered
`device_tags`; detected-device, `adb`, `serial`, and probing fields are rejected.
Omitted context preserves the existing synthetic/profile-derived behavior.
Supplied scalar fields override synthetic/profile-derived fields, and supplied
tags replace profile tags in order. Empty `device_tags` lists are rejected
because the existing P8J flag surface cannot distinguish an explicit empty tag
override from omitted tag flags. The comparison and smoke reports include stable
context presence/key metadata only. This remains dev-only evidence and does not
make Rust default or authoritative.

P8L reclassifies readiness around explicit device context without changing
planner or CLI behavior. The static readiness gate treats `device_context` schema
validity and explicit-context readiness coverage as separate checks:
`device_context: {}` remains schema-valid, but at least one valid scenario with a
meaningful explicit field is required for explicit-context coverage. The former
broad real-device context blocker is narrowed into two still-blocked default
cutover blockers: real-device probing and detected-device profile mismatch
warning parity.

P8M records the future ownership decision for those two blocked areas without
implementing them. For future default Rust planner cutover, Rust should own
real-device probing and detected-device profile mismatch warning parity. Python
remains the current CLI/reference owner, explicit Rust routes remain non-default,
and explicit context evidence remains separate from detected facts and
real-device probing. P8P adds pure/test-backed mismatch-warning helper logic
only; the default-cutover blockers remain until live probing and route evidence
exist.

| Device plan | Device profile | Selected recipes | Private Rust planner status | Required planner-only bindings | Current limitation |
| --- | --- | --- | --- | --- | --- |
| `authored/device_plans/ayaneo.konkr_pocket_fit.base.yaml` | `ayaneo.konkr_pocket_fit` | `app.retroarch.provision` | Success. `repo_plan_e2e_*` tests build input from checked-in plan/profile YAML, emit deterministic selected/expanded refs and step order, and assert normalized RetroArch params. | None. Optional `app.retroarch.provision/retroarch_cfg` may be omitted, which prunes `seed_retroarch_cfg`; a temp `.cfg` binding includes that step. | Not a user-facing planner path. |
| `authored/device_plans/ayaneo.pocket_s_mini.base.yaml` | `ayaneo.pocket_s_mini` | `app.retroarch.provision` | Success. `repo_plan_e2e_*` tests use this app-data-capable context for normalized artifact selection, default params, dependency ordering, shorthand step-ref rewriting, and optional config pruning/binding assertions. | None. Optional `app.retroarch.provision/retroarch_cfg` may be omitted or supplied as a planner-only temp `.cfg` path. | Not a user-facing planner path. |
| `authored/device_plans/ayaneo.generic.base.yaml` | `ayaneo.generic` | `app.retroarch.provision`, `feature.copy_bios` | Success. Required BIOS binding supplied in tests; no-`app_data_write` capability prunes RetroArch app-data copy steps and `launch_retroarch`. | `feature.copy_bios/bios_source_dir` is required by the selected BIOS recipe and is supplied with a planner-only temp directory in tests. | Not a user-facing planner path. Planner-only BIOS binding does not inspect BIOS contents, and executor/apply behavior is unchanged. |
| `authored/device_plans/ayaneo.pocket_air_mini.base.yaml` | `ayaneo.pocket_air_mini` | `app.retroarch.provision`, `feature.copy_bios` | Success. Required BIOS binding supplied in tests; no-`app_data_write` capability prunes RetroArch app-data copy steps and `launch_retroarch`. | `feature.copy_bios/bios_source_dir` is required by the selected BIOS recipe and is supplied with a planner-only temp directory in tests. | Not a user-facing planner path. Planner-only BIOS binding does not inspect BIOS contents, and executor/apply behavior is unchanged. |
| `authored/device_plans/ayaneo.pocket_s2.base.yaml` | `ayaneo.pocket_s2` | `app.retroarch.provision`, `feature.copy_bios`, `app.xaniteog.install` | Success. Required BIOS and XaniteOG bindings supplied in tests; no-`app_data_write` capability prunes RetroArch app-data copy steps and `launch_retroarch`. | `feature.copy_bios/bios_source_dir` and `app.xaniteog.install/xaniteog_apk` are required by selected recipes and are supplied with planner-only temp paths in tests. | Not a user-facing planner path. Planner-only BIOS/APK bindings do not inspect payload contents, and executor/apply behavior is unchanged. |

| P7P scenario id | Device plan | Required matrix bindings | Expected current comparison classification |
| --- | --- | --- | --- |
| `ayaneo_konkr_pocket_fit_base` | `ayaneo.konkr_pocket_fit.base` | None. | `match` |
| `ayaneo_pocket_s_mini_base` | `ayaneo.pocket_s_mini.base` | None. | `match` |
| `ayaneo_generic_base` | `ayaneo.generic.base` | `feature.copy_bios/bios_source_dir` directory. | `match` |
| `ayaneo_pocket_air_mini_base` | `ayaneo.pocket_air_mini.base` | `feature.copy_bios/bios_source_dir` directory. | `match` |
| `ayaneo_pocket_s2_base` | `ayaneo.pocket_s2.base` | `feature.copy_bios/bios_source_dir` directory; `app.xaniteog.install/xaniteog_apk` `.apk` file. | `match` |
| `ayaneo_pocket_s_mini_base_explicit_context` | `ayaneo.pocket_s_mini.base` | None; supplies explicit manufacturer, model, Android version, and ordered device tags. | `match` |

| Path | Recipe id | Parse status | Planner/emission status | Context source | Unsupported gap | Test/evidence path |
| --- | --- | --- | --- | --- | --- | --- |
| `authored/recipes/app.obtainium.install.yaml` | `app.obtainium.install` | Success through Rust domain model. | Success for synthetic selected-recipe context. | Manual selected ref plus synthetic fixture device context/capabilities. | Repo device plans do not currently select this recipe; remote APK URL remains declarative planner data only. | `planner_tests::authored_corpus_recipes_parse_through_rust_domain_model`; `planner_tests::authored_corpus_supported_synthetic_context_emits_execution_plan` |
| `authored/recipes/app.retroarch.provision.yaml` | `app.retroarch.provision` | Success through Rust domain model. | Success with optional `retroarch_cfg` omitted; `seed_retroarch_cfg` is pruned. Success with planner-only temp `.cfg` binding; `seed_retroarch_cfg` is included. P7L proves private repo-plan success for the checked-in `ayaneo.konkr_pocket_fit.base` and `ayaneo.pocket_s_mini.base` contexts. | Manual selected ref plus synthetic fixture context in P7H; checked-in repo device plan/profile ingestion in P7I/P7L with synthetic planner-only context derived from profile fields. | Planner-only config binding does not inspect file contents; remote artifact URLs remain declarative planner data only. | `planner_tests::authored_corpus_recipes_parse_through_rust_domain_model`; `planner_tests::authored_corpus_retroarch_optional_cfg_omitted_and_bound_are_deterministic`; `planner_tests::repo_plan_e2e_from_checked_in_device_plan_succeeds`; `planner_tests::repo_plan_e2e_normalized_steps_reflect_prior_p7_slices` |
| `authored/recipes/app.xaniteog.install.yaml` | `app.xaniteog.install` | Success through Rust domain model. | Unbound synthetic context emits deterministic `binding_missing` for `app.xaniteog.install/xaniteog_apk`; placeholder `.apk` binding emits successfully in the bound synthetic corpus context and in the checked-in `ayaneo.pocket_s2.base` repo-plan context. | Manual selected ref plus synthetic fixture context in P7H; checked-in repo device-plan/profile selected-ref ingestion in P7I/P7J/P7O. | Planner-only APK binding does not validate APK payloads; executor/apply behavior is unchanged. | `planner_tests::authored_corpus_unbound_required_inputs_emit_classified_errors`; `planner_tests::authored_corpus_supported_synthetic_context_emits_execution_plan`; `planner_tests::repo_device_plan_ingestion_accepts_supplied_bindings_without_applying_metadata_overrides`; `planner_tests::repo_plan_e2e_no_app_data_write_contexts_prune_app_data_dependents` |
| `authored/recipes/feature.copy_bios.yaml` | `feature.copy_bios` | Success through Rust domain model. | Unbound synthetic context emits deterministic `binding_missing` for `feature.copy_bios/bios_source_dir`; placeholder directory binding emits successfully in the bound synthetic corpus context and in checked-in repo-plan contexts that select this recipe. | Manual selected ref plus synthetic fixture context in P7H; checked-in repo device-plan/profile selected-ref ingestion in P7I/P7J/P7O. | Planner-only BIOS binding does not inspect BIOS contents; executor/apply behavior is unchanged. | `planner_tests::authored_corpus_unbound_required_inputs_emit_classified_errors`; `planner_tests::authored_corpus_supported_synthetic_context_emits_execution_plan`; `planner_tests::repo_device_plan_ingestion_accepts_supplied_bindings_without_applying_metadata_overrides`; `planner_tests::repo_plan_e2e_no_app_data_write_contexts_prune_app_data_dependents` |

## Selected-Recipe Expansion Classification

The current authored recipe dependency field is the top-level
`recipe_dependencies` sequence. Python parses an omitted field as an empty
tuple, and Rust parses an omitted field as an empty vector. The current
checked-in recipe corpus contains no non-empty recipe dependency metadata:
`authored/recipes/app.obtainium.install.yaml`,
`authored/recipes/app.retroarch.provision.yaml`, and
`authored/recipes/feature.copy_bios.yaml` declare `recipe_dependencies: []`;
`authored/recipes/app.xaniteog.install.yaml` omits the field and therefore
parses as empty. P7K guards this current-state evidence with
`recipe_expansion_current_authored_corpus_has_no_non_empty_recipe_dependencies`.

Python planner expansion is owned by
`src/emuchef/planner/dependencies.py::expand_recipe_dependencies`, called from
`src/emuchef/planner/draft_builder.py::build_draft_plan`. It performs a DFS
over selected recipes and direct `recipe_dependencies`: dependencies are added
before their dependent, sibling dependencies follow authored
`recipe_dependencies` order, selected recipes are expanded in selected-ref
order, transitive dependencies are supported, and already-expanded recipes are
not moved when referenced again. Python returns `recipe_not_found` for unknown
selected refs or dependency refs and `dependency_cycle` for recipe cycles.

Before P7K, `crates/emuchef-rust-backend/src/planner.rs` already used the same
private DFS expansion in fixture-scoped `plan_execution`, with Phase 6M/6N
golden coverage in `recipe_and_step_dependencies_match_python_ordering` and
`phase6n_dependency_expansion_and_namespacing_match_python`. P7K pins that
ordering in `recipe_expansion_explicit_selected_refs_preserve_order_without_dependencies`
and
`recipe_expansion_order_is_dependency_first_authored_and_first_occurrence_stable`.
P7K also distinguishes unknown explicit selected refs from unknown dependency
refs in Rust planner diagnostics by preserving `recipe_ref` and adding a
`source` field plus `selected_recipe_ref` or `dependency_ref` /
`dependent_recipe_ref` context. `recipe_expansion_unknown_selected_ref_error_shape_has_selected_context`,
`recipe_expansion_unknown_dependency_ref_error_shape_has_dependency_context`,
and `recipe_expansion_dependency_cycle_error_shape_has_cycle_context` assert
`status: error`, `execution_plan: null`, the diagnostic code, and relevant
context.

Current checked-in device plans select only recipes with empty dependency
closures. P7K treats selected/expanded equality as evidence for the current
corpus, not a future invariant:
`recipe_expansion_current_corpus_selected_and_expanded_refs_match` and
`recipe_expansion_checked_in_device_plan_selected_sets_match_expanded_refs_for_current_corpus`
assert that checked-in corpus/device-plan selected sets currently produce
matching `selected_recipe_refs` and `expanded_recipe_refs`. Adding non-empty
checked-in `recipe_dependencies` should intentionally update these tests and
this section.

The current schema parses `provides.features`, and current recipes use it for
feature metadata such as `retroarch_provision`, `bios_copy`,
`xaniteog_install`, and `obtainium_install`. The searched current source and
fixtures do not define an active top-level `requires` field, and neither Python
nor Rust planner expansion resolves recipe dependencies from `provides` /
`requires` capabilities. Capability/service resolution remains unsupported in
this slice and is a planner cutover gap if later product requirements depend on
it.

P7K does not expose Rust planner expansion through CLI, Tauri, sidecar
protocol, executor/apply behavior, real-device flows, Python invocation, ADB,
network access, artifact materialization, or fixture/golden regeneration.
Python remains the CLI/reference planner owner. For current checked-in plans,
selected-recipe expansion is not a cutover blocker because there are no
non-empty checked-in dependency closures; broader CLI/reference cutover still
requires proving or retiring the remaining Python planner behavior.

## Device-Plan Defaults/Overrides Classification

Current checked-in device plans under `authored/device_plans/` all contain
`defaults.show_advanced_steps: false` and
`overrides.config_variants.vendor_family: ayaneo`. Their
`overrides.config_variants.screen_class` values are `handheld_16_9` for
`ayaneo.generic.base`, `ayaneo.konkr_pocket_fit.base`,
`ayaneo.pocket_s2.base`, and `ayaneo.pocket_s_mini.base`, and
`handheld_4_3` for `ayaneo.pocket_air_mini.base`. Current checked-in device
plans do not contain ref-shaped override binding keys.

| Field path | Concrete examples | Current Python behavior | Current Rust behavior | Classification | Must implement before planner cutover? | Deletion/cutover blocker |
| --- | --- | --- | --- | --- | --- | --- |
| `defaults.show_advanced_steps` | Present as `false` in all five checked-in device plans. | Parsed into `DevicePlan.defaults`; not consumed by `Planner.start_session`, draft building, binding resolution, or execution-plan emission. | Parsed by private P7J ingestion and classified as inactive; never inserted into `PlannerInput.input_bindings`. | Inactive metadata/ignored for planner behavior. | No for current Python parity and checked-in plans. | Not a blocker unless future product semantics make plan defaults active. |
| `defaults.<recipe_ref>/<input_id>` | No checked-in examples. Temporary tests use `feature.copy_bios/bios_source_dir` only to prove inactivity. | Parsed but not applied as planner bindings. | Parsed but not applied as planner bindings. | Unsupported/inactive. | No for current checked-in plans. | Future active default semantics would need a new Rust slice. |
| `overrides.config_variants` | Present in all five checked-in device plans; contains `vendor_family` and `screen_class`. | `normalize_planner_overrides(..., allow_metadata_keys=True)` permits metadata keys without turning them into bindings; emitted plans are unaffected. | Recognized as the only allowed metadata-only top-level override key; ignored by binding merge and emitted plans. | Metadata-only. | No for current Python parity because Python also does not select variants. | Config variant selection remains a cutover gap if product behavior later requires it. |
| `overrides.<recipe_ref>/<input_id>` | No checked-in examples. Temporary authored-root tests cover `feature.copy_bios/bios_source_dir` and `app.xaniteog.install/xaniteog_apk`. | Ref-shaped keys participate in planner binding resolution; user bindings override planner overrides. | Private P7J ingestion validates exact one-slash keys, inserts override bindings in YAML order, and then applies explicit test bindings so explicit bindings take precedence while existing key order is preserved by `IndexMap`. | Semantic planner input binding. | Yes for any cutover path that needs Python-compatible device-plan override bindings. | Current checked-in plans are not blocked by this because they do not use ref-shaped override keys; full Python-compatible override support still needs broader forms. |
| `overrides.inputs.<id>` or other broader Python forms | No checked-in examples. | Python strips an `inputs.` prefix before resolving known bindings. | Unsupported in P7J; not accepted as an allowed top-level override key form. | Unsupported gap. | Yes if future authored plans or cutover requirements depend on the broader Python spelling. | Broader override-form parity remains a cutover gap. |
| Other top-level `overrides.*` keys | No checked-in examples beyond `config_variants`. | Python currently allows metadata-looking device-plan override keys without slashes. | P7J rejects every top-level override key except `config_variants` and exact `<recipe_ref>/<input_id>` keys with deterministic `device_plan_override_unsupported`, `device_plan_override_malformed`, or `device_plan_override_unknown_binding` ingestion errors. | Unsupported/malformed. | No for current checked-in plans. | Broader metadata semantics remain unsupported unless explicitly approved later. |

## Python Planner Paths Required Today

The Python planner remains required for current user-facing CLI/reference
behavior. The `emuchef` console script still routes through `src/emuchef/cli.py`
and the Python planner modules for `draft` and `plan`. Python planner tests
remain active reference coverage. Dev-only/reference-only regeneration commands
for Phase 6M/6N planner goldens remain documented in
`crates/emuchef-rust-backend/README.md` and classified in
`docs/python-fixture-golden-ownership.md`.

The Rust planner skeleton is not a replacement command path. P8A exposes an
explicit developer-only Python CLI bridge to the shadow binary:
`emuchef plan --planner-backend rust-shadow --rust-planner-bin <path>`. P8G adds
the explicit non-default migration route
`emuchef plan --planner-backend rust-experimental --rust-planner-bin <path>`,
and P8H adds dev-only matrix smoke evidence for that route across the current
scenario matrix. Both routes require a supplied shadow binary, and neither is
exposed as a Tauri command,
sidecar protocol request, production/default CLI command, stable final public
contract, or runtime fallback. The Rust planner's internal permission-intent
helper is not a serialized execution-plan field and is not consumed by
executor/apply behavior. P7G DTO coverage and the explicit Python CLI migration
routes do not change those boundaries.

## Dev-Only Shadow Emission

The Rust backend crate provides a developer-only Cargo binary named
`emuchef-plan-shadow` for manual migration inspection:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml --bin emuchef-plan-shadow -- \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

P8R adds a local fixture-only detected-facts path to the shadow binary:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml --bin emuchef-plan-shadow -- \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --detected-facts-json /path/to/detected-facts.json
```

The fixture file is strict JSON matching `DetectedDeviceFacts`; all scalar
fields are optional and omitted `device_tags` defaults to an empty list. Fixture
mode runs the P8Q detected-facts planning-result composition path and emits the
same pretty JSON `PlanningResult` shape as normal shadow planning. The effective
context precedence is synthetic/profile context -> detected fixture facts ->
explicit CLI context overrides. If explicit context flags and a fixture are both
supplied, the emitted `execution_plan.device_context` reflects the explicit
overrides, while `device_profile_mismatch` warning evaluation remains based on
the detected fixture facts. Missing or invalid fixtures are process errors:
stderr contains a stable `detected_facts_fixture_*` classification and stdout is
empty. This fixture mode does not probe devices, invoke ADB, add Python CLI
forwarding, enter the Python CLI matrix smoke runner, or promote route-level
mismatch-warning parity.

P8S adds optional/manual smoke evidence for the P8R fixture path:

```bash
python3 tools/smoke_rust_detected_facts_fixture.py \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow>
```

The P8S smoke invokes the supplied shadow binary directly, writes only
temporary detected-facts fixtures, and reports deterministic JSON without temp
paths, host-specific binary paths, or full process output. It is not a normal
check, not readiness-gate execution, not Python CLI route behavior, and not live
device probing.

P8A also exposes a thin Python CLI bridge to an already-built shadow binary:

```bash
emuchef plan \
  --planner-backend rust-shadow \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

The Python bridge requires `--rust-planner-bin` in P8A and never invokes Cargo.
It forwards `--authored-root`, `--device-plan`, and repeated raw `--bind` values
to the supplied binary in original order. In the default passthrough mode, Rust
stdout/stderr are passed through directly as JSON/text from the shadow command;
that default mode does not translate Rust JSON into Python YAML, concise summary
text, or Python planner structures. `--output` and `--verbose` are supported
only when the explicit P8E `python-compatible` output mode is selected.

The bridge also supports an explicit P8E formatter mode:

```bash
emuchef plan \
  --planner-backend rust-shadow \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --rust-shadow-output python-compatible \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

In `python-compatible` mode, Rust stdout must contain a `PlanningResult` JSON
object. The Python CLI formats that mapping into the visible Python planning
summary labels by default, emits structured YAML to stdout for `--verbose`, and
writes structured YAML to the requested path for `--output` while stdout remains
the concise summary. If Rust emits usable planning JSON with a non-zero exit
code, the CLI formats the result and preserves the Rust exit code. If stdout is
empty, invalid JSON, or not a planning result, the CLI reports a compatibility
mode error. Omitted `--rust-shadow-output` and explicit
`--rust-shadow-output passthrough` retain P8C passthrough semantics.

P8G adds an explicit non-default migration route for cutover rehearsal:

```bash
emuchef plan \
  --planner-backend rust-experimental \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

`rust-experimental` reuses the same Rust shadow planner binary invocation as
`rust-shadow` and defaults to Python-compatible output formatting. It supports
`--verbose` and `--output` through that compatibility formatter. It rejects
`--rust-shadow-output` because that option is only valid with
`--planner-backend rust-shadow`. `rust-experimental` is not the default planner,
not a stable final public contract, and not Python planner deletion. Its name and
behavior may change before Rust becomes the default planner backend.

P8B provides a dev-only matrix smoke for the Python bridge:

```bash
python3 tools/smoke_rust_shadow_cli_matrix.py \
  --scenario-matrix tools/plan_parity_scenarios.json \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow>
```

The smoke requires an already-built shadow binary and never builds it. It is
route-invocation evidence only, not planner-output parity evidence.

P8F uses the same smoke runner for explicit Python-compatible output-mode smoke:

```bash
python3 tools/smoke_rust_shadow_cli_matrix.py \
  --scenario-matrix tools/plan_parity_scenarios.json \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --rust-shadow-output python-compatible
```

This compatibility smoke requires concise Python-compatible summary stdout for
successful scenarios. It does not call the Python-vs-Rust comparison harness,
execute/apply plans, probe devices, invoke ADB, access the network, materialize
artifacts, regenerate goldens, or participate in normal Rust/Tauri runtime
checks.

P8H uses the same smoke runner for the explicit `rust-experimental` route without
renaming the tool:

```bash
python3 tools/smoke_rust_shadow_cli_matrix.py \
  --scenario-matrix tools/plan_parity_scenarios.json \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --planner-backend rust-experimental
```

In that mode, generated commands omit `--rust-shadow-output`, report an
effective Python-compatible output mode, and require successful scenarios to
exit `0` and emit concise Python-compatible summary stdout.

The shadow command builds `PlannerInput` through the private
`PlannerInput::from_authored_device_plan(...)` path and then calls
`plan_execution(...)`. Explicit `--manufacturer`, `--model`, and
`--android-version` values override the synthetic/profile-derived planner
context for the invocation. Repeated `--device-tag` values replace
profile-derived tags in supplied order; when no tags are supplied, profile tags
remain unchanged. Explicit `--bind <recipe_ref>/<input_id>=<value>` values are
string-only in this slice. Repeated binds for the same ref are grouped into a
string array because the current Python CLI parser groups repeated
`--bind REF=VALUE` entries that way. This is a shadow-command limitation and is
not full future Rust planner CLI binding type parity.

Planner success and planner validation failures, including missing required
bindings, emit deterministic pretty JSON `PlanningResult` values to stdout.
Success exits `0`; planner error results exit non-zero. Argument/usage failures
and authored-root/device-plan load failures write stable text to stderr and do
not emit stdout JSON.

The shadow command and Python bridge do not execute plans, probe devices, invoke
ADB, access the network, download or materialize artifacts, regenerate goldens,
expose Tauri commands, expose sidecar protocol requests, or replace the default
Python `emuchef plan` CLI. The P8R fixture mode can create detected facts only
from a local JSON file supplied directly to the Rust shadow binary; Python CLI
routes do not expose or forward that mode. ADR 0003 decides that Rust should own
real-device probing and detected-device profile mismatch warning parity before
future default Rust planner cutover, but P8M/P8R do not implement live probing
or production route-level warning parity. Planner CLI cutover remains a future
explicit phase.

## Dev-Only Python Planner API Vs Rust Shadow Comparison

`tools/compare_rust_python_plan.py` compares Python planner API output with the
Rust `emuchef-plan-shadow` output for an explicit authored root, device plan,
and repeated `<recipe_ref>/<input_id>=<value>` bindings. It is a reporting tool,
not a cutover path. The harness does not call the user-facing Python
`emuchef plan` CLI because that CLI resolves ADB and probes device facts before
planning.

The Python worker uses the closest current planner API path:
`load_authored_catalog(...)`, `Planner.start_session(...)`, optional
`session.bind_input(...)`, and `session.emit_execution_plan()`. Because
`Planner.start_session(...)` requires an explicit `DeviceContext`, the harness
uses a shared synthetic/profile-derived planner context for both sides:
manufacturer comes from the first profile `match.manufacturer_contains` value
or `profile:<profile_id>`, model comes from the profile name or
`profile:<profile_id>`, Android version comes from `match.android_version.min`
or `0`, API level is `null`, device tags come from the profile, and runtime
capabilities come from `capability_defaults`. This proves Python planner API
versus Rust shadow behavior under that planner context only. It does not prove
Python CLI/device-probing parity.

The report is deterministic JSON with classification buckets for `match`,
`rust_missing`, `python_missing`, `value_mismatch`, `known_gap`,
`intentional_shape_difference`, and `unsupported`. It compares top-level status,
selected refs, expanded refs, execution-plan presence, step count, step ids and
order, step types, dependencies, normalized params, warning/error shape, and
serialized `permission_plan` presence. JSON object key order is ignored;
semantic list order remains compared.

The default Rust command mode is offline Cargo:

```bash
cargo run --offline --quiet --manifest-path crates/emuchef-rust-backend/Cargo.toml --bin emuchef-plan-shadow -- ...
```

Offline mode may fail on a fresh checkout when Cargo dependencies are not
prefetched. Developers can pass `--cargo-online`, set
`EMUCHEF_PLAN_COMPARE_CARGO_OFFLINE=0`, or pass `--rust-bin <path>` for a
prebuilt shadow binary. Command-construction tests cover these modes without
requiring network access.

Example successful comparison:

```bash
.venv/bin/python tools/compare_rust_python_plan.py \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

Required file/directory bindings must satisfy Python planner validation. Use
planner-only local placeholders; the harness does not download or materialize
artifact payloads:

```bash
mkdir -p /tmp/emuchef-p7n-bios
: > /tmp/emuchef-p7n-xaniteog.apk
.venv/bin/python tools/compare_rust_python_plan.py \
  --authored-root authored \
  --device-plan ayaneo.pocket_s2.base \
  --bind feature.copy_bios/bios_source_dir=/tmp/emuchef-p7n-bios \
  --bind app.xaniteog.install/xaniteog_apk=/tmp/emuchef-p7n-xaniteog.apk
```

The Pocket S2 comparison currently reports `match` when required planner-only
BIOS and XaniteOG bindings are supplied. The old
`rust_optional_step_pruning_dependency_bug` remains covered by a synthetic
comparison-harness unit test for stale or future Rust outputs, but it is not a
current checked-in repo-plan comparison gap.

## Cutover Readiness

The current Rust planner evidence is incremental parity evidence only. Python
remains the user-facing CLI/reference planner owner, and Rust remains
shadow/dev-only. The current checked-in scenario matrix covers all five
checked-in device plans with six scenarios, including one P8K explicit-context
scenario, and expects every scenario to classify as `match`. P8L records that
explicit device context has static coverage evidence through that matrix schema
and scenario, but real-device probing and detected-device profile mismatch
warnings remain blocked. P8M records the accepted future-cutover ownership model:
Rust should own those behaviors before default Rust planner cutover. P8N adds
the crate-local probe abstraction and fake/non-live tests only. P8O adds
fake/test-backed detected-facts planner-input construction only. The intended
future precedence is synthetic/profile context -> detected facts -> explicit CLI
overrides. P8P adds pure/test-backed mismatch-warning helper logic for supplied
detected facts. P8Q composes those fake/test-backed pieces into crate-private
`PlanningResult` construction, but does not add live ADB probing, normal planner
warning emission, or route wiring. P8R makes that composition path executable
only through a local `emuchef-plan-shadow --detected-facts-json <path>` fixture
harness; Python CLI routes, smoke tooling, live probing, normal runtime checks,
and readiness blocker IDs remain unchanged. Matching matrix status is necessary
evidence for the compared planner-only fields, not sufficient proof of CLI
routing, real-device context resolution, executor/apply compatibility, artifact
materialization, or Python planner deletability.

Use `docs/rust-planner-cutover-readiness.md` for the current blocker
classification, comparison-matrix gating policy, and proposed staged ladder for
future user-facing Rust planner routing and Python planner deletion. Use
`docs/adr/0002-rust-planner-cli-output-compatibility.md` for the accepted
default-route output and exit-code compatibility target. Use
`docs/adr/0003-rust-real-device-context-ownership.md` for the accepted
future-cutover real-device context ownership model.
