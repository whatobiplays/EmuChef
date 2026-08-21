# BIOS Recipe Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Qualify the real authored `feature.copy_bios` workflow through production runtime configuration, review projection, required-input validation, and deterministic executor execution while migrating the existing RetroArch qualification implementation to stable domain-based names.

**Architecture:** Keep each recipe qualification independent and source-bound. First perform a behavior-preserving naming migration for RetroArch, then add a strict BIOS qualification contract plus a dedicated crate-root test module that plans only `feature.copy_bios` using `ayaneo.generic.base` for real capability context. Use the normal sandbox dry-run adapter for successful recursive copy coverage and a BIOS-test-private `ExecutorDevice` wrapper for the verification-failure case so the generated plan and production executor remain unchanged.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, `sha2`, `tempfile`, existing EmuChef `runtime_configuration`, review projection, planner, executor, fake-device/sandbox adapters, JSON qualification fixtures, Markdown current-state documentation.

## Global Constraints

- Local implementation baseline is `main` at HEAD `44cbce72a127960621f780bb90ab0478088b67f1`. The worktree was clean before this plan file was written.
- The approved design is `docs/superpowers/specs/2026-08-16-bios-recipe-qualification-design.md` at GitHub commit `0f39c0d612586482262b168558b4be7b3e3bb93e`. The local branch contains a local-only commit not present on GitHub, so do **not** pull, rebase, reset, merge, cherry-pick, or otherwise reconcile history as part of implementation. This plan is self-contained for execution.
- Roadmap provenance may say **Phase 6E.1** or **Phase 6E.2** in product/history prose. Active implementation artifacts must not use `phase`, `6e1`, `6e2`, `slice`, or equivalent roadmap numbering in module names, source filenames, fixture paths, test/helper/type names, implementation-facing product-document filenames, reusable APIs, or qualification-state values.
- Historical `.chatgpt/codex-runs/**` and completed historical planning/spec documents are provenance records and must not be rewritten merely to remove historical phase naming.
- `feature.copy_bios` is the sole new qualification target. Use `ayaneo.generic.base` only as real authored device-plan/device-profile context and explicitly select only `feature.copy_bios`; do not qualify the plan's default RetroArch + BIOS composition in this task.
- Preserve `app.retroarch.provision` qualification semantics exactly while renaming its active implementation artifacts and replacing the phase-coded cleanup-authority value with `not_authorized_for_recipe_qualification`.
- The current raw SHA-256 of `authored/recipes/feature.copy_bios.yaml` is `1a3b04aa3f26720701ccbe56336d1f451d3f402c9a092be10ef80682cd9a998b`. Treat the real authored file as source authority; do not copy the recipe under tests.
- `authored/recipes/feature.copy_bios.yaml`, `authored/device_plans/ayaneo.generic.base.yaml`, and `authored/device_profiles/ayaneo.generic.yaml` are read-only for this task. Do not change authored semantics to make qualification easier.
- Do not add or change dependencies, `Cargo.toml`, or `Cargo.lock`.
- Default expectation: no production executor/planner/review/protocol/API behavior changes. If a qualification test exposes a genuine production defect, stop treating it as test scaffolding: add the smallest coherent production correction only if it is required by an existing product/runtime contract, add focused regression coverage, and call it out explicitly in the result.
- The destination-verification negative test must execute the exact generated plan. Do not edit the plan, alter the authored destination, remove the verification condition, or manufacture a hand-built substitute plan.
- Use no ADB, `RealAdbDevice`, live public network, ignored physical tests, device cleanup/reset, host-sleep, identity-replacement, UI-smoke, packaged-GUI, signing, notarization, release, or operator/manual qualification.
- Phase 6D remains **In progress** with all current missing physical/UI evidence unchanged and owner-deferred, not waived. Phase 6E remains **In progress** after this task.
- Combined RetroArch + BIOS device-plan qualification remains explicitly unqualified and is the logical next automated recipe-qualification task.
- Preserve ordinary production real-execution gating and existing Phase 6C/6D safety, recovery, evidence, and sanitization semantics.
- Owner workflow overrides the writing-plans skill's default frequent-commit cadence: implement and verify only. Do **not** stage, commit, or push implementation changes; commit/push occurs only after owner review and an explicit closeout request.

---

## File Structure

### Behavior-preserving RetroArch naming migration

- Rename `crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs` to `crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs` — same qualification behavior, domain-stable module/test/helper naming.
- Rename `tests/fixtures/phase-6e/retroarch/` to `tests/fixtures/recipe-qualification/retroarch/` — same source-bound contract with domain-stable path and cleanup-authority value.
- Rename `docs/product/phase-6e1-recipe-qualification-foundation.md` to `docs/product/recipe-qualification-retroarch.md` — same current-state evidence and limits, with Phase 6E.1 retained only as roadmap provenance in prose.
- Modify `crates/emuchef-rust-backend/src/lib.rs` — register the renamed RetroArch test module and the new BIOS test module under `#[cfg(test)]` only.

