# Device Qualification Harness Design

**Date:** 2026-08-23  
**Status:** Approved design  
**Scope:** Physical-device qualification orchestration, evidence capture, target registration, evidence-contract strengthening, and migration of active qualification artifacts to domain-oriented naming.

## 1. Purpose

EmuChef already has repository-owned qualification definitions, evidence validation, compatibility projection, and matrix generation, but the current foundation intentionally stops short of driving a physical qualification run end to end. The remaining gap is a trusted operator workflow that exercises EmuChef proper, captures evidence from the actual production execution path, and promotes that evidence into the repository without creating a second planner, executor, or qualification authority.

This design adds a development-only physical qualification mode inside EmuChef proper. The mode layers qualification state and evidence capture over the existing product workflow rather than replacing it. Production EmuChef remains the system under test. The qualification subsystem may constrain the declared target/workflow and observe the resulting run, but it may not bypass product review, confirmation, validation, planning, execution, or device authority.

The implementation also strengthens the existing qualification evidence contract so that device-fact provenance and the actual production execution report are preserved durably. Because no real physical qualification records exist yet, the affected target and evidence contracts move cleanly to schema version 2 rather than preserving a weaker version-1 shape.

## 2. Goals

The implementation must:

1. provide a development-only qualification mode inside the existing EmuChef application;
2. exercise the ordinary production path from device discovery through reviewed real execution and production report export;
3. bind each qualification session to one registered physical target and one canonical workflow;
4. capture target facts through trusted production observations wherever possible;
5. allow operator attestation only for narrowly modeled facts that cannot be established reliably by the trusted host/device boundary;
6. preserve per-fact provenance in the canonical device-target and evidence contracts;
7. derive build and runtime identity from trusted application/build metadata rather than operator-entered environment labels;
8. require a clean committed source state for any build that can record authoritative target or run evidence;
9. persist non-authoritative candidates outside Git so a completed physical run is not lost to an application restart;
10. preserve the exact sanitized production execution report as a digest-bound artifact in each evidence bundle;
11. keep the repository qualification tool as the single authority for schema validation, canonicalization, digest generation, target/run identity, immutable recording, compatibility projection, and matrix generation;
12. preserve invalid harness/infrastructure runs as optional audit evidence without allowing them to influence product qualification state;
13. eliminate active phase/slice nomenclature from qualification implementation paths, tooling names, runtime paths, APIs, and source identifiers; and
14. avoid performing or claiming any physical qualification as part of implementation of the harness itself.

## 3. Non-goals

This work does not:

- add a second planner, executor, device abstraction, or review flow;
- auto-confirm irreversible execution;
- invent bindings, silently repair blockers, or reinterpret a different device/workflow as the declared run;
- make qualification behavior available in ordinary end-user builds;
- qualify any physical device merely because the harness exists;
- add Daijisho or ES-DE recipes or qualification coverage;
- complete any separately deferred physical/manual qualification work outside this device-qualification system;
- move historical roadmap, Superpowers plan/spec, or historical run records merely to remove project-management wording; or
- require Node tooling for ordinary EmuChef product behavior.

## 4. Architectural choice

### 4.1 Selected approach

Use an **integrated qualification overlay with a canonical repository evidence tool**.

The operator remains in the existing EmuChef application. Qualification mode adds a persistent controller/banner and qualification-specific controls, but the ordinary product screens remain authoritative for device discovery, configuration, recipe/input selection, review, real-execution confirmation, execution progress, and result inspection.

The system is split into three trust domains:

### React presentation

React is presentation and operator-interaction only. It may:

- display qualification state;
- request selection of a registered target and canonical workflow;
- display target-registration previews;
- render declared human checkpoints;
- present candidate status and explicit registration/recording actions; and
- hold opaque session/candidate identifiers returned by trusted code.

React must not:

- read or write repository files;
- invoke arbitrary filesystem paths;
- run Git or Node directly;
- perform ADB/device probing directly;
- compute canonical target/run/record digests;
- manufacture automated observation outcomes; or
- become the authority for target, workflow, execution, or evidence validity.

