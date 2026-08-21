# Phase 6E.1 Recipe Qualification Foundation — Design

## 1. Status and authority

**Owner:** EmuChef proper / Shared Runtime  
**Phase:** 6E.1  
**Status:** Approved design  
**Primary target:** `app.retroarch.provision`

This design starts Phase 6E automated recipe-qualification work without closing
Phase 6D. The owner has explicitly deferred all manual and physical
qualification until further notice. That decision supersedes the earlier Phase
6D.6 sequencing rule that Phase 6E could not begin before Phase 6D closed.

Phase 6D therefore remains **In progress** with its existing missing physical
and UI-smoke evidence. Phase 6E may become **In progress** for automated
qualification foundation work, but no recipe may be described as physically or
end-to-end qualified until the deferred physical qualification is deliberately
resumed and completed.

## 2. Goal

Establish a reusable, fail-closed recipe-qualification contract and automated
harness using the real authored corpus and the existing production planning,
review, and executor boundaries, with the canonical RetroArch provisioning
workflow as the first qualification target.

Phase 6E.1 must prove as much of the end-to-end recipe contract as can be proven
without a physical Android device while leaving a safe, explicit boundary for
future physical qualification.

## 3. Why RetroArch is the first target

`authored/recipes/app.retroarch.provision.yaml` is the highest-value first
qualification workflow because it exercises a broad set of production behavior
in one authored recipe:

- remote artifact resolution;
- APK installation;
- first-launch bootstrap and bounded waits;
- force-stop behavior;
- runtime permission and app-op application;
- archive and artifact-group extraction on the host;
- host-to-device copies into both app-private and shared storage;
- optional user-provided configuration input;
- verification predicates;
- dependency ordering across independent extraction/copy branches; and
- final application launch.

Using the real authored recipe gives Phase 6E meaningful coverage of production
workflow composition rather than another operation-level fixture. It also
exposes qualification gaps that isolated Phase 6C executor tests cannot detect.

## 4. Scope

### 4.1 In scope

Phase 6E.1 will establish:

1. a machine-readable or equivalently strict repository-owned qualification
   contract for the RetroArch workflow;
2. automated admission of the real authored recipe and any real authored
   dependencies needed for the target workflow;
3. deterministic production-path planning from the real authored corpus;
4. production review projection checks bound to the exact generated plan;
5. deterministic executor qualification using controlled adapters rather than a
   real Android device;
6. explicit qualification disposition separating automated evidence from
   deferred physical evidence;
7. a focused Phase 6E product/current-state document; and
8. only if needed for future reproducibility, an ignored and fail-closed
   real-ADB harness shape that is not executed during this or later automated
   work unless the owner explicitly resumes physical qualification.

### 4.2 Out of scope

Phase 6E.1 does not:

- run a physical Android device;
- run any ignored real-ADB test;
- run host-sleep, identity-replacement, UI-smoke, packaged-GUI, or other manual
  qualification;
- complete Phase 6D;
- mark the RetroArch workflow physically or end-to-end qualified;
- enable ordinary production real execution;
- qualify Obtainium, BIOS copy, ROM copy, Xaniteog, Daijisho, ES-DE, or other
  workflows beyond incidental dependency/corpus validation;
- add checkpointing, resume, replay, rollback, automatic retry, or reconnect;
- add a new public protocol, serialized product DTO, or frontend API only for
  qualification;
- redesign the planner, executor, or review architecture; or
- broadly refactor existing Phase 6C or Phase 6D qualification harnesses.

## 5. Sequencing invariant

Manual and physical qualification is deferred globally until the owner says
otherwise. Automated tasks must not silently interpret "qualification" as
permission to run a device test.

The repository must preserve both truths simultaneously:

- Phase 6D is still **In progress** because required physical/UI evidence is
  incomplete.
- Phase 6E is **In progress** because automated recipe-qualification foundation
  work has started under an explicit owner sequencing decision.

This is not a waiver of Phase 6D evidence and is not permission to weaken Phase
6D closure criteria. It is only a sequencing change.

## 6. Qualification contract

Phase 6E.1 should introduce one strict, repository-owned qualification contract
for the RetroArch target. The exact file format may follow existing fixture or
manifest conventions, but it must be deterministic, reviewable, and validated
by tests.

The contract must identify at least:

- qualification schema/version identity;
- target recipe id `app.retroarch.provision`;
- the source recipe content digest or an equally strong source-provenance
  binding so stale qualification expectations cannot silently survive authored
  changes;
- allowed/expected expanded recipe ids;
- required runtime capabilities;
- required and optional user inputs;
- external artifact categories and the fact that live network retrieval is not
  automated qualification evidence unless deliberately modeled by a bounded
  fixture;
- expected operation families and material ordering constraints;
- required verification behavior;
- expected terminal reporting shape for deterministic success and selected
  deterministic failure cases;
