# BIOS Recipe Qualification Design

**Roadmap provenance:** EmuChef proper Phase 6E.2  
**Owner:** EmuChef proper / Shared Runtime  
**Status:** Approved design; implementation not started

## 1. Purpose

Qualify the real authored `feature.copy_bios` workflow through production planning, review, input validation, and deterministic executor paths without expanding into physical-device or combined-device-plan qualification.

This task also removes roadmap-phase nomenclature from the existing RetroArch qualification implementation artifacts. Phase identifiers remain valid historical/product-roadmap language in documentation, but they must not become API, module, helper, test, fixture, or implementation-path identities.

Phase 6D remains **In progress**. All remaining Phase 6D manual and physical qualification is still owner-deferred; this sequencing decision does not waive or reduce those requirements. Phase 6E remains **In progress** after this task.

## 2. Decisions and rationale

### 2.1 Qualification target

Phase 6E.2 qualifies `feature.copy_bios` as a **standalone recipe**.

`ayaneo.generic.base` supplies the real authored device-plan/device-profile context because it includes `feature.copy_bios` and its `ayaneo.generic` profile grants `shared_storage_write`. Planning must explicitly select only `feature.copy_bios`; the device plan's default RetroArch selection must not broaden the qualification target.

**Rationale:** this adds new semantic coverage—required host-directory input, shared-storage copy, authored copy policy, and destination verification—without re-proving the RetroArch workflow already covered by the previous qualification task.

### 2.2 Qualification depth

Both positive and negative paths are required:

1. A valid BIOS source directory containing representative nested files plans, reviews, and executes successfully through production paths.
2. Missing or invalid required BIOS input is rejected before executable review/apply.
3. A deterministic destination-verification failure causes the authored copy step and overall run to fail rather than report false success.

**Rationale:** a happy-path-only smoke test is insufficient qualification for a required host input and a verified device-side copy.

### 2.3 Structure

Use a dedicated BIOS qualification module and source-bound contract, following the proven RetroArch qualification pattern without extracting a generic qualification framework yet.

**Rationale:** two recipe qualifications are not enough evidence to justify a shared abstraction. Keeping the tests recipe-specific preserves auditability and avoids turning this task into infrastructure refactoring.

### 2.4 Naming invariant

Implementation artifacts use stable domain terminology only. Do not introduce `phase`, `6e1`, `6e2`, `slice`, or equivalent roadmap numbering into:

- Rust module or file names;
- helper/function/type/test names;
- fixture directory or file names;
- implementation-facing documentation filenames; or
- reusable APIs.

Roadmap/product prose may still say **Phase 6E.1** or **Phase 6E.2** when recording historical or sequencing provenance.

## 3. Rejected alternatives

### 3.1 Obtainium as the next recipe

Rejected for this task. Obtainium would primarily revisit remote-artifact resolution, APK installation, and package-installed repeat-run behavior already exercised heavily by RetroArch qualification.

### 3.2 ROM copy as the next recipe

Deferred. ROM copy exposes useful destination/path-policy behavior but does not advance an existing default combined device plan as directly as BIOS copy.

### 3.3 Qualify `ayaneo.generic.base` as a combined RetroArch + BIOS workflow now

Rejected for this task. That would mix already-qualified RetroArch behavior with the new BIOS behavior and collapse the intended next combined-plan qualification into this task.

### 3.4 Extract a generic recipe-qualification harness first

Rejected as premature. Shared extraction should occur only after multiple independent recipe qualifications demonstrate stable common structure worth preserving.

## 4. Naming migration for existing RetroArch qualification

The previous RetroArch automated qualification behavior must remain semantically unchanged while its implementation-facing phase nomenclature is removed.

Rename:

- `crates/emuchef-rust-backend/src/phase_6e1_recipe_qualification_tests.rs`
  to `crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs`;
- `tests/fixtures/phase-6e/retroarch/`
  to `tests/fixtures/recipe-qualification/retroarch/`;
- `docs/product/phase-6e1-recipe-qualification-foundation.md`
  to `docs/product/recipe-qualification-retroarch.md`.

Update the corresponding module declaration, fixture references, `CONTEXT.md`, product-roadmap references, comments, and implementation-facing helper/test identifiers that contain phase-coded naming.

The migration is mechanical. It must not change the qualified RetroArch semantics, source digest expectations, production planning/review paths, deterministic executor behavior, or physical-qualification disposition.

## 5. BIOS qualification artifacts

Create domain-based artifacts:

- `crates/emuchef-rust-backend/src/recipe_qualification_bios_tests.rs`;
- `tests/fixtures/recipe-qualification/bios/qualification-contract.json`;
- `docs/product/recipe-qualification-bios.md`.

The Rust module should remain test-only and be registered from `lib.rs` using its domain-based name.

## 6. Source-bound BIOS qualification contract

The BIOS contract must be strict and bound by SHA-256 to the raw bytes of `authored/recipes/feature.copy_bios.yaml`. Authored changes must fail closed until the qualification contract is deliberately reviewed.

The contract should capture at least:

- target recipe: `feature.copy_bios`;
- planning context: `ayaneo.generic.base`;
- selected and expanded recipe set: only `feature.copy_bios` unless production dependency expansion legitimately requires otherwise;
- required input: `feature.copy_bios/bios_source_dir`;
- required capability: `shared_storage_write`;
- required operation family: `copy_files`;
- authored copy policy: `sync`;
- destination: `/sdcard/RetroArch/system`;
- authored verification: `path_exists` for `/sdcard/RetroArch/system`;
- live-network requirement: false;
- automated qualification status; and
- physical qualification status: deferred with no new cleanup authority.

