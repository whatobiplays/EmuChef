# Copy ROM Library Automated Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add source-bound automated qualification for the real `feature.copy_roms` authored workflow through the production catalog, runtime planner, review projection, and deterministic executor, including the authored `merge`, `replace`, and `sync` policy behaviors without any physical-device or network work.

**Architecture:** Follow the established standalone BIOS/Obtainium qualification pattern: bind a strict checked-in contract to the raw authored recipe bytes, explicitly select `feature.copy_roms` in a real production capability context, plan only through `runtime_configuration::plan_configuration`, inspect the production review, then execute the unchanged generated plan through sandbox-root adapters. Use deterministic host and fake-device filesystem fixtures to prove copy semantics and failure behavior; do not add qualification-only production APIs or alter device-plan membership.

**Tech Stack:** Rust, Serde/serde_json, SHA-256 via `sha2`, the existing authored catalog and runtime planner, production review projection, `ExecutorRunner`/`ExecutorAdapters`, `tempfile`, YAML-authored recipes, existing repository validation commands.

## Global Constraints

- Owner: **EmuChef proper**. Shared Runtime changes are allowed only if a failing qualification test proves an existing production contract defect that cannot be tested otherwise.
- Phase 6D remains **In progress**. Its missing physical/UI evidence is owner-deferred, not waived or reclassified.
- Phase 6E remains **In progress**. This task may add automated qualification for `feature.copy_roms`; it must not claim physical, packaged-GUI, release, or full end-to-end qualification.
- The source authority is `authored/recipes/feature.copy_roms.yaml`. Do not create a copied test-owned recipe.
- Bind the qualification contract to the current raw recipe SHA-256 `956838151ed9048421e4c88d0895abe5b7f1a1998731c7dd2fbbee9cc13c2041`; a source change must fail closed until expectations are reviewed.
- Use `ayaneo.generic.base` only as a real production capability context because it supplies `shared_storage_write`; explicitly select `feature.copy_roms`. Do not add/remove the recipe from any device plan or infer product provenance from plan membership.
- Every executable qualification plan must come from `CatalogSnapshot::legacy_local(authored_root())` through `runtime_configuration::plan_configuration`. Do not hand-build an `ExecutionPlan` or mutate the generated plan before execution.
- Executor qualification must use deterministic sandbox-root adapters. No `RealAdbDevice`, `Command::new("adb")`, ignored physical tests, external device access, host-sleep qualification, UI-smoke qualification, or live network access.
- Preserve Rust/Tauri authority boundaries and all public/serialized APIs. Do not add qualification-only planner, review, executor, DTO, frontend, or protocol fields.
- Preserve existing `copy_files` production semantics. Change production executor/planner behavior only if a new failing test proves a genuine defect against already-authored policy semantics; make the smallest coherent correction and document the rationale.
- The authored default destination remains `/sdcard/ROMs`; qualification must also prove the allowed `/storage/emulated/0/...` device-path form through planning/binding validation without changing the recipe.
- The authored default policy remains `merge`; `replace` and `sync` are explicit alternate bindings and must be qualified independently.
- Keep current strict Clippy behavior. Do not weaken `-D warnings` or add broad lint suppressions.
- Update current-state product documentation only after the automated gates pass.
- Implementation delegation should preserve the repository's normal review workflow; do not stage, commit, or push unless a later explicit owner instruction authorizes integration.

---

## File Structure

- **Create:** `tests/fixtures/recipe-qualification/roms/qualification-contract.json` — strict source/provenance and expected planning/execution contract for `feature.copy_roms`.
- **Create:** `crates/emuchef-rust-backend/src/recipe_qualification_roms_tests.rs` — source binding, production planning/review, deterministic copy-policy execution, invalid-binding, and failure regressions.
- **Modify:** `crates/emuchef-rust-backend/src/lib.rs` — register the qualification module under `#[cfg(test)]` only.
- **Create:** `docs/product/recipe-qualification-roms.md` — current automated qualification evidence, limits, and deferred physical status.
- **Modify:** `docs/product/product-roadmap.md` — record `feature.copy_roms` as automated-qualified after all gates pass while retaining Phase 6D and Phase 6E dispositions.
- **Modify only if current-state wording requires it:** `CONTEXT.md` — add the new automated qualification fact without changing runtime behavior claims.
- **Do not modify by default:** `authored/recipes/feature.copy_roms.yaml`, planner/executor production files, device plans, profiles, Phase 6D evidence/schema/manifest/index files, or frontend/Tauri code.