- future physical-device preconditions and reset/cleanup ownership;
- automated qualification status; and
- physical qualification status, which remains explicitly deferred/unqualified.

The contract must not contain secrets, machine-specific host paths, device
serials, mutable temporary paths, or raw external command output.

## 7. Automated qualification architecture

### 7.1 Authored admission

Automated qualification must load `app.retroarch.provision` from the real
`authored/` tree through production catalog/loading code. Tests must not maintain
a second test-owned copy of the recipe as the source of truth.

Admission should fail if the production recipe becomes structurally or
semantically invalid. If the qualification contract uses a source digest, a
recipe change must require explicit qualification expectation review rather
than silently accepting stale expectations.

### 7.2 Planning

The planning layer must use the production planner and the real authored
catalog. It should provide deterministic device facts/capabilities and controlled
bindings sufficient to plan the workflow without contacting ADB.

The planning qualification must verify material contract rather than brittle
serialization where possible. At minimum it must prove:

- the target/expanded recipe set is correct;
- required capabilities are represented correctly;
- optional `retroarch_cfg` behavior is understood for both absent and supplied
  input cases where the planner differentiates them;
- artifact references resolve into the expected plan families;
- the plan includes the expected install, launch/wait/stop, permission,
  extraction, copy, verification, and final-launch behavior;
- dependency ordering preserves the authored workflow; and
- no synthetic test-only step substitutes for the actual authored recipe.

### 7.3 Review projection

The exact generated production plan must pass through the existing production
review projection.

Automated checks must prove that review remains bound to the same plan identity
and accurately presents the material actions and user-input requirements while
preserving existing sanitization and authority boundaries. Tests must not add a
qualification-only review DTO or bypass production projection logic.

### 7.4 Executor qualification

The executor qualification must exercise the generated plan through
`ExecutorRunner` with deterministic controlled adapters. It should reuse the
existing executor abstraction rather than create a second recipe executor.

The controlled adapters must provide enough behavior to prove:

- dependency/order semantics;
- success accounting;
- skip behavior where the workflow defines skip predicates;
- verification behavior;
- expected propagation when an operation fails;
- later dependent work remains appropriately unattempted after a terminal
  failure according to the existing execution contract; and
- final reporting remains consistent with Phase 6D safety semantics.

The deterministic executor qualification is not evidence that Android accepted
an APK, created an app-private path, honored an app-op, or launched RetroArch on
hardware. Those claims remain physical qualification.

### 7.5 Network/artifact boundary

The authored RetroArch workflow references live remote artifacts. Automated
Phase 6E.1 must not make qualification dependent on mutable public network
availability or current Libretro/GitHub responses.

Qualification should separate two concerns:

- prove that the authored artifact definitions enter the production planning and
  resolution model correctly; and
- execute deterministic downstream recipe behavior using controlled artifact
  results/fixtures at the existing adapter seam.

Live remote downloads are not required to call the automated foundation
complete. If current architecture cannot inject deterministic artifact outcomes
without bypassing production behavior, that limitation must be documented and
handled with the smallest focused seam rather than adding broad network test
infrastructure.

## 8. Future physical harness boundary

A physical recipe-qualification harness may be scaffolded only when doing so is
necessary to make the eventual manual run reproducible. It must be separate
from the very large Phase 6D.6 interruption harness and should follow the
existing Phase 6C fail-closed patterns.

Any such harness must:

- be `#[ignore]` by default;
- require the existing global real-ADB opt-in plus a Phase 6E-specific opt-in;
- require one exact selected serial before ADB access;
- validate every destructive/package/path authority before mutation;
- use the real production planner/executor path rather than hand-built operation
  substitutes where practical;
- define clean/reset preconditions and exact cleanup ownership;
- avoid external mutation outside declared owned paths/packages; and
- emit sanitized evidence suitable for a later qualification contract.

No implementation or automated validation command may run this harness while
physical qualification remains deferred.

## 9. Authored-data change policy

Phase 6E.1 is a qualification slice, not a recipe-redesign slice.

If automated qualification reveals a genuine defect in
`app.retroarch.provision` or another required authored file:

1. reproduce the defect through the new qualification test;
2. determine whether it violates an already documented production contract;
3. make the smallest coherent authored/runtime correction needed to satisfy that
   contract; and
4. record the reason and qualification impact explicitly.

Do not rewrite the recipe merely to make tests easier. Do not update expected
qualification data to bless behavior that contradicts the roadmap or existing
runtime semantics.

## 10. Module and file boundaries

Prefer a focused Phase 6E qualification unit rather than adding substantial new
logic to `physical_interruption_qualification.rs`.

Likely implementation areas are:

- a dedicated Phase 6E qualification test/module near existing backend planner
  and executor qualification code;
- a strict qualification contract under an existing fixture/testing namespace;
- a Phase 6E product document under `docs/product/`; and
- narrowly scoped roadmap/`CONTEXT.md` current-state updates after automated
  verification passes.

