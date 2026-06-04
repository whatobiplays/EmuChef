# Rust Planner Parity Boundary

This document defines the current planner ownership boundary for the
Python-to-Rust/Tauri migration. Python planner behavior is not deleted,
deprecated, or ready for cutover. Rust planner coverage remains crate-internal
and fixture-scoped.

Rust planner tests include an intentional fixture inventory/parsing guard for
the existing Phase 6M/6N planner parity evidence. The guard consumes checked-in
files only; it does not invoke Python or regenerate fixtures.

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
- `crates/emuchef-rust-backend/src/planner_tests.rs`: Rust unit tests comparing
  Phase 6M/6N planning output to checked-in Python planner goldens, including
  the P7A fixture inventory/parsing guard and focused planner-only param
  contract coverage for selected emitted step types. P7F permission-intent tests
  serialize the internal Rust helper output only for assertions; that serialized
  helper output is not an execution-plan DTO surface.
- `crates/emuchef-rust-backend/tests/phase6m_planner.rs`: protocol guard that
  keeps planner requests unrouted and capabilities editor-scoped.
- `crates/emuchef-rust-backend/tests/fixtures/authored_root/planner_*`;
  `crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6m_planner_*.json`;
  `crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6n_planner_*.json`:
  frozen authored inputs and Python planner evidence consumed by Rust tests.

## Planner Boundary Matrix