---

### Task 1: Add the strict ROM-copy qualification contract and source-binding test

**Files:**
- Create: `tests/fixtures/recipe-qualification/roms/qualification-contract.json`
- Create: `crates/emuchef-rust-backend/src/recipe_qualification_roms_tests.rs`
- Modify: `crates/emuchef-rust-backend/src/lib.rs`

**Interfaces:**
- Consumes: `CatalogSnapshot::legacy_local`, `runtime_configuration::plan_configuration`, `ConfigurationContextRequest`, `PlanConfigurationResult`, `BindingSource`, `ExecutionPlan`, `ExecutionStep`.
- Produces: a private `RomQualificationContract`, `load_contract()`, `plan_roms(...)`, `generated_copy_step(...)`, and fixture helpers used by later tasks.

- [ ] **Step 1: Create a strict qualification contract bound to the authored source**

Create `tests/fixtures/recipe-qualification/roms/qualification-contract.json` with this semantic shape and exact current source digest:

```json
{
  "schemaVersion": 1,
  "targetRecipe": "feature.copy_roms",
  "planningDevicePlan": "ayaneo.generic.base",
  "authoredSource": {
    "path": "authored/recipes/feature.copy_roms.yaml",
    "sha256": "956838151ed9048421e4c88d0895abe5b7f1a1998731c7dd2fbbee9cc13c2041"
  },
  "selectedRecipes": ["feature.copy_roms"],
  "expandedRecipes": ["feature.copy_roms"],
  "recipeConstraintCapabilities": ["shared_storage_write"],
  "qualificationContextCapabilities": ["shared_storage_write"],
  "requiredOperationFamilies": ["copy_files"],
  "inputs": {
    "source": {
      "key": "feature.copy_roms/source",
      "required": true,
      "type": "directory",
      "role": "rom_library"
    },
    "destination": {
      "key": "feature.copy_roms/destination",
      "required": true,
      "type": "device_path",
      "role": "rom_destination",
      "default": "/sdcard/ROMs"
    },
    "policy": {
      "key": "feature.copy_roms/policy",
      "required": false,
      "type": "enum",
      "role": "copy_policy",
      "default": "merge",
      "options": ["merge", "replace", "sync"]
    }
  },
  "copyStepIdSuffix": "copy_rom_library",
  "liveNetworkRequiredForAutomatedQualification": false,
  "automatedStatus": "qualified",
  "physicalStatus": "deferred",
  "physicalCleanupAuthority": "not_authorized_for_recipe_qualification"
}
```

The Rust contract types must use `#[serde(rename_all = "camelCase", deny_unknown_fields)]` at every object boundary so contract drift fails closed.

- [ ] **Step 2: Register a test-only qualification module**

Add this adjacent to the existing recipe qualification modules in `lib.rs`:

```rust
#[cfg(test)]
mod recipe_qualification_roms_tests;
```

Do not expose any production symbol.

- [ ] **Step 3: Add repository/contract helpers and the source-binding regression**

In `recipe_qualification_roms_tests.rs`, mirror the existing qualification modules' repository-root and SHA-256 helpers. Define:

```rust
const TARGET_RECIPE: &str = "feature.copy_roms";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.generic.base";
const SOURCE_INPUT_KEY: &str = "feature.copy_roms/source";
const DESTINATION_INPUT_KEY: &str = "feature.copy_roms/destination";
const POLICY_INPUT_KEY: &str = "feature.copy_roms/policy";
const DEFAULT_DESTINATION: &str = "/sdcard/ROMs";
const DEFAULT_POLICY: &str = "merge";
```