### Trusted Rust/Tauri orchestration

Trusted Rust/Tauri owns the live qualification session. It may:

- determine whether qualification mode is enabled;
- expose qualification-specific commands only when all enablement gates pass;
- capture trusted build/runtime metadata;
- observe production device qualification and real-execution state;
- request the existing production root-check boundary;
- accept narrowly typed operator attestations where the contract allows them;
- enforce target/workflow/session binding;
- invalidate a session when a required invariant is broken;
- capture the production execution report through the existing export boundary;
- persist non-authoritative candidates under the ignored runtime root; and
- invoke fixed operations of the repository qualification tool using opaque candidate identifiers.

Trusted Rust/Tauri does not reimplement the canonical repository schema/digest/projection engine.

### Repository qualification tool

`tools/device-qualification.mjs` remains the single repository evidence authority. It owns:

- schema validation;
- exact-field validation and semantic validation;
- canonicalization;
- deterministic SHA-256 digests;
- deterministic device-target IDs;
- deterministic qualification-run IDs;
- evidence-bundle artifact validation;
- target registration into the canonical registry;
- immutable evidence-bundle recording;
- compatibility classification;
- current-evidence selection;
- workflow-state derivation;
- device-tier derivation; and
- deterministic matrix rendering/checking.

The application must not carry a second implementation of these rules.

## 5. Qualification-mode enablement

Qualification mode is available only when all of the following are true:

1. the application is a debug/development build;
2. the `real-execution` feature is compiled in;
3. the build contains valid metadata proving that it was produced from a clean committed source state; and
4. `EMUCHEF_DEVICE_QUALIFICATION=1` is present.

If any gate fails:

- qualification-specific UI is absent;
- qualification-specific Tauri commands are not exposed/usable;
- target/evidence repository mutation is unavailable; and
- ordinary simulation and guarded real-execution behavior remains unchanged.

A hot-reloaded or dirty-source development session may be useful while developing the feature, but it is not recordable. Authoritative target registration and qualification evidence require a build bound to a clean committed source state.

## 6. Build and runtime identity

A recordable qualification build carries trusted build-time metadata sufficient to distinguish materially different source builds even when the human-readable application version is unchanged.

At minimum, the embedded identity includes:

- EmuChef application version;
- exact Git commit SHA;
- a clean-worktree assertion captured at build time;
- a deterministic material build-content digest;
- whether `real-execution` is enabled;
- a qualification-capability/contract version; and
- the production runtime-contract version.

The exact Git commit is immutable audit provenance and is the pre-promotion source-state binding. It is not itself the equality key for the `emuchef_build` compatibility dimension: committing a newly recorded evidence bundle must not immediately make that evidence stale. Instead, `emuchef_build` compatibility compares the application version, material build-content digest, `real-execution` state, and qualification-capability/contract version. The runtime-contract version remains its own compatibility dimension.

The material build-content digest is a canonical SHA-256 over repository-owned product/runtime/authored inputs that can materially change planning or execution. It deliberately excludes qualification evidence, the generated qualification matrix, qualification/operator documentation, tests/fixtures, and ignored runtime candidates. Therefore evidence-only or documentation-only commits do not stale otherwise compatible evidence, while material product/runtime/authored changes do.

The repository qualification tool owns the material-input set and digest algorithm so build-time capture and later projection cannot drift. Where practical, deterministic compiled artifact digests may supplement this identity, but they do not replace the exact source commit provenance or material build-content digest.

The operator cannot edit or override build identity or runtime-contract identity. Both are frozen when a qualification session begins and are rechecked before candidate promotion.

For any authoritative registration or evidence-recording operation, the current repository HEAD must still equal the build's embedded commit and the tracked worktree must be clean before the canonical mutation begins. Ignored runtime candidate state does not count as source dirtiness. If HEAD or tracked source state has changed, the candidate may remain inspectable but is not promotable from that build.

The existing operator-supplied build/runtime environment variables are removed from the active qualification workflow once this design is implemented.

## 7. Domain-oriented naming migration