### BIOS qualification

- Create `tests/fixtures/recipe-qualification/bios/qualification-contract.json` — strict source-bound expectations for the standalone BIOS workflow.
- Create `crates/emuchef-rust-backend/src/recipe_qualification_bios_tests.rs` — production-path planning/review/input validation, successful recursive copy qualification, and deterministic verification-failure qualification.
- Create `docs/product/recipe-qualification-bios.md` — current-state evidence, exact automated boundary, explicit physical deferral, and next combined-workflow boundary.

### Current-state documentation

- Modify `CONTEXT.md` — update RetroArch paths/naming and record BIOS automated qualification after gates pass.
- Modify `docs/product/product-roadmap.md` — record RetroArch + BIOS automated qualification while keeping Phase 6D/6E status truthful and combined-plan qualification pending.
- Update any other **current-state** reference to the renamed RetroArch active paths discovered during implementation. Do not rewrite historical Codex runs or completed historical plans/specs.

No Tauri/React file, authored YAML, production executor source, manifest, lockfile, Phase 6D evidence, or physical harness should need modification.

---

### Task 1: Migrate RetroArch qualification to domain-stable implementation names

**Files:**
- Rename: `crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs` -> `crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs`
- Rename: `tests/fixtures/phase-6e/retroarch/qualification-contract.json` -> `tests/fixtures/recipe-qualification/retroarch/qualification-contract.json`
- Rename: `docs/product/phase-6e1-recipe-qualification-foundation.md` -> `docs/product/recipe-qualification-retroarch.md`
- Modify: `crates/emuchef-rust-backend/src/lib.rs`
- Modify current-state references in: `CONTEXT.md`, `docs/product/product-roadmap.md`, and any directly linked current-state product doc discovered by search

**Interfaces:**
- Consumes: existing working RetroArch qualification contract/tests at local HEAD.
- Produces: `recipe_qualification_retroarch_tests` test module and `tests/fixtures/recipe-qualification/retroarch/qualification-contract.json` with unchanged behavioral qualification semantics.

- [ ] **Step 1: Capture the pre-migration RetroArch focused baseline**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml phase_6e1_ -- --nocapture
```

Record the observed pass/fail count. This is the behavior baseline that must remain unchanged by the naming migration.

- [ ] **Step 2: Move the three active artifacts without staging them**

Use normal filesystem moves, not `git mv`, because this task must leave the index untouched:

```bash
mkdir -p tests/fixtures/recipe-qualification/retroarch
mv tests/fixtures/phase-6e/retroarch/qualification-contract.json \
  tests/fixtures/recipe-qualification/retroarch/qualification-contract.json
rmdir tests/fixtures/phase-6e/retroarch
rmdir tests/fixtures/phase-6e 2>/dev/null || true
mv crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs \
  crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs
mv docs/product/phase-6e1-recipe-qualification-foundation.md \
  docs/product/recipe-qualification-retroarch.md
