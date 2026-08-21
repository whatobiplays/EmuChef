# Phase 6E.1 Recipe Qualification Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish automated, source-bound qualification of the real `app.retroarch.provision` workflow through production catalog loading, planning, review projection, and deterministic executor execution, without ADB, live network dependence, or any manual/physical qualification.

**Architecture:** Add one focused crate-root test module plus one strict JSON qualification contract. The tests load the real authored catalog through `runtime_configuration::plan_configuration`, bind the contract to the current RetroArch source SHA-256, pre-seed the production artifact cache with deterministic tiny fixtures, and execute the unchanged generated plan through `ExecutorAdapters::with_sandbox_roots`. No physical harness is added in this slice because the approved automated architecture can be completed without it; future physical harness work stays deferred until the owner explicitly resumes manual/physical qualification.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, `sha2`, `zip`, `tempfile`, existing EmuChef planner/runtime configuration/review projection/executor APIs, Markdown current-state docs.

## Global Constraints

- Start from `main` at HEAD `f0438960e1b957a67855d60ca97f665c8f91f0ba`; preserve the pre-existing unstaged deletion `.serena/memories/memory_maintenance.md` exactly and never stage, restore, or modify it.
- The approved design authority is `docs/superpowers/specs/2026-08-15-phase-6e1-recipe-qualification-foundation-design.md`.
- All manual and physical qualification is deferred until the owner explicitly says otherwise. Do not run ignored tests, ADB/device commands, host-sleep, identity-replacement, UI-smoke, packaged-GUI, or other operator qualification.
- Phase 6D remains **In progress**. Starting Phase 6E does not waive or reinterpret any Phase 6D evidence or closure requirement.
- Phase 6E becomes **In progress** only after the automated foundation in this plan is implemented and verified.
- `app.retroarch.provision` is the sole Phase 6E.1 qualification target. Do not qualify Obtainium, BIOS copy, ROM copy, Xaniteog, Daijisho, ES-DE, or combined device-plan workflows in this slice.
- Use the real `authored/` corpus as source authority. Do not create a second copy of the RetroArch recipe under tests.
- Do not require public network access for any deterministic qualification test.
- Do not add a qualification-only planner, review DTO, executor, public protocol field, frontend API, or serialized product contract.
- Preserve Phase 6D timeout, transport, identity, root, partial-result, no-resume/no-replay, sanitization, and active-execution semantics.
- Do not edit accepted Phase 6D evidence, traces, scenario manifest, evidence schema, or historical Codex results.
- Do not broadly refactor planner, executor, artifact resolution, or Phase 6C/6D harnesses.
- `authored/recipes/app.retroarch.provision.yaml` may change only if a new Phase 6E.1 regression proves a genuine violation of an already documented product/runtime contract. Do not rewrite authored behavior to make qualification easier.
- Do not stage, commit, or push implementation changes in this task; the owner reviews implementation before closeout.

---

## File Structure

- Create `tests/fixtures/phase-6e/retroarch/qualification-contract.json` — strict source-bound expectations and explicit automated/physical disposition for the first recipe qualification target.
- Create `crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs` — all Phase 6E.1 contract loading, planning/review assertions, deterministic artifact-cache fixture generation, executor success/skip/failure qualification tests.
- Modify `crates/emuchef-rust-backend/src/lib.rs` — register only the new `#[cfg(test)]` module.
- Create `docs/product/phase-6e1-recipe-qualification-foundation.md` — current-state evidence and limits for the automated foundation.
- Modify `docs/product/product-roadmap.md` — record the owner sequencing override, Phase 6E `In progress`, automated RetroArch foundation, and continued manual/physical deferral.
- Modify `docs/product/phase-6d6-physical-interruption-qualification.md` — record only the owner-level deferral of remaining manual/physical qualification; retain every existing Phase 6D closure criterion and missing-evidence statement.
- Modify `CONTEXT.md` — add the concise current-state Phase 6E.1 automated qualification contract and preserve Phase 6D as open.

No Tauri or React file should need modification. No physical Phase 6E harness should be created in this slice.

---

### Task 1: Add the strict source-bound RetroArch qualification contract