Active qualification artifacts must use domain terminology rather than project-management terminology.

The existing active foundation is migrated approximately as follows:

- `docs/testing/phase-6f/` -> `docs/testing/device-qualification/`
- `tests/fixtures/phase-6f/` -> `tests/fixtures/device-qualification/`
- `tools/phase-6f-qualification.mjs` -> `tools/device-qualification.mjs`
- `tools/phase-6f-qualification.test.mjs` -> `tools/device-qualification.test.mjs`
- `docs/qualification/phase-6f-device-matrix.md` -> `docs/qualification/device-qualification-matrix.md`
- `docs/manual/phase-6f-qualification-operator.md` -> `docs/manual/device-qualification-operator.md`
- `make phase-6f-qualification-check` -> `make device-qualification-check`
- runtime candidates -> `.emuchef_runtime/qualification-candidates/`

New production/runtime source identifiers use domain names such as `QualificationSession`, `QualificationCandidate`, `QualificationTarget`, and `recordQualificationRun`. New source files, runtime directories, APIs, and implementation identifiers must not contain phase/slice nomenclature.

Historical roadmap entries, previously committed Superpowers plans/specs, and historical `.chatgpt` run artifacts remain historical records and may retain the terminology under which they were created.

## 8. Device-target registration

### 8.1 Registration is separate from qualification

A new device target must be registered before it can be used for recordable workflow qualification.

Registration changes canonical repository state, so registration and qualification cannot produce authoritative records from the same build lifecycle:

1. run qualification mode from a clean committed build;
2. capture and review a target-registration candidate;
3. explicitly register the target;
4. commit the resulting canonical registry change;
5. build a new clean qualification-capable application from that commit; and
6. only then run recordable workflow qualification against that target.

This ensures the target definition is part of the exact committed source state represented by the qualifying build.

### 8.2 Target-fact capture

Target registration uses existing production device boundaries wherever they can establish the fact authoritatively.

Machine/trusted facts include, as available through the current production device model:

- matched authored profile ID;
- manufacturer;
- model;
- Android version;
- Android API level;
- ABI/SoC classification;
- firmware/build identity; and
- root state via the existing explicit root-check boundary.

The operator may attest only narrowly modeled fields for which the trusted boundary cannot reliably determine a value. The initial attested field is `connectionType`, restricted to the exact enum `usb2 | usb3`.

Operator input cannot override a contradictory machine-observed value.

If a required field can neither be authoritatively observed nor legally attested, registration fails closed.

### 8.3 Per-fact provenance

Schema version 2 makes provenance durable. Each material target fact is represented as a typed value plus source, for example:

```json
{
  "manufacturer": {
    "value": "AYANEO",
    "source": "production_observation"
  },
  "rootState": {
    "value": "non_root",
    "source": "explicit_root_check"
  },
  "connectionType": {
    "value": "usb3",
    "source": "operator_attestation"
  }
}
```

Allowed provenance sources are initially:

- `production_observation`
- `explicit_root_check`
- `operator_attestation`

The canonical validator owns a per-field legal-source matrix. At minimum:

- `rootState` must use `explicit_root_check` and can never be operator-attested;
- normal device identity facts use `production_observation`; and
- `connectionType` may use `operator_attestation` until a reliable trusted machine observation exists.

Evidence records carry the same registered target facts and provenance so reviewers can audit how every material compatibility fact was established.

### 8.4 Deterministic target identity

Operators do not author target IDs.

The repository qualification tool canonicalizes the material registered target identity and derives:

`device-target-sha256:<64 lowercase hex characters>`

Recapturing the exact same material target facts yields the same ID and is recognized as the existing target. A materially different target identity yields a different ID.

Human-readable manufacturer/model/profile data remains visible in UI and generated documentation; the digest-backed ID is the immutable machine identity.

Policy/projection state such as current support tier does not participate in target identity.

## 9. Schema version 2

The device-target and evidence contracts move to schema version 2.

There is no migration ambiguity for real physical records because no authoritative physical target/evidence records currently exist.

Version 2 must:

- require target-fact provenance;
- use domain-oriented target/run ID prefixes;
- bind the actual production execution-report artifact for every valid run and for any invalid run where that artifact was successfully captured;
- permit an invalid/not-observed audit bundle to omit the execution-report artifact when missing or failed report capture is itself part of why the run is invalid;
- preserve strict exact-field validation;
- fail closed on unsupported schema versions or illegal mixed-version records; and
- maintain deterministic canonicalization/digest behavior.

The workflow catalog remains at its existing schema version unless implementation proves that its structure itself must change. A schema version is bumped only when its own contract changes.

Synthetic version-1 fixtures may be retained only where they explicitly test rejection/backward behavior; active valid fixtures should represent the current schema contract.

## 10. Qualification session lifecycle

### 10.1 Start and bind

A recordable qualification session starts only after enablement/build validation succeeds.

The operator selects:

- one registered target; and
- one canonical workflow from the repository workflow catalog.

The selected workflow determines the expected production recipe composition and required capabilities/prerequisites. Qualification mode preselects that intent and prevents changes that would make the production configuration cease to represent the declared workflow.

The qualification controller does not create an alternate planner. Existing product validation, configuration, planning, and review remain authoritative.

### 10.2 Device identity continuity

The session continuously binds to the selected registered target.

If the connected device disappears, is replaced, or presents facts inconsistent with the registered target, the session becomes permanently invalid. It does not silently rebind or adapt to the newly observed device.

Once invalidated, that session cannot later become a valid qualification run. The operator may explicitly record the invalid run for audit or start a fresh session.

### 10.3 Production workflow execution

The operator continues through the ordinary EmuChef workflow:

1. device discovery/probe/qualification;
2. production configuration and input binding;
3. production review;
4. explicit real-execution confirmation;
5. real production execution;
6. terminal result; and
7. production execution-report export.

Qualification mode cannot:

- auto-accept review;
- auto-confirm irreversible execution;
- fabricate input bindings;
- bypass blockers;
- suppress product errors; or
- manufacture a successful terminal result.

### 10.4 Automated observations

Only automated observations declared by the canonical workflow may be recorded.

Trusted Rust/Tauri derives them from trusted product/device output, including the production execution report. React does not author observation outcomes.

Missing required automated observations invalidate the run rather than being interpreted as product failure.

### 10.5 Human checkpoints

Human checkpoints are captured inside qualification mode.

The workflow catalog remains authoritative for:

- checkpoint IDs;
- instructions;
- fact descriptions;
- allowed outcomes; and
- required/optional status.

The UI renders only declared checkpoints. It cannot invent IDs or outcomes and must not default any checkpoint to pass.

Allowed outcomes remain:

- `pass`
- `fail`
- `unable_to_verify`

A required `unable_to_verify` makes the run `invalid + not_observed`. A required `fail` may support a `valid + failed` qualification result when the rest of the harness remained trustworthy.

Each recorded checkpoint receives its timestamp when the operator explicitly records that outcome.

## 11. Run classification

The qualification controller and canonical validator preserve three terminal semantic classes:

### Valid and passed

`runValidity = valid` and `qualificationOutcome = passed` when all required automated observations and required human checkpoints establish success and no modeled target-wide failure applies.

### Valid and failed

`runValidity = valid` and `qualificationOutcome = failed` only when the harness/session remained valid and the product failure is supported by at least one modeled signal:

- failed automated observation;
- failed human checkpoint; or
- modeled target-wide prerequisite/safety failure.

### Invalid and not observed

`runValidity = invalid` and `qualificationOutcome = not_observed` for harness/infrastructure/evidence-integrity failures, including:

- device identity unverified or changed;
- device disappearance/replacement;
- build identity no longer verifiable;
- canonical definitions inconsistent with the embedded build state;
- missing required automated evidence;
- execution-report export/capture/digest failure;
- required human checkpoint `unable_to_verify`;
- loss of qualification-controller state required to prove the run; or
- inability to reconstruct the candidate deterministically.

Harness faults must never be projected as product qualification failures.

