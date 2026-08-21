# Combined RetroArch + BIOS Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the authored AYANEO default plans capability-correct and add source-bound automated qualification for the real default combined RetroArch + BIOS workflow on the capable KONKR Pocket FIT plan.

**Architecture:** Preserve the existing standalone RetroArch and BIOS qualification modules unchanged except where shared authored default-plan expectations require updates. Correct authored device-plan defaults so the generic non-root/app-data-incapable profile does not recommend an incomplete RetroArch workflow, while the capable KONKR Pocket FIT plan defaults both RetroArch and BIOS. Add one domain-named combined qualification module and strict fixture that plans with `selected_recipes: None`, proving the actual default selection through production planning, review, and deterministic executor paths.

**Tech Stack:** Rust backend tests, authored YAML catalog, serde/serde_json, SHA-256 source binding, existing `runtime_configuration::plan_configuration`, `ExecutorAdapters` sandbox execution.

## Global Constraints

- Phase 6D remains **In progress** and all owner-deferred manual/physical evidence requirements remain unwaived.
- Phase 6E remains **In progress** after this slice; this qualifies only the combined RetroArch + BIOS automated workflow.
- Do not broaden `ayaneo.generic` capabilities. Its `root_shell: false` and `app_data_write: false` contract remains authoritative.
- Do not change RetroArch or BIOS recipe YAML to make qualification easier.
- The combined qualification must use the real `ayaneo.konkr_pocket_fit.base` default selection with `selected_recipes: None`; explicit two-recipe selection is not sufficient.
- Do not add qualification-only production APIs, protocol fields, executor seams, dependencies, manifest changes, or lockfile changes.
- Do not model `/sdcard` and `/storage/emulated/0` as equivalent in the fake filesystem. Real Android shared-storage alias semantics remain physical-qualification scope.
- Run no ADB, `RealAdbDevice`, live network, ignored physical tests, cleanup/reset, host-sleep, identity-replacement, UI-smoke, packaged-GUI, signing/notarization, release, or operator/manual qualification.
- Implement and verify only. Do not stage, commit, push, pull, rebase, reset, merge, or cherry-pick.

---

### Task 1: Correct authored default-plan recommendations

**Files:**
- Modify: `authored/device_plans/ayaneo.konkr_pocket_fit.base.yaml`
- Modify: `authored/device_plans/ayaneo.generic.base.yaml`
- Test: existing planner/device-plan validation suites plus the combined qualification added in Task 2

**Interfaces:**
- Consumes: existing `ayaneo.konkr_pocket_fit` capabilities (`root_shell: true`, `app_data_write: true`) and `ayaneo.generic` capabilities (`root_shell: false`, `app_data_write: false`).
- Produces: a capable default combined plan on KONKR Pocket FIT and a generic default that does not recommend an incomplete RetroArch workflow.

- [ ] **Step 1: Capture the current default-selection behavior in a failing combined qualification test**

Add the initial Task 2 test first, asserting `selected_recipe_refs == ["app.retroarch.provision", "feature.copy_bios"]` when planning `ayaneo.konkr_pocket_fit.base` with `selected_recipes: None`. Run it before editing YAML and confirm it fails because BIOS is not currently default-selected.

- [ ] **Step 2: Enable BIOS by default on the capable KONKR Pocket FIT plan**

Change the recipe list to:

```yaml
recipes:
  - recipe_ref: app.retroarch.provision
    selected_by_default: true
  - recipe_ref: feature.copy_bios
    selected_by_default: true
  # - recipe_ref: app.xaniteog.install
  #   selected_by_default: true
```

Do not alter its profile capabilities.

- [ ] **Step 3: Stop the generic plan from default-recommending incomplete RetroArch provisioning**

Change `ayaneo.generic.base` so `feature.copy_bios` remains default-selected but `app.retroarch.provision` is not selected by default. Preserve RetroArch as an authored option if the device-plan schema/pattern supports a non-default recipe entry; otherwise remove only that default recipe-plan entry rather than changing generic capabilities.

- [ ] **Step 4: Run focused authored/planner validation**

Run the existing backend tests covering device-plan loading/default selection and CLI validation for both modified device plans. Expected: all pass; generic defaults no longer expand RetroArch, KONKR defaults expand RetroArch + BIOS.

---

### Task 2: Add a strict combined qualification contract and production planning/review tests

**Files:**
- Create: `tests/fixtures/recipe-qualification/retroarch-bios/qualification-contract.json`
- Create: `crates/emuchef-rust-backend/src/recipe_qualification_retroarch_bios_tests.rs`
- Modify: `crates/emuchef-rust-backend/src/lib.rs`