Add `rom_contract_binds_current_source_and_deferred_physical_status()` that:

```rust
let contract = load_contract();
assert_eq!(contract.schema_version, 1);
assert_eq!(contract.target_recipe, TARGET_RECIPE);
assert_eq!(contract.planning_device_plan, QUALIFICATION_DEVICE_PLAN);
assert_eq!(contract.selected_recipes, vec![TARGET_RECIPE]);
assert_eq!(contract.expanded_recipes, vec![TARGET_RECIPE]);
assert_eq!(contract.authored_source.path, "authored/recipes/feature.copy_roms.yaml");
assert_eq!(sha256_hex(&fs::read(repository_root().join(&contract.authored_source.path)).unwrap()), contract.authored_source.sha256);
assert_eq!(contract.automated_status, "qualified");
assert_eq!(contract.physical_status, "deferred");
assert_eq!(contract.physical_cleanup_authority, "not_authorized_for_recipe_qualification");
assert!(!contract.live_network_required_for_automated_qualification);
```

Also canonicalize the repository root and source path and assert the resolved recipe stays beneath the repository root, matching the BIOS/Obtainium fail-closed provenance pattern.

- [ ] **Step 4: Run the focused source-binding test**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml rom_contract_binds_current_source_and_deferred_physical_status -- --exact
```

Expected: PASS. If the exact-test filter syntax does not match the crate's module-qualified test naming, run the focused module filter instead and record the actual command used.

---

### Task 2: Prove production planning, binding validation, and review projection

**Files:**
- Modify: `crates/emuchef-rust-backend/src/recipe_qualification_roms_tests.rs`

**Interfaces:**
- Consumes: the Task 1 contract/helpers and production `plan_configuration` result.
- Produces: `plan_roms(source, destination, policy)` and plan/review regressions used unchanged by executor qualification.

- [ ] **Step 1: Implement a production-path planner helper**

Add a helper that loads the real authored catalog, explicitly selects only `feature.copy_roms`, and fills bindings only from its parameters:

```rust
fn plan_roms(
    source: Option<&Path>,
    destination: Option<&str>,
    policy: Option<&str>,
) -> PlanConfigurationResult {
    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    let mut explicit_bindings = OrderedMap::new();
    if let Some(path) = source {
        explicit_bindings.insert(
            SOURCE_INPUT_KEY.to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
    }
    if let Some(path) = destination {
        explicit_bindings.insert(
            DESTINATION_INPUT_KEY.to_string(),
            Value::String(path.to_string()),
        );
    }
    if let Some(value) = policy {
        explicit_bindings.insert(
            POLICY_INPUT_KEY.to_string(),
            Value::String(value.to_string()),
        );
    }

    plan_configuration(ConfigurationContextRequest {
        catalog,
        configuration_root: None,
        user_configuration: None,
        device_plan: Some(QUALIFICATION_DEVICE_PLAN.to_string()),
        selected_recipes: Some(vec![TARGET_RECIPE.to_string()]),
        explicit_bindings,
        device_context: None,
        target_device: None,
        runtime_capability_availability: None,
    })
    .expect("real authored ROM-copy configuration should prepare")
}
```

Do not use `selected_recipes: None`; this standalone qualification must not inherit unrelated device-plan defaults.

- [ ] **Step 2: Add the canonical default-binding planning/review regression**

Create a temporary source directory with at least two nested ROM-like files, e.g. `Nintendo/GBA/game.gba` and `Sony/PSX/game.chd`, then call `plan_roms(Some(&source), None, None)`.

Assert all of the following:

```rust
assert!(result.diagnostics.iter().all(|d| d.severity != "error"));
let plan = result.plan.as_ref().expect("ROM-copy plan should be generated");
assert!(result.plan_digest.as_deref().is_some_and(|digest| !digest.is_empty()));
assert_eq!(plan.source.selected_recipe_refs, vec![TARGET_RECIPE]);
assert_eq!(plan.source.expanded_recipe_refs, vec![TARGET_RECIPE]);
assert_eq!(plan.source.device_plan_ref, QUALIFICATION_DEVICE_PLAN);
assert_eq!(plan.source.device_profile_ref, "ayaneo.generic");
assert_eq!(plan.steps.len(), 1);
assert_eq!(plan.steps[0].type_name, "copy_files");
assert_eq!(plan.steps[0].constraints.capabilities, vec!["shared_storage_write"]);
assert!(plan.runtime_capabilities.shared_storage_write);
```

Find the three resolved input bindings by key and assert:

- source resolves to the explicit temporary directory and reports explicit provenance;
- destination resolves to `/sdcard/ROMs` from the authored default;
- policy resolves to `merge` from the authored default.

Do not hard-code a guessed enum variant for authored-default provenance if the existing runtime uses a different `BindingSource` name; read the actual `BindingSource` definition during implementation and assert the exact production variant used by current default binding resolution.

For the generated step, assert exact references/literals after planner resolution:

```rust
let step = generated_copy_step(plan);
assert_eq!(step.type_name, "copy_files");
assert_eq!(step.params.get("dest"), Some(&ExecutionParamValue::Literal {
    value: Value::String(DEFAULT_DESTINATION.to_string()),
}));
assert_eq!(step.params.get("copy_policy"), Some(&ExecutionParamValue::Literal {
    value: Value::String(DEFAULT_POLICY.to_string()),
}));
assert!(step.verify.is_empty());
```

Assert the source parameter is bound from `inputs.source` according to the current production planner representation; use the exact `ExecutionParamValue` shape emitted by the generated plan rather than inventing a test-only translation.

For the production review, assert:

```rust
let review = result.review.as_ref().expect("production review should exist");
assert!(review.can_execute);
assert_eq!(review.features.len(), 1);
assert!(!review.features[0].automatically_added);
assert!(review.features[0].sections.iter().any(|section| section.kind == "copies"));
assert_eq!(review.work.action_count, 1);
assert!(review.notices.iter().all(|notice| notice.severity != "blocker"));
```

Serialize the review and prove it may expose the friendly source leaf/label needed for user comprehension but does not expose the full temporary parent path, any device serial, or runtime authority data.

- [ ] **Step 3: Add required-source and path-validation regressions**

Add `rom_missing_required_source_prevents_executable_plan_and_review()`:

```rust
let result = plan_roms(None, None, None);
assert!(result.plan.is_none());
assert!(result.plan_digest.is_none());
assert!(result.review.is_none());
let diagnostic = result.diagnostics.iter()
    .find(|d| d.code == "binding_missing" && d.key.as_deref() == Some(SOURCE_INPUT_KEY))
    .expect("missing ROM source must be diagnosed");
```

Add `rom_nonexistent_source_directory_prevents_executable_plan_and_review()` and require the existing `binding_path_missing` diagnostic with explicit source provenance.

Add `rom_invalid_destination_prefix_prevents_executable_plan_and_review()` using `/data/local/tmp/roms` and require planning to fail via the existing binding-validation diagnostic for disallowed device-path prefixes. Do not assert a guessed diagnostic code until implementation has inspected the existing validator; assert the exact existing stable code that production already emits.

Add `rom_allowed_storage_emulated_destination_plans_successfully()` using `/storage/emulated/0/Emulation/ROMs` and assert the generated copy destination is exactly that value.

- [ ] **Step 4: Add enum-policy validation regressions**

For each valid explicit policy in `merge`, `replace`, and `sync`, call `plan_roms(Some(&source), None, Some(policy))` and assert the generated `copy_policy` literal equals the requested value.

Add an invalid-value case using `overwrite` and require planning to return no executable plan/review and the existing enum-binding diagnostic for `POLICY_INPUT_KEY`. Use the production diagnostic code already defined by runtime validation; do not introduce a new qualification-only code.

- [ ] **Step 5: Run the focused planning/review tests**

Run the module-focused qualification set:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml recipe_qualification_roms_tests
```

Expected: all Task 1-2 ROM qualification tests pass and no physical/ignored test executes.

---

### Task 3: Qualify default `merge` execution against deterministic filesystem state

**Files:**
- Modify: `crates/emuchef-rust-backend/src/recipe_qualification_roms_tests.rs`

**Interfaces:**
- Consumes: unchanged `ExecutionPlan` from `plan_roms` and `ExecutorAdapters::with_sandbox_roots`.
- Produces: deterministic workspace helpers reused by `replace`, `sync`, and failure tests.

- [ ] **Step 1: Build a deterministic ROM qualification workspace**

Add a private workspace with:

```rust
struct RomQualificationWorkspace {
    _temp: tempfile::TempDir,
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    host_input_root: PathBuf,
    rom_source_dir: PathBuf,
}
```

`new()` must create this source tree:

```text
rom-source/
  Nintendo/GBA/Metroid Fusion.gba     -> b"source-gba\n"
  Sony/PSX/Crash Bandicoot.chd        -> b"source-psx\n"
```

Add helpers mapping a device path like `/sdcard/ROMs/...` into the fake device root exactly as the existing sandbox adapter does. Do not duplicate executor path-normalization logic; only compute expected fixture locations beneath `fake_device_root` for assertions.

Create adapters with:

```rust
ExecutorAdapters::with_sandbox_roots(
    workspace.runtime_root.clone(),
    workspace.cache_root.clone(),
    workspace.fake_device_root.clone(),
    vec![workspace.host_input_root.clone()],
)
```

- [ ] **Step 2: Seed a pre-existing destination tree that distinguishes merge semantics**

Before execution, create:

```text
fake-device/sdcard/ROMs/
  Existing/keep.txt                    -> b"keep-me\n"
  Nintendo/GBA/Metroid Fusion.gba      -> b"old-gba\n"
```

This fixture proves both dimensions of merge behavior: a colliding source file is refreshed from the source while unrelated destination content remains present.

- [ ] **Step 3: Execute the exact default-generated plan**

Call `plan_roms(Some(&workspace.rom_source_dir), None, None)`, retain a clone for immutability comparison, and execute the returned plan without edits.

Assert:

```rust
assert_eq!(generated_copy_step(&plan).params.get("copy_policy"), Some(&ExecutionParamValue::Literal {
    value: Value::String("merge".to_string()),
}));
let plan_before = plan.clone();
let mut runner = ExecutorRunner::new(rom_adapters(&workspace));
let result = runner.run(&plan);
assert_eq!(plan, plan_before);
assert!(result.success);
assert_eq!(result.total_steps, 1);
assert_eq!(result.steps[0].status, StepRunStatus::Executed);
```

Then assert exact filesystem outcomes:

```text
Existing/keep.txt                 remains b"keep-me\n"
Nintendo/GBA/Metroid Fusion.gba  becomes b"source-gba\n"
Sony/PSX/Crash Bandicoot.chd     becomes b"source-psx\n"
```

Do not add a verification predicate to the recipe or generated plan merely for the test; the authored recipe currently has `verify: []`.

- [ ] **Step 4: Run the merge execution regression**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml rom_default_merge_generated_plan_executes_with_expected_filesystem_semantics
```

Expected: PASS with no network or ADB access.

---

### Task 4: Qualify explicit `replace` and `sync` policies from production-generated plans

**Files:**
- Modify: `crates/emuchef-rust-backend/src/recipe_qualification_roms_tests.rs`

**Interfaces:**
- Consumes: Task 3 workspace/adapters and production-generated plans.
- Produces: policy-specific executor evidence that distinguishes all three authored enum choices.

- [ ] **Step 1: Prove `replace` behavior using destination-only stale content**

Create a fresh workspace and pre-seed destination state:

```text
fake-device/sdcard/ROMs/
  stale-only.txt                        -> b"remove-me\n"
  Nintendo/GBA/Metroid Fusion.gba       -> b"old-gba\n"
```

Plan with `policy = Some("replace")`; assert the generated step contains the exact literal `replace`; execute the unchanged plan.

The test must assert the concrete current production semantics for `replace` by inspecting the existing executor implementation before finalizing the expectations. The intended authored meaning is **Replace destination**: the destination tree is recreated from source, so `stale-only.txt` is absent after success while both source ROM files exist with source bytes. If the current executor does not implement that authored meaning, first capture the mismatch as a failing qualification test; only then make the smallest production correction necessary and add a lower-level regression in the existing executor test module.

- [ ] **Step 2: Prove `sync` behavior using both stale and matching content**

Create a fresh workspace with:

```text
fake-device/sdcard/ROMs/
  stale-only.txt                        -> b"remove-me\n"
  Nintendo/GBA/Metroid Fusion.gba       -> b"old-gba\n"
```

Plan with `policy = Some("sync")`; assert the generated step literal is `sync`; execute the unchanged plan.

Assert the current production contract for **Mirror source**: source files match their source bytes and destination-only `stale-only.txt` is absent after success. Also prove nested source layout is preserved.

If executor implementation differentiates `replace` from `sync` beyond final tree equivalence (for example operation ordering or destructive setup), assert only stable externally meaningful semantics here; keep implementation-detail assertions in existing executor unit tests.

- [ ] **Step 3: Protect `merge` from accidental destructive convergence**

Retain the Task 3 assertion that `Existing/keep.txt` survives. Together the three policy tests must prove:

```text
merge   -> preserve unrelated destination content; update/add source content
replace -> destination becomes a fresh copy of source according to current production semantics
sync    -> destination mirrors source and removes destination-only content
```

If `replace` and `sync` are intentionally equivalent in the current product contract, document that explicitly in the qualification evidence rather than inventing a distinction. If they are accidentally equivalent despite different authored labels, treat that as a product-contract defect requiring owner review before broadening scope.

- [ ] **Step 4: Run the three policy execution regressions together**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml recipe_qualification_roms_tests
```

Expected: all merge/replace/sync execution cases pass with generated plans unchanged.

---

### Task 5: Prove deterministic failure behavior without inventing authored verification

**Files:**
- Modify: `crates/emuchef-rust-backend/src/recipe_qualification_roms_tests.rs`
- Modify only if a proven production defect requires it: `crates/emuchef-rust-backend/src/executor.rs` and the existing executor test module owning `copy_files` semantics.

**Interfaces:**
- Consumes: `ExecutorDevice`, `FakeDryRunDevice`, `DeviceOperationError`, generated ROM-copy plan.
- Produces: truthful fail-stop regression for copy failure.

- [ ] **Step 1: Add a delegating device that fails one copy operation**

Create `CopyFailureDevice` wrapping `FakeDryRunDevice`. Delegate every required `ExecutorDevice` method exactly as the existing BIOS/Obtainium qualification doubles do, except the operation used by `copy_files` to transfer source material must return:

```rust
Err(DeviceOperationError::other(
    "deterministic ROM copy failure",
))
```

The failing method must be chosen from the actual current executor path after inspection; do not guess whether the generated directory copy currently reaches `push`, `copy_on_device`, or another existing adapter method. Keep `uses_fake_device_filesystem()` consistent with the path needed to exercise the production `copy_files` branch.

- [ ] **Step 2: Assert fail-stop/reporting behavior on the unchanged plan**

Use the default `merge` generated plan. Clone it before execution, run with `ExecutorAdapters::with_device_and_sandbox_roots(...)`, and assert:

```rust
assert_eq!(plan, plan_before);
assert!(!result.success);
let record = result.steps.iter()
    .find(|record| record.step_id.ends_with("/copy_rom_library"))
    .expect("ROM copy execution record should exist");
assert_eq!(record.status, StepRunStatus::Failed);
assert!(record.message.as_deref().is_some_and(|message| !message.is_empty()));
assert!(result.steps.iter().all(|r| r.status != StepRunStatus::Blocked && r.status != StepRunStatus::Cancelled));
```

Because this recipe has one step and no authored verification, do not fabricate a downstream step or verification failure. The qualification claim is limited to production planning/review plus truthful copy execution success/failure semantics.

- [ ] **Step 3: If policy execution exposed a real production defect, fix it test-first and minimally**

Only when a Task 4 test fails because production behavior contradicts the authored policy label/contract:

1. Add the smallest focused failing regression in the existing executor test module that owns `copy_files` policy behavior.
2. Run that exact test and capture the failure.
3. Correct only the responsible `copy_files` production branch.
4. Re-run the focused executor regression and all ROM qualification tests.
5. Record why the authored contract required the change and why alternatives were rejected.

Do not use this task to refactor unrelated executor code or change policy names/options.

- [ ] **Step 4: Run focused failure and policy tests**

Run the focused ROM qualification module plus any new lower-level executor regression required by Step 3.

Expected: PASS; no physical-device code path executes.

---

### Task 6: Record truthful product evidence and run repository validation

**Files:**
- Create: `docs/product/recipe-qualification-roms.md`
- Modify: `docs/product/product-roadmap.md`
- Modify only if required by the repository's current-state contract: `CONTEXT.md`

**Interfaces:**
- Consumes: passing automated qualification evidence from Tasks 1-5.
- Produces: current product-state documentation only; no runtime/API behavior.

- [ ] **Step 1: Create the ROM-copy qualification evidence document after focused tests pass**

`docs/product/recipe-qualification-roms.md` must state:

- owner: EmuChef proper;
- target: real authored `feature.copy_roms`;
- source-bound SHA-256 and exact authored source path;
- production planning context: `ayaneo.generic.base` used only for `shared_storage_write` capability context with explicit recipe selection;
- required source directory, default `/sdcard/ROMs` destination, allowed `/storage/emulated/0/...` alternate destination, and `merge`/`replace`/`sync` policy coverage;
- production review projection coverage;
- deterministic sandbox executor evidence and exact policy behavior observed;
- invalid source, invalid destination, invalid enum, and deterministic copy-failure coverage;
- no ADB, physical device, live network, packaged GUI, signing/notarization, release, or physical cleanup authority;
- physical/end-to-end qualification remains deferred;
- Phase 6D remains In progress and its missing evidence is unchanged.

Do not claim verification-predicate coverage for this recipe because its authored `verify` list is empty.

- [ ] **Step 2: Update the canonical roadmap only after qualification passes**

In `docs/product/product-roadmap.md`:

- add `feature.copy_roms` to the Phase 6E current automated qualification status;
- keep Phase 6E **In progress** because other recipe and physical/end-to-end qualification remains;
- keep Phase 6D **In progress** and preserve every currently missing physical/UI evidence requirement;
- preserve ordinary production real execution as disabled;
- do not move Phase 6F to In progress.

- [ ] **Step 3: Update `CONTEXT.md` only if it currently enumerates recipe-qualification current state**

If `CONTEXT.md` already records standalone qualification status, add the ROM-copy automated qualification fact using the same distinction between automated and physical/end-to-end qualification. If it does not own that current-state fact, leave it untouched.

- [ ] **Step 4: Run the authored recipe validator**

Run the repository's existing CLI validator against the real authored recipe using the same command/path used by the current RetroArch/BIOS/Obtainium qualification work. Expected: `authored/recipes/feature.copy_roms.yaml` validates without errors.

Do not substitute a copied fixture recipe.

- [ ] **Step 5: Run backend format, compile, focused tests, full tests, and strict Clippy**

Run the established backend gates under both default and `real-execution` feature configurations. At minimum:

```bash
cargo fmt --all --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --check
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --no-default-features
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml recipe_qualification_roms_tests
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --no-default-features
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets --all-features -- -D warnings
```

If repository validation policy exposes canonical wrappers instead of direct Cargo commands, use those wrappers and record the exact accepted commands. Record observed test counts; do not bake historical counts into the acceptance criteria.

- [ ] **Step 6: Preserve Phase 6D evidence integrity**

Run the existing Phase 6D.6 evidence validator/regression gates used by recent qualification tasks. They may continue to report the known missing owner-deferred repetitions, but this task must not modify any accepted evidence, trace, schema, scenario manifest, or UI binding index merely to change that disposition.

- [ ] **Step 7: Verify final scope and repository state**

Use Git status/diff to prove:

- only the ROM qualification test/contract/docs plus any narrowly proven executor regression/fix changed;
- `authored/recipes/feature.copy_roms.yaml` remains byte-identical unless a separately justified authored defect was proven;
- no device plan/profile membership changed;
- no Phase 6D evidence/schema/manifest/index file changed;
- nothing is staged, committed, or pushed by the implementation run unless the owner separately authorizes integration.

---

## Technical Acceptance Criteria

- **TAC-1:** A strict deny-unknown-fields qualification contract binds `feature.copy_roms` to `authored/recipes/feature.copy_roms.yaml` SHA-256 `956838151ed9048421e4c88d0895abe5b7f1a1998731c7dd2fbbee9cc13c2041`, records automated status as qualified, and keeps physical status deferred with no recipe-qualification cleanup authority.
- **TAC-2:** Production planning loads the real authored catalog through `CatalogSnapshot::legacy_local`, uses `runtime_configuration::plan_configuration`, uses `ayaneo.generic.base` only for real capability context, explicitly selects only `feature.copy_roms`, and emits one `copy_files` step requiring `shared_storage_write`.
- **TAC-3:** The real binding contract is proven: source is a required existing directory; default destination is `/sdcard/ROMs`; `/storage/emulated/0/...` is accepted; disallowed destination prefixes fail closed; default policy is `merge`; explicit `merge`, `replace`, and `sync` are accepted; an unsupported policy is rejected before executable review/plan generation.
- **TAC-4:** Production review for a valid ROM-copy plan is executable and blocker-free, contains the copy action in the expected feature section, reports one action, and does not leak the full host parent path, device serial, or runtime authority data.
- **TAC-5:** The exact default-generated `merge` plan executes through deterministic sandbox-root adapters, preserves unrelated destination content, overwrites/adds colliding/source files with source bytes, preserves nested layout, and does not mutate the generated plan.
- **TAC-6:** Exact production-generated `replace` and `sync` plans execute deterministically and satisfy their existing authored policy meanings. Any discovered divergence is captured first as a failing regression and corrected only in the smallest responsible production branch.
- **TAC-7:** A deterministic device-operation failure causes the unchanged one-step ROM-copy plan to report failure truthfully with a failed copy record and no fabricated verification/downstream execution claim.
- **TAC-8:** The qualification module performs no ADB, physical-device, ignored-harness, packaged-GUI, or live-network work and adds no public/serialized API or qualification-only runtime authority.
- **TAC-9:** Authored recipe validation, backend format/check, focused ROM qualification tests, full backend tests under default and `real-execution`, strict backend Clippy, and the existing Phase 6D.6 validator/regressions satisfy their current contracts; actual observed counts/results are recorded.
- **TAC-10:** Product documentation records `feature.copy_roms` as automated-qualified only, keeps Phase 6E In progress, keeps Phase 6D In progress with all deferred evidence requirements unchanged, leaves Phase 6F Planned, and makes no physical/full-end-to-end/release qualification claim.
- **TAC-11:** Final repository review proves no device-plan membership, unrelated authored content, Phase 6D evidence, frontend/Tauri surface, or release configuration changed; any production executor change is present only if directly justified by a failing policy regression.

## Self-Review

- Spec coverage: source provenance, real catalog/planning, input/default/enum validation, review projection, merge/replace/sync executor behavior, deterministic failure behavior, documentation, and repository-wide validation all have explicit tasks.
- Placeholder scan: no implementation step depends on a `TBD`/`TODO` or unspecified future design. Two implementation-time inspections are intentionally required where the plan must use the **existing** stable production enum/diagnostic/adapter path rather than guessing a symbol or code; they are bounded by exact expected behavior and may not create new qualification-only contracts.
- Type consistency: `plan_roms(...) -> PlanConfigurationResult`, `generated_copy_step(&ExecutionPlan) -> &ExecutionStep`, the three binding keys, target recipe, planning device plan, default destination, and default policy are consistent across all tasks.
- Scope integrity: no new durable API/file naming uses project-management terms; phase terminology appears only in plans/product documentation where it is appropriate.