## 12. Candidate persistence

Non-authoritative target/run candidates live under:

`.emuchef_runtime/qualification-candidates/`

Each candidate resides under an opaque trusted identifier and contains sufficient trusted metadata to bind:

- build identity;
- target registration or registered target identity;
- workflow identity/version;
- production review/execution identity;
- authored-content digests;
- runtime-contract identity;
- captured observations;
- human checkpoints;
- production execution-report bytes/digest when applicable; and
- canonical candidate status/digest metadata needed for later verification.

Candidates survive application restart.

On restart, trusted Rust/Tauri revalidates all session-independent invariants before exposing a candidate as promotable. If the current build/source/definition state no longer matches, the candidate remains inspectable but cannot be promoted.

Candidates are not authoritative, are ignored by repository projection, and may be explicitly discarded.

## 13. Immutable evidence bundles

Each recorded qualification run is an immutable directory under:

`docs/testing/device-qualification/evidence/`

A valid run, and any invalid run for which report capture succeeded, has the structure:

```text
docs/testing/device-qualification/evidence/
  qualification-run-sha256:<digest>/
    evidence.json
    execution-report.json
```

An invalid/not-observed audit run may contain only `evidence.json` when the production execution report was never available or its capture/integrity check failed. That exception cannot be used by a valid run.

Whenever `execution-report.json` is present, it must originate from the existing production report/export boundary. Qualification code may perform narrowly defined sanitization required for safe repository storage, but sanitization cannot manufacture success or alter the semantic execution outcome.

`evidence.json` contains a typed artifacts array. A present execution-report entry carries the report SHA-256 and is bound by the record digest. Replacing, altering, or omitting a referenced report invalidates the bundle. Every valid run must contain exactly one execution-report artifact and the required `execution-report` automated observation; an invalid run may omit both when the report could not be captured.

The canonical tool validates the bundle as a unit.

The deterministic run ID uses the domain form:

`qualification-run-sha256:<64 lowercase hex characters>`

To avoid circular hashing, the run ID is derived from a canonical immutable run-identity payload that excludes `runId` and `recordDigest` but includes the material run binding and artifact digests. After inserting the derived run ID, `recordDigest` is computed over the final canonical evidence record with only `recordDigest` itself omitted. The canonical qualification tool owns both operations; React, Rust, and the operator author neither value.

Synthetic fixtures use the same bundle shape under `tests/fixtures/device-qualification/` and remain structurally isolated from production evidence.

## 14. Evidence promotion

### 14.1 Explicit action

No qualification session silently mutates canonical evidence.

At the terminal state, qualification mode shows the candidate as one of:

- valid / passed;
- valid / failed; or
- invalid / not observed.

The operator must explicitly choose **Record qualification run** to promote it.

Invalid candidates are labeled clearly as invalid qualification runs and not product evidence, but the operator may still record them for historical audit.

### 14.2 Promotion boundary

On promotion:

1. trusted Rust/Tauri rechecks the live/current build and candidate binding;
2. trusted Rust/Tauri verifies that repository HEAD still equals the embedded build commit and that the tracked worktree was clean before promotion;
3. trusted Rust/Tauri invokes a fixed operation of `tools/device-qualification.mjs` using only an opaque candidate identity, never a React-supplied arbitrary path;
4. the Node tool independently loads and validates the candidate;
5. the tool validates schema-v2 semantics, workflow/target references, artifact bytes/digests, canonical digests, and run classification;
6. the tool creates the immutable evidence directory using create-new semantics;
7. existing run IDs/directories cannot be overwritten; and
8. projection/matrix generation is completed as part of the same logical recording transaction.

Repository mutation must be atomic from the caller's perspective. A recording failure must not leave a partially authoritative evidence bundle or a knowingly stale generated matrix.

## 15. Canonical target registration

Target registration uses the same authority split.

Trusted Rust/Tauri captures a target-registration candidate and shows a reviewable preview. The operator explicitly chooses **Register device target**.

The canonical Node tool then:

- validates every fact and legal provenance source;
- computes the deterministic target ID;
- recognizes an identical existing target as the same target;
- rejects an ID/content collision;
- updates the canonical device-target registry without rewriting unrelated target records; and
- leaves the repository in a state that must be committed before a new clean qualification build can produce authoritative workflow evidence for that target.

## 16. Compatibility fingerprint

The qualification fingerprint continues to bind the dimensions required by the canonical workflow contract, including as applicable:

- EmuChef build identity;
- workflow version;
- exact authored recipe/content digests;
- runtime contract;
- device profile;
- Android API;
- firmware build;
- ABI/SoC class;
- root state; and
- connection type where modeled.

Schema v2 preserves the value and provenance necessary to audit the material target facts.

For the `emuchef_build` dimension specifically, evidence retains the exact Git commit as audit provenance, but compatibility equality uses the application version, material build-content digest, `real-execution` state, and qualification-capability/contract version. The Git commit itself is deliberately excluded from compatibility equality so recording evidence or changing qualification-only documentation cannot self-invalidate the run.

Compatibility projection remains workflow-specific: only the dimensions declared by that workflow may invalidate otherwise valid historical evidence.

## 17. Current-state projection

Support state remains derived, never authored.

Only `valid` compatible evidence can become current qualification evidence.

For each required workflow, the projector derives one of:

- `qualified`
- `failed`
- `stale`
- `deferred`
- `missing`
- `not_applicable`

Invalid runs remain visible only as historical audit evidence and are never selected as current qualification evidence.

Historical valid evidence that no longer matches an invalidating compatibility dimension produces `stale` when no current valid compatible run exists.

Device support tiers remain derived from workflow states and modeled target-wide failures rather than being authored into evidence.

## 18. Failure handling and safety invariants

The subsystem fails closed.

### Session invariants

A session must become permanently invalid if a required identity/evidence invariant is lost. A later recovery does not retroactively make that session valid.

### Product-vs-harness distinction

A failure in evidence capture, harness orchestration, device identity continuity, or qualification infrastructure is an invalid run. A product failure is recordable only when the evidence infrastructure remained trustworthy.

### Filesystem and process safety

- React never supplies arbitrary repository paths.
- Trusted Rust resolves the fixed repository/runtime roots.
- The Node tool accepts bounded qualification operations rather than an arbitrary shell command.
- Candidate and evidence identities are opaque/content-derived, not path-like operator input.
- Existing canonical records cannot be overwritten.

### Ordinary product behavior

When qualification mode is disabled, qualification functionality has no effect on ordinary application behavior and Node is not a runtime dependency for end users.

## 19. User experience

Qualification mode is a persistent development overlay/controller, not a parallel wizard.

The operator experience should minimize manual duplication while preserving explicit safety decisions:

- choose a canonical target/workflow;
- let EmuChef preselect the canonical production intent;
- supply ordinary workflow inputs through the normal product UI;
- review the normal execution review;
- explicitly confirm real execution through the existing safety contract;
- record only declared human checkpoints when present;
- inspect the terminal candidate summary;
- explicitly record the qualification run.

Target registration likewise captures as much as possible automatically and asks only for legally attested facts the trusted boundary cannot establish.

## 20. Testing strategy

Implementation must prove the harness and repository contracts without running or claiming real physical qualification.

### 20.1 Node/tooling tests

`tools/device-qualification.test.mjs` must cover at least:

- domain-path migration and active-artifact naming;
- schema-v2 device-target validation;
- required per-fact provenance;
- illegal provenance-source rejection;
- deterministic target-ID generation;
- deterministic fingerprint/run/record digests;
- domain-oriented run-ID validation;
- execution-report artifact reference validation;
- missing execution-report rejection for valid runs and for records that reference the artifact;
- invalid/not-observed audit-bundle acceptance when report capture itself failed and no artifact is referenced;
- changed/replaced execution-report rejection;
- create-new immutable target/evidence behavior;
- duplicate/collision handling;
- valid/passed evidence;
- valid/failed evidence;
- invalid/not-observed evidence;
- invalid evidence never becoming current;
- stale derivation after an invalidating fingerprint change;
- required automated-observation enforcement;
- required human-checkpoint enforcement;
- synthetic-fixture isolation; and
- byte-deterministic matrix generation/checking.