**Interfaces:**
- Consumes: real authored `app.retroarch.provision`, `feature.copy_bios`, `ayaneo.konkr_pocket_fit.base`, `ayaneo.konkr_pocket_fit`, `CatalogSnapshot::legacy_local`, and `runtime_configuration::plan_configuration`.
- Produces: a domain-stable automated contract proving actual default-selection composition, input binding, production review, and source identity.

- [ ] **Step 1: Create a strict contract bound to all authored composition inputs**

The contract must use `deny_unknown_fields` in its Rust deserializer and record at minimum:

```json
{
  "schemaVersion": 1,
  "planningDevicePlan": "ayaneo.konkr_pocket_fit.base",
  "deviceProfile": "ayaneo.konkr_pocket_fit",
  "selectedRecipes": ["app.retroarch.provision", "feature.copy_bios"],
  "expandedRecipes": ["app.retroarch.provision", "feature.copy_bios"],
  "requiredInputs": ["feature.copy_bios/bios_source_dir"],
  "optionalInputs": ["app.retroarch.provision/retroarch_cfg"],
  "automatedStatus": "qualified",
  "physicalStatus": "deferred",
  "physicalCleanupAuthority": "not_authorized_for_recipe_qualification"
}
```

Also bind SHA-256 values for both recipe YAML files and the KONKR device-plan/profile YAML files using their exact raw bytes at implementation time. Do not hard-code stale hashes from this plan; calculate and record the current bytes after Task 1 edits.

- [ ] **Step 2: Register the combined test module**

Add only:

```rust
#[cfg(test)]
mod recipe_qualification_retroarch_bios_tests;
```

to `lib.rs` near the existing standalone qualification modules.

- [ ] **Step 3: Add production default-selection planning helper**

Implement a test-local helper that calls `runtime_configuration::plan_configuration(ConfigurationContextRequest { ... })` with:

```rust
device_plan: Some("ayaneo.konkr_pocket_fit.base".to_string()),
selected_recipes: None,
explicit_bindings: bindings_containing_required_bios_directory,
```

Optional RetroArch config may be bound for deterministic executor coverage, but recipe selection must remain `None`.

- [ ] **Step 4: Prove source binding and exact composition**

Add tests asserting:

- all four authored source digests match the strict contract;
- selected and expanded recipes are exactly RetroArch then BIOS;
- device plan/profile identities are exact;
- both recipes appear in production recipe snapshots;
- required BIOS input is explicit and optional RetroArch config follows the existing optional-input semantics;
- required capabilities include the capabilities needed by the emitted combined plan and the KONKR profile provides them.

- [ ] **Step 5: Prove required BIOS input fails closed in the real default composition**

Plan with `selected_recipes: None` and no BIOS binding. Assert `binding_missing` for `feature.copy_bios/bios_source_dir`, with no executable plan, digest, or review. This proves default composition cannot silently omit the required second recipe.

- [ ] **Step 6: Prove the production review represents both workflows without authority leakage**

For valid inputs assert the review:

- is executable and blocker-free;
- contains exactly two features in the same production composition;
- contains RetroArch download/install/copy/permission/launch work and BIOS copy work;
- reports action count equal to the exact generated plan;
- exposes friendly file basenames where expected but does not serialize temporary host parent paths or device serial/authority.

- [ ] **Step 7: Run focused planning/review tests**

Run the new module only. Expected: contract, default-selection, required-input, and review tests all pass.

---

### Task 3: Qualify deterministic combined execution and composition failure semantics

**Files:**
- Modify: `crates/emuchef-rust-backend/src/recipe_qualification_retroarch_bios_tests.rs`

**Interfaces:**
- Consumes: the exact generated combined plan from Task 2, standalone RetroArch cache-fixture conventions, normal `ExecutorAdapters::with_sandbox_roots`, and test-local device wrappers only where a negative condition cannot be represented by the fake filesystem.
- Produces: evidence that the two real recipes execute as one unchanged plan with truthful repeat-run and late-failure behavior.

- [ ] **Step 1: Build a combined deterministic workspace**

Create test-local temp roots for runtime, cache, fake device, host inputs, RetroArch config, and nested BIOS input. Seed every RetroArch `default` cache artifact exactly as the existing standalone qualification does and create representative nested BIOS files under the allowed host-input root.

Do not add a shared production helper solely to deduplicate test setup.

- [ ] **Step 2: Execute the exact generated combined plan through normal sandbox adapters**

Run `ExecutorRunner` with `ExecutorAdapters::with_sandbox_roots`. Assert:

- run success is true;
- every generated step has a terminal record;
- representative RetroArch steps execute;
- `feature.copy_bios/copy_bios_dir` executes;
- representative nested BIOS files arrive under fake-device `sdcard/RetroArch/system/...` with exact bytes;
- no Failed, Blocked, or Cancelled records exist.

Do not assert fake-filesystem equivalence between `/sdcard/...` and RetroArch-authored `/storage/emulated/0/...` paths.