| Planner area | Current Python owner | Rust evidence | Current status | Before Python planner deletion |
| --- | --- | --- | --- | --- |
| Authored YAML loading/parsing | `src/emuchef/io/loader.py`; `src/emuchef/io/validation.py` | `crates/emuchef-rust-backend/src/yaml.rs`; `phase6e_yaml.rs`; `planner_tests.rs` loading fixture roots | Fixture-scoped Rust parsing exists for selected authored roots. | Prove all required authored corpus and planner inputs parse with matching semantics. |
| Schema/domain validation | `src/emuchef/io/validation.py`; `src/emuchef/planner/contracts.py`; domain modules under `src/emuchef/domain/` | `validation.rs`; `phase6k_validation.rs`; `phase6l_catalog_validation.rs`; planner golden comparisons | Rust validates selected editor/planner rules only. | Port or retire Python planner-owned diagnostics and validation rules with parity tests. |
| Input binding/default handling | `src/emuchef/planner/bindings.py`; `draft_builder.py`; `emitter.py` | `required_input_bindings_match_python_success_and_missing_error`; `phase6n_optional_inputs_prune_and_rebind_like_python`; `phase6n_input_defaults_and_multiple_values_match_python` | Covered for selected required, optional, default, and multiple input fixtures. | Cover device plans, overrides, operation replay, validation errors, and broader input declarations. |
| Selected recipe expansion | `src/emuchef/planner/service.py`; `dependencies.py`; `draft_builder.py` | `recipe_and_step_dependencies_match_python_ordering`; `phase6n_dependency_expansion_and_namespacing_match_python` | Covered for focused recipe dependency fixtures. | Prove selected recipe expansion across the required authored catalog and CLI workflows. |
| Dependency validation | `src/emuchef/planner/dependencies.py`; `src/emuchef/io/validation.py` | `phase6l_catalog_validation.rs`; planner dependency tests; `dependency_validation_*` planner tests | Rust validates selected/emitted step dependencies for unknown or non-emitted targets, self-dependencies, and static cycles. Duplicate authored dependencies remain allowed and are preserved in emitted execution steps. Runtime dependency outcomes remain executor behavior. | Match Python planner diagnostics, ordering, and failure behavior for broader planner-facing dependency cases. |
| Artifact group expansion | `src/emuchef/steps/planner_hooks.py`; `src/emuchef/planner/emitter.py` | `refs_artifacts_defaults_and_conditions_match_python_execution_plan` | Covered for focused `resolve_artifacts` and `extract_archive` fixture behavior. | Cover all planner hook cases and invalid artifact/group diagnostics. |
| Duplicate detection | `src/emuchef/io/validation.py`; `src/emuchef/planner/catalog.py` | `phase6l_catalog_validation.rs` duplicate fixture cases | Catalog duplicate diagnostics are fixture-scoped; planner replacement is not complete. | Prove duplicate behavior for all catalog/planner inputs or explicitly retire unsupported shapes. |
| Param normalization and focused contract checks | `src/emuchef/planner/contracts.py`; `src/emuchef/steps/planner_hooks.py`; `emitter.py` | `phase6n_builtin_planner_mappings_match_python_goldens`; `phase6n_step_data_refs_conditions_and_constraints_match_python`; focused P7E/P7F param contract tests in `planner_tests.rs` | Covered for selected built-in StepSpec defaults and hook outputs. The Rust planner also validates focused emitted-step param contracts for `copy_files`, `extract_artifacts`, `extract_archive`, `install_apk`, `wait`, and `grant_permissions`, including required params, ref/literal mode, selected enum/bool/scalar values, group-only artifact selection, and unknown params only for those focused step types. | Port full StepSpec planner hooks and normalization behavior for all supported steps. |
| Literal/ref param representation | `src/emuchef/domain/param_values.py`; `src/emuchef/planner/emitter.py` | Phase 6M/6N planner goldens parsed by `planner_parity_fixture_inventory_consumes_checked_in_evidence_only` | Rust emits Python-shaped literal/ref JSON for selected fixtures. | Prove all required literal/runtime/ref values match Python output and plan IO expectations. |
| Shorthand step ref rewriting | `src/emuchef/planner/emitter.py`; `src/emuchef/planner/contracts.py` | `refs_artifacts_defaults_and_conditions_match_python_execution_plan`; `phase6n_step_data_refs_conditions_and_constraints_match_python`; `step_ref_validation_preserves_explicit_refs_and_rewrites_shorthand_refs` | Covered for selected `steps.<id>` and `steps.<id>.outputs.<field>` refs in emitted Rust planner steps. | Cover cross-recipe refs, all planner-supported ref contexts, and broader command paths. |
| Static ref target validation | `src/emuchef/planner/contracts.py`; `src/emuchef/io/validation.py` | `phase6k_validation.rs`; `phase6l_catalog_validation.rs`; selected planner tests | Rust validates selected emitted step refs for malformed `steps.*`, unknown selected step targets, unknown outputs, and shorthand refs to steps without a primary output. Non-step refs remain outside this slice. | Port planner-owned static ref diagnostics and ensure CLI planning paths use Rust equivalents. |
| Permission intent construction | Current permission declarations are step-local authored data under selected `grant_permissions.params.runtime`, `grant_permissions.params.appops`, and `grant_permissions.params.policy`; top-level recipe `permissions:` is invalid, and manual permission declarations are unsupported. | `grant_permissions_stays_step_local_without_permission_plan`; P7F `build_permission_intent` tests; executor permission tests | Rust builds internal structured permission intent from selected `grant_permissions` authored step params. The helper preserves runtime permissions, appops, policy metadata, required flags, and `when` metadata without emitting shell/ADB command strings. Rust planner output still omits serialized `permission_plan`. | Keep step-local permission semantics unless a later approved schema/DTO slice deliberately changes the source or serialized surface. |
| Execution plan DTO/schema emission | `src/emuchef/domain/execution_plan.py`; `src/emuchef/planner/emitter.py`; `src/emuchef/io/serde.py` | `PlanningResult`/`ExecutionPlan` structs in `planner.rs`; Phase 6M/6N golden comparisons | Rust emits matching DTOs for selected frozen flows only. | Prove all required plan DTO fields, schema handling, output formats, and CLI `--output` behavior. |
| Deterministic output ordering | `src/emuchef/planner/dependencies.py`; `emitter.py`; Python ordered domain models | `recipe_and_step_dependencies_match_python_ordering`; `phase6n_dependency_expansion_and_namespacing_match_python`; P7A inventory guard | Fixture-scoped ordering parity exists for selected dependencies, inputs, artifacts, and steps. | Prove deterministic ordering for all required catalog/plan combinations. |
| Diagnostics/error model | `src/emuchef/domain/errors.py`; `src/emuchef/planner/*`; `src/emuchef/io/validation.py` | Required binding missing, dependency cycle, catalog validation tests, focused P7E param contract diagnostics | Rust covers selected error/status shapes. Focused Rust planner param contract diagnostics include recipe, step, step type, param, expected value/mode, and actual value context. | Port or retire all planner-facing diagnostics, details fields, CLI output, and exit-code expectations. |
| Corpus/golden coverage | `tests/test_planner_core.py`; `tests/test_cli.py`; dev-only/reference-only golden generation in `crates/emuchef-rust-backend/README.md` | `planner_tests.rs`; `phase6m_planner.rs`; `phase6m_planner_*.json`; `phase6n_planner_*.json` | P7A guards the current frozen inventory and parses every planner golden. | Broaden corpus coverage or reclassify historical tests before deleting Python planner code. |

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
coverage, including P7F internal permission-intent construction coverage, are
incremental parity evidence only.