### 20.2 Rust/Tauri tests

Tests must cover qualification-specific orchestration without requiring a physical device:

- four-gate qualification enablement;
- clean-build metadata validation;
- non-recordability of dirty/hot-reload builds;
- registered target/workflow binding;
- workflow intent constraint without alternate planning authority;
- device identity mismatch permanently invalidating a session;
- production root-check capture;
- typed `usb2 | usb3` operator attestation;
- rejection of illegal operator override;
- candidate persistence;
- restart recovery;
- stale/non-promotable candidate classification;
- exact production report capture/binding;
- candidate artifact tamper rejection;
- fixed qualification-tool invocation with opaque candidate IDs;
- no arbitrary filesystem path acceptance; and
- registration and workflow qualification requiring separate clean-build cycles.

### 20.3 Frontend tests

Tests must prove the overlay is not an alternate product flow:

- qualification UI absent when disabled;
- existing product screens remain authoritative;
- canonical workflow constrains recipe intent;
- review cannot be skipped;
- real-execution confirmation cannot be skipped;
- only declared human checkpoints render;
- checkpoints have no default pass state;
- valid/failed and invalid/not-observed are visually and semantically distinct;
- target registration requires review and explicit action; and
- qualification evidence recording always requires explicit operator action.

### 20.4 Repository integration

The active validation surface becomes:

```sh
node --test tools/device-qualification.test.mjs
node tools/device-qualification.mjs --check
make device-qualification-check
```

`make test` continues to depend on the domain-named qualification check.

CI/local validation must continue to require no connected hardware for the automated harness tests.

## 21. Acceptance criteria

The implementation is acceptable when all of the following are true:

1. active qualification artifacts and source/runtime identifiers use domain terminology rather than phase/slice nomenclature;
2. target/evidence contracts are schema version 2 with required legal provenance;
3. the repository qualification tool remains the single canonical schema/digest/projection authority;
4. qualification mode exists only behind debug + `real-execution` + valid clean-build metadata + `EMUCHEF_DEVICE_QUALIFICATION=1`;
5. a new physical target can be captured from trusted production observations, minimally attested where necessary, reviewed, and explicitly registered;
6. deterministic target IDs are generated by the canonical tool;
7. target registration requires commit/rebuild before recordable workflow qualification;
8. a qualification session binds one registered target and one canonical workflow while preserving normal production review/execution authority;
9. device identity drift permanently invalidates the session;
10. human checkpoints are captured inside qualification mode strictly from catalog declarations;
11. candidates persist under `.emuchef_runtime/qualification-candidates/` and can survive restart without becoming authoritative;
12. every valid run preserves the exact sanitized production execution report as a digest-bound artifact, while an invalid/not-observed audit run may omit it only when report capture itself failed or never became available;
13. explicit promotion creates immutable create-new evidence bundles with deterministic domain-oriented run IDs;
14. valid product failures and invalid infrastructure/harness runs remain distinct;
15. invalid runs may be explicitly recorded for audit but cannot affect current qualification state;
16. projection and generated matrix remain deterministic and are updated atomically with successful evidence recording;
17. ordinary EmuChef behavior is unchanged when qualification mode is disabled; and
18. no physical device or workflow is claimed qualified merely because this implementation exists.

## 22. Implementation constraints

The implementation plan must preserve these constraints:

- use the existing production EmuChef device/configuration/review/real-execution/report boundaries;
- do not add qualification-only planner/executor/device authority;
- do not auto-confirm irreversible execution;
- do not duplicate the Node qualification validator in Rust;
- do not expose arbitrary filesystem/process authority to React;
- do not introduce phase/slice terminology in new active source paths, APIs, tooling names, runtime directories, or durable qualification data paths;
- retain historical planning artifacts as historical records rather than rewriting them for naming purity; and
- do not perform real physical qualification during implementation unless separately requested by the owner.