Do not bind incidental internal representation that is irrelevant to qualification semantics.

## 7. Production-path planning and review

Load the real authored catalog and call the production runtime-configuration planning path.

Use `ayaneo.generic.base` for real device-plan/profile context while explicitly selecting only `feature.copy_bios`. The resulting plan must demonstrate that the context grants the authored `shared_storage_write` requirement without accidentally adding the device plan's default RetroArch recipe to the qualification target.

With valid BIOS input:

- planning must produce no error diagnostics;
- the plan must contain the expected BIOS recipe and copy operation semantics;
- the production review projection must be executable and blocker-free;
- the review must represent the required BIOS input and copy action truthfully without exposing unnecessary host-path details.

## 8. Input-validation qualification

Qualification must cover required-input rejection before execution.

At minimum:

1. No `bios_source_dir` binding supplied.
2. A binding supplied for a directory that does not exist or otherwise violates the authored directory-input validation contract.

The authoritative production validation/planning path must reject these states before an executable review/apply can proceed. Tests should assert the semantic failure classification and avoid brittle dependence on incidental full diagnostic strings when a stable code/shape is available.

## 9. Successful deterministic execution

Create a temporary BIOS source directory containing representative nested content rather than a single empty marker. The exact filenames are test fixtures, not product semantics, but the tree should prove recursive directory-copy behavior.

Execute the unchanged generated plan through the existing deterministic sandbox-root executor adapters used by production-oriented qualification. The successful run must prove:

- the authored `copy_files` step executes;
- the authored `sync` policy is preserved;
- the destination remains `/sdcard/RetroArch/system`;
- nested representative source content traverses the production copy path; and
- the copy step and overall run complete successfully with no Failed, Blocked, or Cancelled result.

No ADB, physical device, public network, packaged GUI, or manual operator action is allowed.

## 10. Destination-verification failure

The negative executor test must execute the **unchanged generated plan**.

Arrange only deterministic adapter/device state so the production copy operation can complete while the subsequent authored `path_exists` predicate for `/sdcard/RetroArch/system` evaluates false. Do not edit the plan, remove the verification condition, replace the destination, or manufacture an alternate test-only plan.

Expected outcome:

- the BIOS copy step is reported Failed because its authored verification fails;
- the overall run is unsuccessful;
- no result path reports the authored step as successful after failed verification; and
- any prior truthful operation record remains intact rather than being rewritten as if the copy never ran.

If the current deterministic adapter cannot express this state, add the narrowest test-only/synthetic control needed. Do not alter real-device production semantics merely to make the test possible.

## 11. Production-code change boundary

The default expectation is **no production executor behavior change**.

If qualification exposes a genuine production defect that prevents the approved authored workflow from behaving correctly, implementation may propose a narrowly scoped production fix, but it must:

1. be justified by the production contract rather than by test convenience;
2. preserve existing real-device behavior outside the defect;
3. receive focused regression coverage; and
4. be called out explicitly in the implementation result instead of being hidden inside qualification scaffolding.

Do not modify authored recipe or device-plan semantics merely to make qualification pass.

## 12. Documentation and status

After successful implementation:

- `docs/product/recipe-qualification-retroarch.md` records RetroArch qualification with Phase 6E.1 retained only as roadmap provenance;
- `docs/product/recipe-qualification-bios.md` records BIOS automated qualification and identifies it as Phase 6E.2 in product prose;
- `CONTEXT.md` and `docs/product/product-roadmap.md` state that RetroArch and BIOS automated qualification are complete;
- Phase 6E remains **In progress**;
- Phase 6D remains **In progress**, with all owner-deferred manual/physical evidence and closure requirements unchanged; and
- combined RetroArch + BIOS device-plan qualification remains explicitly unqualified and is the logical next recipe-qualification task.

The documentation must continue to distinguish automated deterministic qualification from physical or fully end-to-end qualification.

## 13. Out of scope

This task does not include:

- Obtainium qualification;
- ROM/content-copy qualification;
- combined RetroArch + BIOS qualification;
- generic qualification-framework extraction;
- real ADB or physical-device runs;
- manual/UI-smoke qualification;
- live network qualification;
- device cleanup/reset authority;
- packaged-GUI qualification;
- signing, notarization, release qualification, or production promotion; or
- closure of Phase 6D or Phase 6E.

## 14. Completion criteria

Implementation is complete only when all of the following are true:

1. Existing RetroArch qualification passes unchanged after its domain-naming migration.
2. No implementation-facing recipe-qualification artifact introduced or migrated by this task uses phase/slice nomenclature.
3. The BIOS qualification contract is SHA-256-bound to the real authored source and fails closed when that source changes.
4. Valid BIOS input plans through the production runtime with `ayaneo.generic.base` context while selecting only the BIOS recipe.
5. The production review for valid input is executable and blocker-free.
6. Missing and invalid BIOS input are rejected before execution.
7. A representative nested BIOS directory executes successfully through the deterministic production executor path with authored `sync` semantics and destination preserved.
8. Forced destination-verification failure produces a failed BIOS step and failed overall run using the unchanged generated plan.
9. Focused qualification tests and repository-required Rust formatting, lint, and test gates pass under the applicable feature configurations.
10. Documentation truthfully records automated qualification without claiming physical/end-to-end success.

## 15. Next task boundary

The logical follow-up is combined **RetroArch + BIOS** qualification through a real default device-plan selection. This design deliberately does not pre-implement, partially claim, or weaken that future qualification boundary.