The implementation plan must inspect existing test conventions before fixing
exact filenames. New files should have one clear responsibility and avoid
creating a parallel planning, review, or execution stack.

## 11. Error and failure policy

Qualification must fail closed on:

- stale or mismatched qualification-source provenance;
- missing or invalid authored recipe data;
- unexpected dependency expansion;
- missing expected operation families;
- material dependency-order drift;
- review projection that no longer represents the generated plan safely;
- executor accounting or failure propagation inconsistent with existing Phase
  6D semantics; or
- any attempt to satisfy an automated criterion by using physical/manual
  evidence while the deferral is active.

Qualification expectations should describe stable semantic behavior rather than
incidental ordering or serialization details unless the exact ordering is part
of the authored contract.

## 12. Verification strategy

Phase 6E.1 implementation must leave all applicable automated quality gates
green. At minimum the implementation plan should include:

- focused Phase 6E qualification tests;
- authored corpus/catalog validation covering the real target recipe;
- relevant planner tests;
- relevant review-projection tests;
- relevant executor tests;
- backend `cargo fmt --check`;
- backend `cargo check`;
- backend tests for affected feature sets;
- backend strict Clippy with `-D warnings`;
- Tauri/frontend validation only if the implementation actually touches those
  boundaries; and
- Phase 6D.6 validator/regression checks if any source-digested or shared
  execution path changes require them.

No ignored tests or device-dependent commands may be included in the automated
verification run.

Test counts must be recorded from the actual run rather than copied into the
plan as permanent expectations.

## 13. Documentation and status after Phase 6E.1

After all automated acceptance criteria pass, documentation should state:

- Phase 6D: **In progress**, manual/physical qualification deferred by owner;
- Phase 6E: **In progress**;
- RetroArch: automated recipe-qualification foundation complete;
- RetroArch physical/end-to-end qualification: deferred and not claimed; and
- ordinary production real execution: unchanged/disabled according to the
  existing release boundary.

Do not change Phase 6D evidence files, traces, scenario manifest, or historical
qualification results merely to reflect the sequencing decision.

## 14. Rejected alternatives

### 14.1 Static/planner-only qualification

Rejected as the primary design because it would prove authored loading and plan
shape but stop before review and executor semantics. That would leave too much
of the recipe-level contract untested and make Phase 6E little more than an
expanded planner regression suite.

### 14.2 Full multi-recipe Phase 6E harness in one slice

Rejected because RetroArch alone spans enough artifact, planner, executor, and
future physical concerns to validate the qualification architecture. Building
Obtainium, BIOS, ROM-copy, and future launcher workflows simultaneously would
increase scope before the reusable contract has proven itself.

### 14.3 Wait for Phase 6D physical closure

Rejected by explicit owner sequencing decision. Manual and physical
qualification is deferred until further notice. Blocking all automated Phase 6E
work on that deferral would create avoidable idle sequencing while providing no
additional safety. Phase status and evidence claims remain independent so this
does not waive Phase 6D.

## 15. Invariants to preserve

Phase 6E.1 must preserve:

- Rust runtime authority over authored loading, planning, review contracts, and
  execution semantics;
- the existing real-execution compile/runtime gating model;
- Phase 6D terminal-state, partial-result, timeout, identity, transport, root,
  and no-resume/no-replay safety behavior;
- accepted Phase 6C and Phase 6D physical evidence and provenance;
- sanitization boundaries for serials, paths, command output, credentials, and
  runtime authority;
- no production claims derived from deterministic fake adapters;
- no physical/manual qualification while owner deferral is active; and
- no broad architecture change solely to make qualification convenient.

## 16. Phase 6E.1 acceptance criteria

Phase 6E.1 is complete when:

1. a strict RetroArch qualification contract is checked into the repository and
   bound to the real authored source;
2. automated qualification loads the real recipe through production catalog
   code;
3. the production planner generates a contract-consistent RetroArch plan under
   deterministic device/input context;
4. the production review projection is exercised against that exact plan;
5. the generated workflow executes through `ExecutorRunner` with deterministic
   adapters and proves material ordering, skip/verification, success, and
   selected failure/report semantics;
6. live public network availability is not required for deterministic automated
   success;
7. no physical/manual/ignored qualification was run;
8. all affected automated quality gates pass with strict Clippy preserved;
9. current-state docs truthfully show Phase 6D still In progress and Phase 6E
   now In progress; and
10. no wording claims that RetroArch is physically or fully end-to-end
    qualified.

## 17. Follow-on slices

After this foundation is implemented and reviewed, later Phase 6E slices can
reuse the contract/harness pattern for:

- Obtainium installation;
- BIOS copy;
- ROM/content copy;
- canonical combined device-plan workflows; and
- later production-intended recipes such as Daijisho or ES-DE only when those
  authored recipes and required assets actually exist and are promoted into
  scope.

Physical recipe qualification remains deferred across those slices until the
owner explicitly changes that decision.