```

Do not touch historical `.chatgpt/codex-runs/**` or completed `docs/superpowers/**` provenance files during this migration.

- [ ] **Step 3: Update `lib.rs` and internal RetroArch identifiers**

Replace:

```rust
#[cfg(test)]
mod phase_6e1_recipe_qualification_tests;
```

with:

```rust
#[cfg(test)]
mod recipe_qualification_retroarch_tests;
```

In `recipe_qualification_retroarch_tests.rs`:

- change the module-level prose from Phase-coded implementation language to `RetroArch automated recipe qualification` language;
- change `contract_path()` to `tests/fixtures/recipe-qualification/retroarch/qualification-contract.json`;
- change fixture payload strings such as `phase-6e1\n` and `phase-6e1 deterministic apk fixture\n` to domain text such as `retroarch qualification fixture\n` and `retroarch deterministic apk fixture\n`;
- preserve all planner/review/executor assertions and fixture semantics.

Rename the seven active tests exactly as follows:

```text
phase_6e1_contract_binds_current_retroarch_source_and_deferred_physical_status
  -> retroarch_contract_binds_current_source_and_deferred_physical_status
phase_6e1_real_authored_retroarch_plan_matches_qualification_contract
  -> retroarch_real_authored_plan_matches_qualification_contract
phase_6e1_optional_retroarch_cfg_is_not_required_for_planning
  -> retroarch_optional_cfg_is_not_required_for_planning
phase_6e1_supplied_retroarch_cfg_is_bound_and_reviewed_without_parent_path_leakage
  -> retroarch_supplied_cfg_is_bound_and_reviewed_without_parent_path_leakage
phase_6e1_retroarch_generated_plan_executes_successfully_without_network_or_adb
  -> retroarch_generated_plan_executes_successfully_without_network_or_adb
phase_6e1_retroarch_install_skip_on_repeated_deterministic_run
  -> retroarch_install_skips_on_repeated_deterministic_run
phase_6e1_missing_core_system_verification_fails_and_stops_later_recipe_work
  -> retroarch_missing_core_system_verification_fails_and_stops_later_recipe_work
```

Do not rename ordinary domain helpers such as `plan_retroarch`, `QualificationWorkspace`, `seed_artifact_cache`, or `SystemFixtureMode` unless needed for clarity; they already satisfy the naming invariant.

- [ ] **Step 4: Replace the phase-coded cleanup authority without changing disposition**

In the moved RetroArch contract replace only:

```json
"physicalCleanupAuthority": "not_authorized_in_phase_6e1"
```

with:

```json
"physicalCleanupAuthority": "not_authorized_for_recipe_qualification"
```

Update `load_contract()` to assert the new exact value and use domain-stable failure text such as:

```rust
assert_eq!(
    contract.physical_cleanup_authority,
    "not_authorized_for_recipe_qualification",
    "RetroArch recipe qualification grants no physical cleanup authority"
);
```

This is a naming migration only: physical qualification remains deferred and no cleanup authority is granted.

- [ ] **Step 5: Update active product/current-state references to the moved paths**

Update the renamed RetroArch product document, `CONTEXT.md`, and product roadmap references so current-state documentation points to:

```text
crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs
tests/fixtures/recipe-qualification/retroarch/qualification-contract.json
docs/product/recipe-qualification-retroarch.md
```

Retain **Phase 6E.1** in prose only when it is useful roadmap provenance. Prefer `task`, `qualification`, or `workflow` over `slice` in newly edited prose.

- [ ] **Step 6: Prove the migration preserved behavior and removed active naming leakage**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml retroarch_ -- --nocapture
rg -n 'phase_6e1|phase-6e|not_authorized_in_phase_6e1' \
  crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs \
  tests/fixtures/recipe-qualification/retroarch \
  crates/emuchef-rust-backend/src/lib.rs
```

Expected:

- the same seven RetroArch qualification behaviors pass;
- the `rg` command returns no match in active implementation/fixture paths;
- no production API or behavior changed.

---

### Task 2: Add the strict source-bound BIOS contract and production planning/review qualification

**Files:**
- Create: `tests/fixtures/recipe-qualification/bios/qualification-contract.json`
- Create: `crates/emuchef-rust-backend/src/recipe_qualification_bios_tests.rs`
- Modify: `crates/emuchef-rust-backend/src/lib.rs`

**Interfaces:**
- Consumes: `CatalogSnapshot::legacy_local`, `runtime_configuration::plan_configuration`, `ConfigurationContextRequest`, `PlanConfigurationResult`, real authored `feature.copy_bios`, `ayaneo.generic.base`, `ayaneo.generic` capability defaults.
- Produces: `plan_bios(bios_dir: Option<&Path>) -> PlanConfigurationResult`, strict BIOS contract loader, production-plan/review/input-validation qualification tests reused by later executor tests.

- [ ] **Step 1: Create the BIOS contract with exact semantic expectations**

Create `tests/fixtures/recipe-qualification/bios/qualification-contract.json` with this exact initial shape:

```json
{
  "schemaVersion": 1,
  "targetRecipe": "feature.copy_bios",
  "planningDevicePlan": "ayaneo.generic.base",
  "authoredSource": {
    "path": "authored/recipes/feature.copy_bios.yaml",
    "sha256": "1a3b04aa3f26720701ccbe56336d1f451d3f402c9a092be10ef80682cd9a998b"
  },
  "selectedRecipes": ["feature.copy_bios"],
  "expandedRecipes": ["feature.copy_bios"],
  "recipeConstraintCapabilities": ["shared_storage_write"],
  "qualificationContextCapabilities": ["shared_storage_write"],
  "requiredInputs": ["feature.copy_bios/bios_source_dir"],
  "optionalInputs": [],
  "requiredOperationFamilies": ["copy_files"],
  "copyPolicy": "sync",
  "destination": "/sdcard/RetroArch/system",
  "verification": {
    "type": "path_exists",
    "path": "/sdcard/RetroArch/system"
  },
  "liveNetworkRequiredForAutomatedQualification": false,
  "automatedStatus": "qualified",
  "physicalStatus": "deferred",
  "physicalCleanupAuthority": "not_authorized_for_recipe_qualification"
}
```

The contract intentionally binds qualification semantics, not incidental serialized ordering or temporary paths.

- [ ] **Step 2: Register the BIOS module under `#[cfg(test)]` only**

Add beside the renamed RetroArch module:

```rust
#[cfg(test)]
mod recipe_qualification_bios_tests;
```

Do not make qualification code production-visible.

- [ ] **Step 3: Add strict contract types and source-provenance helpers**

In `recipe_qualification_bios_tests.rs`, define strict contract structs using:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BiosQualificationContract { /* exact contract fields */ }
```

Use domain constants:

```rust
const TARGET_RECIPE: &str = "feature.copy_bios";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.generic.base";
const BIOS_INPUT_KEY: &str = "feature.copy_bios/bios_source_dir";
const BIOS_DESTINATION: &str = "/sdcard/RetroArch/system";
```

Reuse the same safe repository-root/raw-SHA pattern as RetroArch, with:

```rust
fn contract_path() -> PathBuf {
    repository_root().join("tests/fixtures/recipe-qualification/bios/qualification-contract.json")
}
```

`load_contract()` must require schema version 1, exact target/device plan, `selectedRecipes == expandedRecipes == [feature.copy_bios]`, `automatedStatus == "qualified"`, `physicalStatus == "deferred"`, `physicalCleanupAuthority == "not_authorized_for_recipe_qualification"`, and `liveNetworkRequiredForAutomatedQualification == false`.

- [ ] **Step 4: Write the source-binding regression first**

Add:

```rust
#[test]
fn bios_contract_binds_current_source_and_deferred_physical_status() { /* ... */ }
```

It must:

- require a repository-relative source path;
- require exactly `authored/recipes/feature.copy_bios.yaml`;
- canonicalize and prove the source stays beneath repository root;
- hash raw bytes and compare exactly to the contract SHA;
- assert physical status/cleanup authority through `load_contract()`.

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  recipe_qualification_bios_tests::bios_contract_binds_current_source_and_deferred_physical_status \
  -- --exact --nocapture
```