- [ ] **Step 3: Prove repeated combined execution preserves recipe-specific skip semantics**

Run the same exact plan twice through the same deterministic runner/device state. Assert:

- first run executes `install_retroarch`;
- second run skips only the authored RetroArch install via `package_installed`;
- BIOS copy still executes on the second run under its authored `sync` policy;
- the second run remains successful and contains no failed/blocked/cancelled steps.

- [ ] **Step 4: Prove a late BIOS verification failure preserves truthful prior RetroArch results**

Use a combined-test-private `ExecutorDevice` wrapper around `FakeDryRunDevice` that delegates ordinary operations but forces only `/sdcard/RetroArch/system` `path_exists` false. Execute the unchanged generated plan and assert:

- the plan is byte/structurally unchanged before vs after execution;
- RetroArch work that completed before the BIOS step remains recorded as completed/skipped truthfully;
- the BIOS copy step attempts its delegated copy operations before verification;
- BIOS verification fails and the overall run is unsuccessful;
- no later/nonexistent work is fabricated as successful.

Do not modify `executor.rs` to create this test path.

- [ ] **Step 5: Run focused execution tests under both feature configurations**

Run the combined qualification module under default and `real-execution` features. Expected: identical deterministic qualification behavior and no physical/device access.

---

### Task 4: Preserve standalone qualification and update truthful current-state documentation

**Files:**
- Modify: `docs/product/recipe-qualification-retroarch.md`
- Modify: `docs/product/recipe-qualification-bios.md`
- Create: `docs/product/recipe-qualification-retroarch-bios.md`
- Modify: `docs/product/product-roadmap.md`
- Modify: `CONTEXT.md`

**Interfaces:**
- Consumes: passing standalone and combined qualification results.
- Produces: current-state documentation that distinguishes standalone qualification, combined automated qualification, and still-deferred physical/end-to-end work.

- [ ] **Step 1: Re-run both standalone qualification modules before documentation edits**

Expected: all existing standalone RetroArch and BIOS tests pass unchanged in behavior after the device-plan default corrections. If an assertion legitimately depends on default-plan content, update only that assertion while preserving its standalone explicit-selection boundary.

- [ ] **Step 2: Document the combined qualification boundary**

Create `docs/product/recipe-qualification-retroarch-bios.md` recording:

- actual default selection via `ayaneo.konkr_pocket_fit.base`;
- source-bound production planning and review;
- deterministic combined execution;
- repeat-run RetroArch install skip plus BIOS re-execution;
- truthful late BIOS failure behavior;
- no claim that fake paths prove Android `/sdcard` and `/storage/emulated/0` alias semantics;
- physical/manual/end-to-end qualification remains deferred;
- no physical cleanup authority.

- [ ] **Step 3: Update standalone docs and roadmap without overclaiming**

Update standalone docs only to point to the now-qualified combined workflow. Update roadmap/current context so:

- combined RetroArch + BIOS automated qualification is recorded complete;
- Phase 6E remains **In progress** because Obtainium, ROM/content, and physical/end-to-end recipe qualification remain;
- Phase 6D remains **In progress** with the same owner-deferred evidence requirements;
- the next automated recipe slice is selected from remaining production-intended recipes rather than implying Phase 6E completion.

- [ ] **Step 4: Verify no stale "combined remains unqualified" statements remain in active current-state docs**

Search bounded active docs and qualification sources. Historical `.chatgpt/codex-runs/**` provenance must not be rewritten.

---

### Task 5: Full validation and scope audit

**Files:**
- No additional implementation files expected.

**Interfaces:**
- Consumes: Tasks 1-4.
- Produces: evidence that the slice is ready for implementation review without broadening runtime authority or physical claims.

- [ ] **Step 1: Run formatting and strict lint gates**

Run backend formatting/check and strict Clippy with `--all-targets --all-features -- -D warnings`.

- [ ] **Step 2: Run complete backend suites under both configurations**

Run the full backend test suite under default features and `--features real-execution`.

- [ ] **Step 3: Run authored CLI validation**

Validate both recipes and both modified device plans through existing CLI/catalog validation paths.

- [ ] **Step 4: Re-run Phase 6D.6 evidence validation without collecting evidence**

Run the existing Phase 6D.6 validator/tests. Expected: still valid but incomplete with exactly the previously owner-deferred physical/UI-smoke requirements; no evidence, manifest, schema, or index changes.

- [ ] **Step 5: Audit the final diff and Git state**

Confirm:

- no production executor/runtime/protocol code changed except `lib.rs` test-module registration;
- no recipe YAML changed;
- generic profile capabilities did not change;
- only the two approved device-plan default selections changed in authored catalog data;
- no dependencies/Cargo manifests/lockfiles changed;
- no physical evidence files changed;
- nothing is staged, committed, or pushed.