**Files:**
- Create: `tests/fixtures/phase-6e/retroarch/qualification-contract.json`
- Create: `crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs`
- Modify: `crates/emuchef-rust-backend/src/lib.rs`

**Interfaces:**
- Consumes: real source `authored/recipes/app.retroarch.provision.yaml`, current SHA-256 `d3fb4fc56064377e1d8e6954e0ac0aa3fc79d2e51d22e59ab00e0bbad821b2fa`, device-plan context `ayaneo.konkr_pocket_fit.base`.
- Produces: crate-private test helper `load_contract() -> RetroArchQualificationContract`, `repository_root() -> PathBuf`, and `authored_root() -> PathBuf` used by later tasks.

- [ ] **Step 1: Create the contract with exact initial expectations**

Use this semantic shape; field names must remain `camelCase` and the Rust struct must use `#[serde(rename_all = "camelCase", deny_unknown_fields)]` so stale/unreviewed contract fields fail closed:

```json
{
  "schemaVersion": 1,
  "targetRecipe": "app.retroarch.provision",
  "planningDevicePlan": "ayaneo.konkr_pocket_fit.base",
  "authoredSource": {
    "path": "authored/recipes/app.retroarch.provision.yaml",
    "sha256": "d3fb4fc56064377e1d8e6954e0ac0aa3fc79d2e51d22e59ab00e0bbad821b2fa"
  },
  "selectedRecipes": ["app.retroarch.provision"],
  "expandedRecipes": ["app.retroarch.provision"],
  "recipeConstraintCapabilities": ["apk_install", "app_launch", "app_data_write"],
  "qualificationContextCapabilities": [
    "adb_available",
    "apk_install",
    "shared_storage_write",
    "app_launch",
    "shell_command",
    "root_shell",
    "app_data_write"
  ],
  "requiredInputs": [],
  "optionalInputs": ["app.retroarch.provision/retroarch_cfg"],
  "requiredOperationFamilies": [
    "resolve_artifacts",
    "install_apk",
    "launch_app",
    "wait",
    "force_stop_app",
    "grant_permissions",
    "extract_archive",
    "extract_artifacts",
    "copy_files"
  ],
  "materialDependencyEdges": [
    ["resolve_artifacts", "install_retroarch"],
    ["install_retroarch", "launch_retroarch_bootstrap"],
    ["launch_retroarch_bootstrap", "wait_for_retroarch_bootstrap"],
    ["wait_for_retroarch_bootstrap", "stop_retroarch_after_bootstrap"],
    ["stop_retroarch_after_bootstrap", "grant_retroarch_permissions"],
    ["grant_retroarch_permissions", "launch_retroarch_permissions"],
    ["launch_retroarch_permissions", "wait_for_retroarch_permissions"],
    ["wait_for_retroarch_permissions", "stop_retroarch_after_permissions"],
    ["extract_core_system_files", "copy_core_system_files"],
    ["copy_core_system_files", "launch_retroarch"]
  ],
  "liveNetworkRequiredForAutomatedQualification": false,
  "automatedStatus": "foundation",
  "physicalStatus": "deferred",
  "physicalCleanupAuthority": "not_authorized_in_phase_6e1"
}
```

The `qualificationContextCapabilities` intentionally describes the rooted Pocket FIT deterministic context used to exercise the recipe's rooted app-op branch; `package_remove_for_user` remains false and therefore is intentionally absent.

- [ ] **Step 2: Register the test module only under `#[cfg(test)]`**

Add beside the existing `executor_tests`/`planner_tests` registrations in `lib.rs`:

```rust
#[cfg(test)]
mod phase_6e1_recipe_qualification_tests;
```

Do not expose any new production module or public API.

- [ ] **Step 3: Add strict contract types and provenance helpers**

In the new test module, define typed contract structs for the JSON above and helpers equivalent to:

```rust
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const TARGET_RECIPE: &str = "app.retroarch.provision";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.konkr_pocket_fit.base";

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
    repository_root().join("tests/fixtures/phase-6e/retroarch/qualification-contract.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
```