Expected: PASS after the contract/module exists; any later authored-byte change fails closed until reviewed.

- [ ] **Step 5: Add the production-path planning helper**

Implement:

```rust
fn plan_bios(bios_dir: Option<&Path>) -> PlanConfigurationResult {
    use crate::catalog_source::CatalogSnapshot;
    use crate::model::OrderedMap;
    use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    let mut explicit_bindings = OrderedMap::new();
    if let Some(path) = bios_dir {
        explicit_bindings.insert(
            BIOS_INPUT_KEY.to_string(),
            Value::String(path.to_string_lossy().into_owned()),
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
    .expect("real authored BIOS configuration should prepare")
}
```

The explicit `selected_recipes` field is the invariant that prevents `ayaneo.generic.base` from adding its default RetroArch recipe to this qualification.

- [ ] **Step 6: Qualify the real authored plan and review with a valid BIOS directory**

Create a temporary parent plus `bios-source` directory and call `plan_bios(Some(&bios_dir))`. Add `bios_real_authored_plan_and_review_match_qualification_contract` asserting:

- no diagnostic has severity `error`;
- `plan`, `plan_digest`, and production `review` exist;
- selected and expanded recipe refs exactly equal `vec!["feature.copy_bios"]`;
- `device_plan_ref == "ayaneo.generic.base"` and `device_profile_ref == "ayaneo.generic"`;
- recipe snapshots contain exactly `feature.copy_bios`;
- there is exactly one generated target-recipe step ending in `/copy_bios_dir`;
- that step has `type_name == "copy_files"` and step constraint capabilities exactly `shared_storage_write` after deterministic sort/de-dup;
- `plan.runtime_capabilities.shared_storage_write == true`;
- resolved inputs contain exactly the required BIOS input with `BindingSource::Explicit` and the supplied directory value;
- the step's literal destination is `/sdcard/RetroArch/system`;
- the step's literal copy policy is `sync`;
- the step has exactly one `path_exists` verification for `/sdcard/RetroArch/system`;
- review is executable, has no blocker notice, contains one non-automatically-added BIOS feature, includes a `copies` section, and has `work.action_count == plan.steps.len()`;
- the serialized review may contain the BIOS directory basename but must not contain the temp parent directory or any ADB serial authority.

Use enum pattern matching on `ExecutionParamValue::Literal` rather than relying on full serialized-plan equality.

- [ ] **Step 7: Qualify missing and nonexistent required BIOS input failures**

Add two tests:

```rust
#[test]
fn bios_missing_required_input_prevents_executable_plan_and_review() { /* ... */ }

#[test]
fn bios_nonexistent_required_directory_prevents_executable_plan_and_review() { /* ... */ }
```

For the missing binding, call `plan_bios(None)` and assert:

- `plan.is_none()`;
- `plan_digest.is_none()`;
- `review.is_none()`;
- an error diagnostic exists with `code == "binding_missing"` and `key == Some(BIOS_INPUT_KEY.to_string())`;
- resolved input exists with no value/source.

For the nonexistent directory, create a tempdir but pass a child path that was **not** created. Assert:

- `plan`, `plan_digest`, and `review` are absent;
- an error diagnostic exists with `code == "binding_path_missing"`, matching BIOS key and `BindingSource::Explicit` provenance;
- no diagnostic serialization contains unrelated file contents or introduces a qualification-only API field.

Do not assert complete free-form diagnostic messages.

- [ ] **Step 8: Run all contract/planning/review/input tests under both backend configurations**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml bios_ -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution bios_ -- --nocapture
```

All tests added in Task 2 must pass without network or ADB.

---

### Task 3: Qualify successful nested BIOS copy through the normal sandbox executor

**Files:**
- Modify: `crates/emuchef-rust-backend/src/recipe_qualification_bios_tests.rs`

**Interfaces:**
- Consumes: `plan_bios`, `ExecutorAdapters::with_sandbox_roots`, `ExecutorRunner`, exact generated `ExecutionPlan`.
- Produces: deterministic success qualification proving recursive host-directory copy, authored `sync` policy, authored destination, and authored verification through the production dry-run path.

- [ ] **Step 1: Add a BIOS qualification workspace with representative nested files**

Define a test workspace with separate:

```text
runtime/
cache/
fake-device/
host-input/bios-source/
```

Create at least these nested files with deterministic contents:

```text
host-input/bios-source/sony/psx/scph5501.bin     -> "psx-bios\n"
host-input/bios-source/nintendo/gba/gba_bios.bin -> "gba-bios\n"
```

The read-only sandbox root must be `host-input`, not an unrestricted repository or temp root.

- [ ] **Step 2: Add a helper that builds the normal production-oriented dry-run adapters**

Use only:

```rust
ExecutorAdapters::with_sandbox_roots(
    workspace.runtime_root.clone(),
    workspace.cache_root.clone(),
    workspace.fake_device_root.clone(),
    vec![workspace.host_input_root.clone()],
)
```

Do not use a real device or a qualification-only execution engine.

- [ ] **Step 3: Add the successful execution test**

Add:

```rust
#[test]
fn bios_generated_plan_copies_nested_files_successfully_without_network_or_adb() { /* ... */ }
```

The test must:

1. call `plan_bios(Some(&workspace.bios_source_dir))`;
2. use the exact returned `ExecutionPlan` without mutation;
3. execute it through `ExecutorRunner::new(normal_bios_adapters(&workspace)).run(&plan)`;
4. assert `result.success == true`;
5. assert `result.total_steps == plan.steps.len() == 1` and the single `/copy_bios_dir` record is `StepRunStatus::Executed`;
6. assert no Failed, Blocked, or Cancelled record exists;
7. inspect the real fake-device sandbox tree and assert both nested files exist at:

```text
fake-device/sdcard/RetroArch/system/sony/psx/scph5501.bin
fake-device/sdcard/RetroArch/system/nintendo/gba/gba_bios.bin
```

8. assert their bytes equal the source bytes exactly;
9. reassert the generated step retains destination `/sdcard/RetroArch/system`, policy `sync`, and `path_exists` verification for the same destination.

This positive test is the proof of recursive directory copying. Do not replace it with a mock-only command assertion.

- [ ] **Step 4: Run the focused success test under both backend configurations**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  recipe_qualification_bios_tests::bios_generated_plan_copies_nested_files_successfully_without_network_or_adb \
  -- --exact --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution \
  recipe_qualification_bios_tests::bios_generated_plan_copies_nested_files_successfully_without_network_or_adb \
  -- --exact --nocapture
```

Both must pass.

---

### Task 4: Prove destination-verification failure without changing production code or the generated plan

**Files:**
- Modify: `crates/emuchef-rust-backend/src/recipe_qualification_bios_tests.rs`

**Interfaces:**
- Consumes: `FakeDryRunDevice`, `ExecutorDevice`, `DeviceOperationError`, `ExecutorAdapters::with_device_and_sandbox_roots`, exact generated BIOS plan.
- Produces: a BIOS-test-private `VerificationFailDevice` that delegates all operations to `FakeDryRunDevice` except the target `path_exists` result, plus a regression proving copy success followed by authored verification failure.

- [ ] **Step 1: Add a private wrapper rather than a production executor seam**

Define only in `recipe_qualification_bios_tests.rs`:

```rust
#[derive(Debug, Default)]
struct VerificationFailDevice {
    inner: FakeDryRunDevice,
    missing_path: String,
}

impl VerificationFailDevice {
    fn for_missing_path(path: &str) -> Self {
        Self {
            inner: FakeDryRunDevice::default(),
            missing_path: path.to_string(),
        }
    }

    fn commands(&self) -> &[Vec<String>] {
        self.inner.commands()
    }
}
```

Implement `ExecutorDevice` by delegating every required operation to `<FakeDryRunDevice as ExecutorDevice>::...` except:

```rust
fn uses_fake_device_filesystem(&self) -> bool {
    false
}

fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
    if path == self.missing_path {
        // Record the same observation through the inner fake before overriding
        // the deterministic predicate result.
        let _ = <FakeDryRunDevice as ExecutorDevice>::path_exists(&mut self.inner, path)?;
        return Ok(false);
    }
    <FakeDryRunDevice as ExecutorDevice>::path_exists(&mut self.inner, path)
}
```

Delegate `install_apk`, `push`, `mkdir_p`, `remove_file`, `remove_tree`, `copy_on_device`, `package_installed`, `path_is_dir`, `run_plan_command`, `launch_app`, and `force_stop_app` with their exact `ExecutorDevice` signatures. Default lifecycle hooks may remain defaults because the BIOS workflow requires neither identity nor root authority.

Returning `uses_fake_device_filesystem() == false` is deliberate for this wrapper: it makes the production `copy_files` path execute `mkdir_p`/`push` against the delegated fake device instead of creating the destination in the sandbox filesystem, so the subsequent production `path_exists` verification can deterministically observe the wrapper's false result. This wrapper is test-local and must not be added to `executor.rs`.

- [ ] **Step 2: Build adapters around the private wrapper**

Construct:

```rust
let device = VerificationFailDevice::for_missing_path(BIOS_DESTINATION);
let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
    device,
    workspace.runtime_root.clone(),
    workspace.cache_root.clone(),
    workspace.fake_device_root.clone(),
    vec![workspace.host_input_root.clone()],
);
```

No public API change is needed; `ExecutorAdapters` is already generic over `ExecutorDevice`.

- [ ] **Step 3: Add the unchanged-plan verification-failure regression**

Add:

```rust
#[test]
fn bios_destination_verification_failure_fails_the_unchanged_generated_plan() { /* ... */ }
```

The test must:

1. generate the plan with `plan_bios(Some(&workspace.bios_source_dir))`;
2. serialize or clone a snapshot of the plan before execution;
3. run it through the wrapper-backed production `ExecutorRunner`;
4. assert the plan equals the pre-run snapshot afterward, proving no test mutation;
5. assert `result.success == false`;
6. assert the `/copy_bios_dir` record is `StepRunStatus::Failed` with a non-empty message;
7. assert the wrapper's delegated command log contains `mkdir_p` for `/sdcard/RetroArch/system` and at least one `push` operation before the target `path_exists` observation, proving the copy operation ran rather than failing before execution;
8. assert the target `path_exists` command was observed;
9. assert no result path reports the BIOS copy step as `Executed` after failed verification.

Do not assert free-form failure text. The semantic contract is Failed step + unsuccessful run after an attempted copy and false authored verification.

- [ ] **Step 4: Prove no production executor change was needed**

Run:

```bash
git diff --name-only -- crates/emuchef-rust-backend/src/executor.rs
```

Expected: no output. If `executor.rs` is modified, treat that as a scope exception requiring a demonstrated production defect and explicit result rationale; do not leave a test-convenience seam in production code.

- [ ] **Step 5: Run the negative test under both backend configurations**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  recipe_qualification_bios_tests::bios_destination_verification_failure_fails_the_unchanged_generated_plan \
  -- --exact --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution \
  recipe_qualification_bios_tests::bios_destination_verification_failure_fails_the_unchanged_generated_plan \
  -- --exact --nocapture
```

Both must pass.

---

### Task 5: Record RetroArch and BIOS automated qualification truthfully

**Files:**
- Modify renamed: `docs/product/recipe-qualification-retroarch.md`
- Create: `docs/product/recipe-qualification-bios.md`
- Modify: `CONTEXT.md`
- Modify: `docs/product/product-roadmap.md`
- Modify only directly relevant current-state product docs if they contain stale active-path references discovered by search

**Interfaces:**
- Consumes: verified automated results from Tasks 1–4.
- Produces: current-state documentation that distinguishes automated deterministic qualification from physical/end-to-end qualification and identifies combined RetroArch + BIOS as still unqualified.

- [ ] **Step 1: Normalize the renamed RetroArch product document without changing claims**

In `docs/product/recipe-qualification-retroarch.md`:

- retain Phase 6E.1 only as roadmap/history provenance;
- point to `tests/fixtures/recipe-qualification/retroarch/qualification-contract.json`;
- describe cleanup authority as `not_authorized_for_recipe_qualification`;
- retain the exact automated behaviors already proven: real authored catalog/planning/review, deterministic cache/execution, repeated install skip, authored verification failure;
- retain the exact exclusions: no real network-service proof, APK acceptance, Android permission/app-op semantics, hardware storage behavior, actual launch, cleanup/reset, packaged GUI, or physical end-to-end qualification;
- remove implementation-facing language that calls the module/path a phase or slice.

- [ ] **Step 2: Create the BIOS qualification product document**

Create `docs/product/recipe-qualification-bios.md` with these sections and facts:

1. **Status and authority** — roadmap provenance Phase 6E.2; owner EmuChef proper / Shared Runtime; automated BIOS qualification complete only after all gates pass.
2. **Qualified automated boundary** — exact authored SHA-bound contract, real `ayaneo.generic.base` capability context, explicit BIOS-only selection, production planning/review, deterministic executor.
3. **What the tests prove** — required input validation, clean valid review, exact `sync` copy semantics and destination, recursive nested copy, successful authored `path_exists`, and forced authored verification failure after copy attempt.
4. **What the tests do not prove** — real ADB/device storage permissions/writability/performance, device cleanup, packaged GUI, physical end-to-end success, or combined RetroArch + BIOS behavior.
5. **Physical disposition** — `deferred`; cleanup authority `not_authorized_for_recipe_qualification`.
6. **Next automated qualification** — combined RetroArch + BIOS through a real default device-plan selection remains pending and must not be claimed here.

- [ ] **Step 3: Update `CONTEXT.md`**

Update the current recipe-qualification section so it records both domain paths and both automated states:

- RetroArch qualification remains complete with the renamed contract/test/doc paths;
- BIOS qualification is complete through source-bound real catalog/runtime configuration, production review, required-input rejection, nested deterministic copy, and verification-failure coverage;
- Phase 6E remains `In progress` because combined workflows and other recipe qualifications remain outstanding;
- Phase 6D remains `In progress` with all deferred evidence requirements unchanged;
- manual/physical qualification remains deferred by owner decision, not waived.

- [ ] **Step 4: Update the product roadmap**

In `docs/product/product-roadmap.md`:

- retain the Phase 6D row/status and its exact missing repetition list;
- update the current EmuChef proper status to include automated RetroArch and BIOS qualification;
- update Phase 6E primary outcome to state that standalone RetroArch and BIOS automated qualification are complete while physical/end-to-end qualification remains deferred;
- set next priority to the combined RetroArch + BIOS automated device-plan qualification, not Obtainium/ROMs and not deferred physical work;
- leave Phase 6F/6G unchanged.

- [ ] **Step 5: Search for stale active references and contradictory current-state claims**

Run a bounded search over current-state code/fixtures/product docs:

```bash
rg -n \
  'phase_6e1_recipe_qualification_tests|tests/fixtures/phase-6e/retroarch|phase-6e1-recipe-qualification-foundation|not_authorized_in_phase_6e1' \
  crates/emuchef-rust-backend/src tests/fixtures CONTEXT.md docs/product