`load_contract()` must deserialize with `deny_unknown_fields`, require `schemaVersion == 1`, exact target/device-plan identity, `automatedStatus == "foundation"`, `physicalStatus == "deferred"`, `physicalCleanupAuthority == "not_authorized_in_phase_6e1"`, and `liveNetworkRequiredForAutomatedQualification == false`.

- [ ] **Step 4: Write the provenance regression first**

Add `phase_6e1_contract_binds_current_retroarch_source_and_deferred_physical_status`. It must read `contract.authored_source.path` from repository root, hash the raw file bytes, and assert exact equality with the contract digest. Also assert the contract source path resolves to `authored/recipes/app.retroarch.provision.yaml` and does not escape the repo root.

Expected initial result before the contract/module exists: compilation/test discovery failure. After Steps 1–3, the test passes and future authored changes fail until expectations are deliberately reviewed.

- [ ] **Step 5: Run the focused contract test**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml phase_6e1_recipe_qualification_tests::phase_6e1_contract_binds_current_retroarch_source_and_deferred_physical_status -- --exact --nocapture
```

The command is intentionally module-qualified so `--exact` runs one test and cannot silently match zero or multiple Phase 6E.1 tests.

---

### Task 2: Qualify real authored planning and production review projection

**Files:**
- Modify: `crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs`

**Interfaces:**
- Consumes: `CatalogSnapshot::legacy_local`, `runtime_configuration::ConfigurationContextRequest`, `runtime_configuration::plan_configuration`, `model::OrderedMap`, the Task 1 contract.
- Produces: `plan_retroarch(config_path: Option<&Path>) -> PlanConfigurationResult`, plus semantic plan/review assertions reused by executor tests.

- [ ] **Step 1: Add the production-path planning helper**

Build the request from the real catalog rather than `PlannerInput::from_authored_root` so the qualification covers runtime configuration and production review creation in one path:

```rust
fn plan_retroarch(config_path: Option<&Path>) -> crate::runtime_configuration::PlanConfigurationResult {
    use crate::catalog_source::CatalogSnapshot;
    use crate::model::OrderedMap;
    use crate::runtime_configuration::{
        plan_configuration, ConfigurationContextRequest,
    };

    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    let mut explicit_bindings = OrderedMap::new();
    if let Some(path) = config_path {
        explicit_bindings.insert(
            "app.retroarch.provision/retroarch_cfg".to_string(),
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
    .expect("real authored RetroArch configuration should prepare")
}
```

Do not set a target serial and do not invoke any device-probe API.

- [ ] **Step 2: Add semantic plan-contract assertions**

Add `phase_6e1_real_authored_retroarch_plan_matches_qualification_contract`. Assert:

- no `error` diagnostics;
- a plan and plan digest exist;
- `plan.source.selected_recipe_refs` and `expanded_recipe_refs` equal the contract exactly;
- `plan.source.device_plan_ref == "ayaneo.konkr_pocket_fit.base"` and `device_profile_ref == "ayaneo.konkr_pocket_fit"`;
- plan recipe snapshots contain only `app.retroarch.provision`;
- the operation-family set derived from `plan.steps[*].type_name` contains every `requiredOperationFamilies` value;
- the capability set required by emitted step constraints contains exactly the contract's `recipeConstraintCapabilities` after de-duplication/sort;
- the plan runtime capabilities have every contract `qualificationContextCapabilities` entry enabled and `package_remove_for_user == false`;
- the contract's `requiredInputs` is empty and its `optionalInputs` equals the resolved target input identities, with `app.retroarch.provision/retroarch_cfg` represented as optional;
- every material dependency edge resolves by authored step suffix within the same recipe, and the later step directly lists the earlier generated step ID where the authored contract declares a direct dependency.

Use a helper that maps an authored step ID to exactly one generated step whose `recipe_ref == TARGET_RECIPE` and whose `id` ends in `/{authored_step_id}`. Fail on zero or multiple matches rather than guessing prefixes.

- [ ] **Step 3: Qualify optional input omission and supplied input**

Create two tests:

1. `phase_6e1_optional_retroarch_cfg_is_not_required_for_planning` — call `plan_retroarch(None)`, assert the resolved input exists with `value == None`/null and planning still succeeds; assert no emitted step retains an unresolved reference that makes the plan invalid.
2. `phase_6e1_supplied_retroarch_cfg_is_bound_and_reviewed_without_parent_path_leakage` — create a temp `retroarch.cfg`, plan with it, assert the resolved binding source is explicit and the generated `seed_retroarch_cfg` copy step is present. The review input summary may contain `retroarch.cfg` but must not contain the temp parent directory.

Do not hard-code the temp directory in expected output.

- [ ] **Step 4: Qualify the production review projection**

In the supplied-config test, unwrap `result.review` and assert:

- `can_execute == true`;
- one feature represents the real authored RetroArch recipe and is not marked automatically added;
- the section-kind set includes `preparation`, `downloads`, `installs`, `copies`, `permissions`, `launches`, and `device_changes`;
- `work.action_count == plan.steps.len()`;
- known waits equal the authored 1.5 s + 5 s total rounded by production projection (`known_wait_seconds == Some(7)`);
- no review notice has severity `blocker`;
- serialized review text does not contain the full config parent directory, any ADB serial field, or a qualification-only field.

This test must use the `review` returned by `plan_configuration`; do not call a second test-only projection implementation.

- [ ] **Step 5: Run focused planning/review tests**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml phase_6e1_ -- --nocapture
```

At this checkpoint, all Phase 6E.1 tests added so far must pass without network or ADB.

---

### Task 3: Execute the unchanged generated workflow deterministically with the production dry-run adapters

**Files:**
- Modify: `crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs`

**Interfaces:**
- Consumes: `artifact_resolver::artifact_local_filename`, generated `ExecutionPlan`, `ExecutorAdapters::with_sandbox_roots`, `ExecutorRunner`, `zip::ZipWriter`.
- Produces: deterministic cache seeding helpers and a success/idempotent-skip executor qualification that performs zero public network requests.

- [ ] **Step 1: Add deterministic cache seeding helpers**

The production resolver treats an existing regular default-cache file as a cache hit before performing HTTP. Seed every `plan.artifacts` entry at the exact production cache filename:

```rust
fn artifact_cache_path(cache_root: &Path, artifact: &crate::planner::ExecutionArtifact) -> PathBuf {
    cache_root.join(crate::artifact_resolver::artifact_local_filename(
        &artifact.id,
        &artifact.url,
        &artifact.cache,
    ))
}
```

For `retroarch_apk`, write small non-empty deterministic bytes. For every ZIP artifact, create a valid ZIP with `zip::ZipWriter`; do not copy the real remote payloads into the repo.

Use these minimum entries so the authored verification paths can succeed after extraction/copy:

- `core_files_dolphin_zip` -> `dolphin-emu/marker.txt`
- `core_files_fbneo_zip` -> `fbneo/marker.txt`
- `core_files_ppsspp_zip` -> `PPSSPP/marker.txt`
- all other ZIP artifacts -> `marker.txt`

A helper equivalent to this is sufficient:

```rust
fn write_zip(path: &Path, entries: &[&str]) {
    let file = fs::File::create(path).expect("cache zip should be created");
    let mut zip = zip::ZipWriter::new(file);
    for entry in entries {
        zip.start_file(*entry, zip::write::SimpleFileOptions::default())
            .expect("zip entry should start");
        zip.write_all(b"phase-6e1\n").expect("zip entry should write");
    }
    zip.finish().expect("zip should finish");
}
```

Assert every authored artifact uses `cache == "default"` in this qualification. If that authored contract changes, fail and review the deterministic boundary rather than silently performing HTTP.

- [ ] **Step 2: Add a dry-run workspace helper**

Create a temp workspace with separate `runtime`, `cache`, `fake-device`, and host-input paths. Create `retroarch.cfg` under a dedicated host-input directory. Plan with that file, seed the cache, then build:

```rust
let adapters = crate::executor::ExecutorAdapters::with_sandbox_roots(
    runtime_root,
    cache_root,
    fake_device_root,
    vec![host_input_root],
);
let mut runner = crate::executor::ExecutorRunner::new(adapters);
let result = runner.run(&plan);
```

Do not use `RealAdbDevice`, `with_device_and_sandbox_roots`, `Command`, `adb`, environment opt-ins, or HTTP test servers.

- [ ] **Step 3: Add full deterministic success qualification**

Add `phase_6e1_retroarch_generated_plan_executes_successfully_without_network_or_adb` and assert:

- `result.success == true`;
- `result.total_steps == plan.steps.len()`;
- `result.steps.len() == result.total_steps`;
- no step is `Failed`, `Blocked`, or `Cancelled`;
- the generated `resolve_artifacts`, `install_retroarch`, `grant_retroarch_permissions`, `copy_core_system_files`, `seed_retroarch_cfg`, and final `launch_retroarch` records are present;
- `copy_core_system_files` reaches `Executed`, proving the three authored verification paths are present in the fake device tree;
- `seed_retroarch_cfg` reaches `Executed` and its verification passes;
- final `launch_retroarch` reaches `Executed`.

Do not assert incidental temporary filenames or full serialized result equality.

- [ ] **Step 4: Prove the authored install skip predicate on a repeated deterministic run**

Reuse the same fake-device root after the first successful run, construct a fresh `ExecutorRunner` with the same runtime/cache/fake-device roots, and execute the same exact plan again. Assert the `install_retroarch` record is `StepRunStatus::Skipped` due to the production `package_installed` predicate while the overall run remains successful.

If the production dry-run adapter intentionally clears package state when a runner is recreated, keep one runner and call `run` twice instead; do not add test-only package-state mutation merely to force the skip.

- [ ] **Step 5: Run focused executor success/skip tests**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml phase_6e1_retroarch_generated_plan_executes -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml phase_6e1_retroarch_install_skip -- --nocapture
```

If the final test name differs, preserve the semantic assertion and use the actual test name from `--list`.

---

### Task 4: Prove deterministic verification failure and fail-stop reporting

**Files:**
- Modify: `crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs`

**Interfaces:**
- Consumes: Task 3 cache fixture builder and production executor result/status types.
- Produces: one selected deterministic failure regression proving recipe-level verification and Phase 6D fail-stop accounting without a device.

- [ ] **Step 1: Parameterize the system-file fixture only**

Add a fixture mode such as:

```rust
#[derive(Clone, Copy)]
enum SystemFixtureMode {
    Complete,
    MissingPpsspp,
}
```

`Complete` emits all three expected top-level directories. `MissingPpsspp` emits `dolphin-emu/` and `fbneo/` but intentionally omits `PPSSPP/`. Every other artifact remains valid and cache-hit. This must alter only deterministic fixture bytes, never the generated plan.

- [ ] **Step 2: Add the verification-failure regression**

Add `phase_6e1_missing_core_system_verification_fails_and_stops_later_recipe_work`. Plan the real recipe with a supplied config, seed `MissingPpsspp`, and run the unchanged plan through the same production dry-run adapters.

Assert:

- `result.success == false`;
- the `copy_core_system_files` step record is `StepRunStatus::Failed` because its authored `path_exists` verification for `/storage/emulated/0/RetroArch/system/PPSSPP` fails;
- the record message is present but the test does not depend on free-form wording;
- the final `launch_retroarch` step is not reported as successfully executed after the failure;
- `result.steps.len() < result.total_steps` or later records use the production non-success status consistent with the existing executor contract; do not invent a new status to represent Not attempted;
- previously completed records remain retained, proving truthful partial-result accounting.

The exact count of not-run work should be derived from the actual plan/result rather than frozen into the contract unless current executor semantics expose a stable count that the test can compute from step positions.

- [ ] **Step 3: Preserve existing operation-failure coverage rather than duplicating it**

Do not add a malformed-archive or second synthetic operation-failure fixture in Phase 6E.1. Existing executor regressions remain the authority for generic operation-failure mechanics; the new recipe-level failure obligation is specifically the authored `copy_core_system_files` verification failure above. In the Codex result, cite the existing executor test(s) that cover generic operation failure after inspecting the current suite, without modifying those tests solely for Phase 6E.1.

- [ ] **Step 4: Run all Phase 6E.1 tests under both backend feature configurations**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml phase_6e1_ -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution phase_6e1_ -- --nocapture
```

Both runs must execute only non-ignored deterministic tests.

---

### Task 5: Record Phase 6E.1 current state without weakening Phase 6D

**Files:**
- Create: `docs/product/phase-6e1-recipe-qualification-foundation.md`
- Modify: `docs/product/product-roadmap.md`
- Modify: `docs/product/phase-6d6-physical-interruption-qualification.md`
- Modify: `CONTEXT.md`

**Interfaces:**
- Consumes: verified automated test results from Tasks 1–4.
- Produces: authoritative current-state wording: Phase 6D still open/deferred, Phase 6E now in progress, RetroArch automated foundation complete, physical qualification unclaimed.

- [ ] **Step 1: Create the focused Phase 6E.1 product document**

Use these sections and facts:

1. **Status and authority** — Owner EmuChef proper / Shared Runtime; Phase 6E.1 automated foundation complete only after tests pass.
2. **Qualified automated boundary** — real authored source digest binding, real catalog/runtime-configuration planner, production review projection, deterministic production executor dry-run adapters, cache-hit artifact fixtures.
3. **What the tests prove** — recipe admission, expansion, capability context, optional/supplied config behavior, material dependencies, review sections/waits/sanitization, artifact resolution path without live network, successful execution, install skip, verification failure/fail-stop reporting.
4. **What they do not prove** — no real download service availability, APK acceptance, Android permission/app-op behavior, private/shared-storage semantics on hardware, actual launches, device cleanup/reset, packaged GUI, or physical end-to-end success.
5. **Physical qualification disposition** — explicitly `Deferred by owner`; no physical cleanup authority is granted by Phase 6E.1.
6. **Next automated recipe slices** — Obtainium, BIOS, ROM/content, or combined workflows may be separately promoted later; do not mark them started here.

Do not use “end-to-end qualified” without the word “not” or “deferred” when referring to physical product behavior.

- [ ] **Step 2: Update the roadmap sequencing truthfully**

In `docs/product/product-roadmap.md`:

- keep Phase 6D `In progress` and retain the exact missing `identity_replacement`, both host-sleep, and `ui_smoke_composite` repetitions;
- state that all remaining manual/physical qualification is owner-deferred until explicitly resumed;
- change Phase 6E from `Planned` to `In progress` only after the automated tests are green;
- record Phase 6E.1 RetroArch automated qualification foundation as completed automated work, with physical/end-to-end qualification deferred;
- make the next priority an automated Phase 6E follow-on selection rather than collecting physical evidence while the deferral is active;
- do not mark Phase 6D completed and do not change Phase 6F/6G status.

- [ ] **Step 3: Update the Phase 6D.6 document only for the owner deferral**

Add concise current-disposition wording that the remaining manual/physical Phase 6D.6 work is intentionally deferred by owner decision. Preserve every existing missing-evidence list, validator authority statement, closure requirement, and ordinary-production disablement.

Do not edit any Phase 6D evidence or validator contract to make the deferral look like completion.

- [ ] **Step 4: Update `CONTEXT.md`**

Add a focused Phase 6E.1 section or paragraph stating:

- source-bound qualification target and contract path;
- production planning/review/executor dry-run path;
- deterministic cache pre-seeding means no live network is required;
- Phase 6E is `In progress` for automated work;
- RetroArch physical/end-to-end qualification remains deferred;
- Phase 6D remains `In progress` with unchanged missing evidence.

Keep rationale with decisions: Phase 6E is allowed to proceed because the owner explicitly deferred remaining manual/physical work, not because Phase 6D requirements were reduced.

- [ ] **Step 5: Check documentation for contradictory status wording**

Search current-state docs for `Phase 6E` plus `Planned`, `not started`, and `Do not begin Phase 6E`. Historical Codex prompts/results may retain their historical wording; current-state `CONTEXT.md` and product docs must not contradict the new owner-approved sequencing decision.

---

### Task 6: Run the complete automated verification matrix and enforce the boundary

**Files:**
- No new files unless a test exposes an in-scope defect.
- Potentially modify `authored/recipes/app.retroarch.provision.yaml` only under the Global Constraints defect rule.

**Interfaces:**
- Consumes: all implementation and documentation changes.
- Produces: final evidence for Codex `RESULT` and owner review; no commit or push.

- [ ] **Step 1: Verify no manual/physical test was introduced into the automated path**

Inspect the new module and commands. It must contain no `Command::new("adb")`, `RealAdbDevice`, physical environment gate, or invocation of an ignored test. If a future physical harness seemed useful during implementation, leave it out and record it as deferred follow-up rather than scaffolding unneeded code.

- [ ] **Step 2: Run backend formatting/checks**

Run:

```bash
cargo fmt --all --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --check
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --no-default-features
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
```

All must exit 0.

- [ ] **Step 3: Run focused and full backend tests**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml phase_6e1_ -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution phase_6e1_ -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
```

Record observed pass/fail/ignored counts from the run. Do not copy historical counts into the result.

- [ ] **Step 4: Run strict backend Clippy**

Run:

```bash
cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets --all-features -- -D warnings
```

Do not weaken lint configuration or add broad suppressions.

- [ ] **Step 5: Validate the real authored recipe through the existing CLI validator**

Run:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- validate --authored-root authored authored/recipes/app.retroarch.provision.yaml
```

The validation must complete without errors. Warnings, if any, must be reviewed and recorded rather than silently ignored.

- [ ] **Step 6: Prove Phase 6D.6 evidence remains valid and untouched**

Run:

```bash
node tools/phase-6d6-evidence.mjs
node --test tools/phase-6d6-evidence.test.mjs
```

Expected disposition: contract valid but still incomplete with the existing deferred missing repetitions. Do not regenerate evidence, UI captures, or physical bindings unless the validator explicitly reports a source-digest dependency on an actually changed source file; this plan should not change one.

- [ ] **Step 7: Run Git boundary checks**

Run:

```bash
git diff --check
git status --short --branch
git diff --name-only
```

Required final boundary:

- `.serena/memories/memory_maintenance.md` remains the same pre-existing unstaged deletion and is not staged;
- no `docs/testing/phase-6d6/evidence/**`, traces, UI captures, scenario manifest, or evidence schema changed;
- no Tauri/frontend file changed unless an unexpected compile defect made a separately justified correction necessary;
- no ignored/manual/physical test was run;
- nothing is staged, committed, or pushed.

- [ ] **Step 8: Write the Codex result with explicit phase disposition**

The result must state all of the following together:

- Phase 6D remains **In progress**; its manual/physical qualification is owner-deferred, not waived.
- Phase 6E is **In progress** after this automated foundation.
- `app.retroarch.provision` has an automated recipe-qualification foundation covering real authored admission, production planning/review, and deterministic production executor dry-run semantics.
- RetroArch is **not physically or fully end-to-end qualified**.
- No physical/manual/ignored qualification ran.
- Live public network availability was not required by the deterministic qualification tests.
- All actual verification commands and observed outcomes are recorded.
- Any authored correction, if one was required, includes the failing regression, violated existing contract, smallest fix, and reason; otherwise state that authored production YAML was unchanged.

---

## Self-Review Checklist for the Implementer

Before reporting completion, verify:

- Every Phase 6E.1 acceptance criterion in the approved design maps to at least one passing test or explicit documentation boundary above.
- The qualification contract digest matches raw current RetroArch recipe bytes.
- Tests load `authored/` directly; no copied test recipe exists.
- The plan used by executor qualification is the exact output from `plan_configuration`, not a hand-built substitute.
- Cache fixtures prevent network access without changing artifact URLs or the generated plan.
- Review assertions use the production `PlanConfigurationResult.review`.
- Physical status is still `deferred` everywhere and no cleanup authority was accidentally granted.
- Phase 6D closure requirements and evidence remain unchanged.
- No TODO/TBD/placeholders or unbounded follow-up language remains in new contract/docs.
- No implementation files are staged or committed.