```

Expected: no stale active reference remains. Historical `.chatgpt/codex-runs/**` and completed `docs/superpowers/**` files are intentionally outside this check.

Also search current-state docs for contradictory Phase 6E wording such as `Phase 6E.*Planned`, `not started`, or a next-priority statement that still asks to choose the next standalone recipe.

---

### Task 6: Run the full automated gate matrix and enforce scope/naming boundaries

**Files:**
- No additional files unless an approved test exposes an in-scope production defect.

**Interfaces:**
- Consumes: all implementation/documentation changes.
- Produces: verification evidence for Codex `RESULT.json` and owner review; leaves implementation unstaged and uncommitted.

- [ ] **Step 1: Run formatting and compile checks**

Run:

```bash
cargo fmt --all --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --check
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --no-default-features
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
```

All must exit 0.

- [ ] **Step 2: Run all domain-named recipe qualification tests under both configurations**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml recipe_qualification_ -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution recipe_qualification_ -- --nocapture
```

If Rust's test filter matches module paths differently, use `cargo test -- --list` to select the domain-named RetroArch and BIOS qualification tests explicitly. Do not fall back to phase-coded test filters.

- [ ] **Step 3: Run the complete backend tests under both configurations**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
```

Record observed pass/fail/ignored counts from the actual run; do not copy historical counts.

- [ ] **Step 4: Run strict backend Clippy**

Run:

```bash
cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets --all-features -- -D warnings
```

Do not weaken lint settings or add broad suppressions.

- [ ] **Step 5: Validate both real authored recipes through the existing CLI validator**

Run:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- validate --authored-root authored authored/recipes/app.retroarch.provision.yaml
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- validate --authored-root authored authored/recipes/feature.copy_bios.yaml
```

Both must complete without errors. Review and report any warnings.

- [ ] **Step 6: Re-run the existing Phase 6D.6 evidence validator/tests without regenerating evidence**

Run:

```bash
node tools/phase-6d6-evidence.mjs
node --test tools/phase-6d6-evidence.test.mjs
```

Expected disposition: the evidence contract remains valid but truthfully incomplete with the same owner-deferred missing repetitions. Do not alter evidence, traces, UI captures, scenario manifest, schema, or binding index.

- [ ] **Step 7: Enforce the implementation naming invariant**

Run:

```bash
find crates/emuchef-rust-backend/src tests/fixtures/recipe-qualification \
  \( -name '*phase*6e*' -o -name '*6e1*' -o -name '*6e2*' -o -name '*slice*' \) -print
printf '%s\n' \
  docs/product/recipe-qualification-retroarch.md \
  docs/product/recipe-qualification-bios.md | \
  rg 'phase.*6e|6e1|6e2|slice' || true
rg -n 'phase_6e1|phase_6e2|not_authorized_in_phase_6e1|not_authorized_in_phase_6e2' \
  crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs \
  crates/emuchef-rust-backend/src/recipe_qualification_bios_tests.rs \
  tests/fixtures/recipe-qualification
```

Expected: the `find` command prints no active qualification implementation/fixture path, the two explicit product-document filenames do not contain roadmap numbering, and the final `rg` returns no implementation/state-value matches. Existing unrelated historical/product roadmap filenames elsewhere in `docs/product/` are outside this naming migration.

- [ ] **Step 8: Run final Git boundary checks**

Run:

```bash
git diff --check
git status --short --branch
git diff --name-only
git diff --cached --name-only
```

Required final state:

- the implementation plan itself remains the pre-existing planning change created before Codex delegation;
- only authorized RetroArch rename/reference changes, new BIOS qualification artifacts, current-state docs, and that plan file are modified/untracked;
- `authored/**` bytes are unchanged;
- `crates/emuchef-rust-backend/src/executor.rs` is unchanged unless a separately justified production defect was found;
- no `apps/**`, `Cargo.toml`, `Cargo.lock`, Phase 6D evidence/schema/manifest/index, or historical Codex-run bytes changed;
- `git diff --cached --name-only` is empty;
- no commit or push occurred.

- [ ] **Step 9: Write the Codex result with explicit provenance and limits**

The result must state together:

- RetroArch active qualification artifacts were migrated to domain naming with behavior preserved;
- BIOS standalone automated qualification is source-bound and covers production planning/review/input validation, nested successful copy, and unchanged-plan verification failure;
- the exact source SHA checked by the BIOS contract;
- no ADB, live network, physical/manual/ignored qualification, cleanup, or packaged-GUI work ran;
- no production executor change was required, or if one was required, identify the failing contract/regression and smallest fix explicitly;
- Phase 6D remains **In progress** and owner-deferred evidence is unwaived;
- Phase 6E remains **In progress**;
- combined RetroArch + BIOS remains unqualified and is the next logical automated workflow;
- all actual validation commands and observed results/counts;
- nothing was staged, committed, or pushed.

---

## Self-Review Checklist for the Implementer

Before reporting completion, verify all of the following:

- Every requirement in the approved BIOS qualification design maps to a concrete test, migration step, documentation update, or explicit exclusion above.
- RetroArch qualification behavior did not change while module/file/test/fixture/product-doc naming became domain-based.
- No active RetroArch or BIOS qualification implementation identifier/path/state value leaks phase/slice nomenclature.
- The BIOS contract digest matches the raw current authored source bytes and fails closed on change.
- `plan_bios` uses the real authored catalog and `runtime_configuration::plan_configuration` with `ayaneo.generic.base` context and explicit BIOS-only selection.
- Missing required input is rejected with `binding_missing`; nonexistent required directory is rejected with `binding_path_missing`; neither produces an executable plan/review.
- Review assertions use `PlanConfigurationResult.review`, not a test-only projection.
- The successful executor test uses the exact generated plan and normal `with_sandbox_roots`, and verifies actual nested files in the fake-device sandbox.
- The failure executor test uses the exact generated plan plus only the private wrapper's predicate override; no production executor seam exists for test convenience.
- Authored YAML, device plan/profile, public/serialized APIs, manifests, and lockfile remain unchanged.
- Phase 6D evidence and closure requirements remain unchanged and truthful.
- Combined RetroArch + BIOS behavior is not claimed as qualified.
- No TODO/TBD/placeholders remain in new qualification artifacts or product docs.
- Nothing is staged, committed, or pushed.
