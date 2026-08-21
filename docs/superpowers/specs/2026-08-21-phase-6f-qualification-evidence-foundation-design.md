# Phase 6F Qualification Evidence Foundation Design

## 1. Purpose

Phase 6F establishes repository-owned infrastructure for physical-device qualification without performing or claiming any new physical qualification in this task.

The primary qualification unit is **device target x workflow**. A qualification run binds that unit to a versioned compatibility fingerprint and produces immutable evidence. Current support state is derived deterministically from applicability plus the newest compatible evidence; it is never manually assigned.

This design intentionally separates:

1. qualification intent;
2. immutable physical-run evidence; and
3. derived current support state.

A matching authored device profile does not itself imply support.

## 2. Scope

### 2.1 In scope

This foundation defines and implements the contracts required for later physical-device qualification:

- a canonical qualification-workflow catalog;
- device-target identity and observed hardware/software facts;
- applicability classification for each device target and workflow;
- a structured, versioned compatibility fingerprint;
- deterministic evidence-validity and staleness rules;
- immutable machine-readable qualification-run records;
- typed automated observations and human checkpoints;
- qualification-run validity versus product qualification failure;
- workflow-level qualification state;
- derived device support tiers;
- repository validation of definitions and evidence;
- deterministic generation of a human-readable qualification matrix;
- strict separation of synthetic fixtures from real physical evidence; and
- documentation of the operator-facing validation and matrix-generation workflow.

### 2.2 Explicitly out of scope

This task does not:

- perform a physical-device qualification run;
- create evidence that represents a real physical qualification;
- change Phase 6E recipe-qualification semantics;
- make Phase 6E automated evidence satisfy a Phase 6F physical requirement;
- enable real execution in production builds;
- change public product APIs solely for qualification convenience;
- add alternate planner, executor, or device-authority paths;
- claim broad device compatibility from authored profile matching;
- resume the owner-deferred Phase 6D physical/manual evidence collection; or
- begin Phase 6G release promotion.

## 3. Core architecture

The conceptual evidence key is:

`device target x workflow x compatibility fingerprint -> immutable qualification run`

The repository owns three distinct layers.

### 3.1 Definitions

Definitions describe what qualification means. They include:

- canonical workflow definitions;
- workflow versions;
- production-intended recipe or composition references;
- capability and prerequisite requirements;
- applicability rules;
- compatibility dimensions;
- automated observation requirements;
- typed human-checkpoint contracts;
- success/failure criteria; and
- narrowly defined target-wide prerequisite or safety failure classes.

Definitions are versioned, reviewable repository data. Individual physical runs cannot redefine their own qualification scope.

### 3.2 Evidence

Evidence records what happened during a physical run. Records are immutable once committed. A failed run remains evidence and is never overwritten or discarded merely because a later run succeeds.

Evidence contains captured facts and outcomes. It does not contain a manually assigned overall support tier.

### 3.3 Projection

A deterministic validator/projector consumes current definitions plus committed physical evidence and derives:

- workflow applicability;
- evidence validity and staleness;
- workflow-level state;
- overall device support tier; and
- the generated human-readable qualification matrix.

Historical evidence is never mutated when it becomes stale. Only its current interpretation changes.

## 4. Device target identity

A Phase 6F device target binds an authored device profile to observed physical facts.

The target identity must capture, at minimum, the material facts needed to describe what hardware/software configuration was qualified:

- authored device-profile identifier;
- manufacturer;
- model or product identity sufficient to distinguish materially different hardware variants;
- Android version;
- Android API level;
- ABI class;
- SoC or hardware class when material to qualification;
- root state;
- connection type when material to the workflow;
- firmware/build identity when available and material; and
- any additional workflow-relevant environment fact explicitly defined by the qualification model.

The authored profile is product context, not sole identity. Two physical configurations that match the same profile may require distinct evidence if a compatibility dimension differs materially.

## 5. Compatibility fingerprint

### 5.1 Structured representation

A compatibility fingerprint is a structured, versioned set of material inputs. It must remain inspectable. An opaque digest may be derived for stable comparison or indexing, but the constituent fields are authoritative.

The fingerprint includes only dimensions that can materially change whether evidence remains applicable, including as relevant:

- EmuChef build identity;
- qualification-workflow version;
- relevant authored recipe or composition content digests;
- applicable execution/runtime contract version;
- authored device-profile identity or digest where material;
- Android/API facts;
- firmware/build identity;
- hardware/ABI/SoC class;
- root state; and
- other explicitly workflow-relevant environment facts.

### 5.2 Compatibility classification

Changes are classified deterministically as one of:

- **Compatible**: existing evidence remains current.
- **Invalidating**: existing evidence becomes stale and requalification is required.
- **Not applicable**: the changed dimension has no bearing on the workflow.

The qualification system owns these rules. Operator judgment is not authoritative for whether committed evidence is current.

The system must not use a blanket "repository changed, rerun everything" rule.

## 6. Canonical workflow catalog

Phase 6F defines a canonical workflow catalog rather than treating every authored recipe as an independent physical qualification target.

Each workflow definition includes:

- stable workflow ID;
- workflow version;
- user-visible purpose;
- production-intended recipe or composition exercised;
- capability requirements;
- prerequisites;
- required automated observations;
- required human checkpoints, if any;
- explicit success/failure criteria; and
- compatibility dimensions relevant to that workflow.

The catalog is grounded in production-intended EmuChef user workflows. It may represent a composition such as RetroArch plus BIOS as one end-user qualification workflow where that is the meaningful product behavior.

Phase 6E automated recipe qualification can support confidence and preconditions for a Phase 6F workflow, but it cannot satisfy the physical qualification requirement.

## 7. Applicability model

For each device target and canonical workflow, applicability is deterministically derived as one of:

- **Required**: production intent and device capabilities make the workflow part of the device support claim.
- **Not applicable**: the workflow legitimately does not apply, with a deterministic reason.
- **Deferred**: the workflow applies, but physical qualification has intentionally not yet been performed, with an explicit reason.

`Deferred` never counts as passing evidence.

Per-device free-form workflow selection is not authoritative. Applicability comes from the repository-owned catalog, production intent, and modeled device capabilities/prerequisites.

## 8. Physical-run harness boundary

The Phase 6F harness orchestrates qualification, but production EmuChef remains the system under test.

The harness must not introduce alternate planning or execution behavior to make qualification easier. Where possible, it exercises the same externally meaningful workflow boundary used by the real product. Qualification-only adapters may capture observations or package evidence but cannot replace production authority.

A run follows this lifecycle:

1. validate the qualification target and workflow definition;
2. capture device identity, prerequisites, EmuChef build identity, and authored-content fingerprints;
3. validate the compatibility fingerprint inputs before execution;
4. capture a pre-run environment snapshot;
5. exercise the production workflow;
6. capture structured execution results, logs, relevant device observations, timestamps, and post-run state;
7. collect typed human checkpoints only where automation cannot reliably establish the required fact;
8. validate the complete evidence package against the workflow success contract;
9. write a new immutable run record; and
10. recompute the derived qualification matrix.

## 9. Human checkpoints

Human checkpoints are typed evidence, not free-form operator prose.

Each checkpoint definition has:

- stable checkpoint ID;
- explicit operator instruction;
- a clear statement of the fact being established;
- allowed outcomes such as `pass`, `fail`, and `unable_to_verify`;
- whether the checkpoint is required for qualification;
- bounded note or attachment metadata where justified; and
- validation rules for the recorded result.

A missing required checkpoint or `unable_to_verify` result cannot produce a `Qualified` workflow state.

Human checkpoints are used only for facts automation cannot establish reliably. Automation remains responsible for all observations it can prove deterministically.

## 10. Run validity versus qualification failure

The system distinguishes a valid product qualification failure from an invalid qualification run.

Examples of **qualification failure** include a production workflow completing with a product failure or a required product/device observation failing its criterion.

Examples of **invalid run/infrastructure failure** include:

- malformed workflow definitions;
- inability to initialize the evidence recorder;
- missing mandatory pre-run identity inputs;
- corrupted or internally inconsistent evidence packaging; or
- harness failures that prevent the product behavior from being observed reliably.

An invalid run does not become product-failure evidence.

A transport failure caused by the product workflow may be legitimate qualification evidence; an unrelated harness transport failure may instead invalidate the run. The distinction must be typed and deterministic where possible.

## 11. Workflow-level state

For each applicable device-target/workflow pair, the projector derives one current state:

- **Qualified**: the newest current compatible valid evidence passes every required automated assertion and human checkpoint.
- **Failed**: the newest current compatible valid evidence contains a qualification failure.
- **Stale**: historical evidence exists, but no evidence remains compatible with the current fingerprint requirements.
- **Deferred**: applicability is explicitly deferred.
- **Missing**: the workflow is required but no applicable valid physical evidence exists.
- **Not applicable**: applicability rules exclude the workflow.

Historical runs remain available regardless of current state.

## 12. Device support tiers

The overall device-target support tier is a pure deterministic projection. It is not authored directly in evidence.

### 12.1 Qualified

A device target is **Qualified** when every required workflow is currently `Qualified` and no target-wide prerequisite or safety failure applies.

### 12.2 Limited

A device target is **Limited** when at least one required workflow is `Failed`, `Stale`, `Deferred`, or `Missing`, while meaningful currently qualified functionality remains and no target-wide prerequisite or safety failure makes the target unusable as a whole.

Successful workflow evidence remains qualified even when another required workflow fails.

### 12.3 Unqualified

A device target is **Unqualified** when either:

- no meaningful current qualification exists; or
- an explicitly modeled fundamental prerequisite or safety failure invalidates the target as a whole.

Target-wide escalation is intentionally narrow. The system must not include a generic mechanism that allows arbitrary workflow failures to mark a whole target `Unqualified`.

## 13. Immutable evidence and supersession

Completed physical-run records are immutable.

Later runs may supersede older evidence for current-state projection, but they do not rewrite history. The projector chooses current applicable evidence deterministically using workflow identity/version, device-target identity, compatibility rules, run validity, and run ordering metadata.

Older evidence may remain:

- compatible but superseded;
- stale;
- failed;
- invalid; or
- historically relevant.

The human-readable matrix shows current state while preserving traceability to the underlying run identity and date.

## 14. Machine-readable storage

The repository stores physical qualification records in a strict versioned machine-readable format. JSON is the preferred initial representation unless an existing repository convention provides a stronger fit during implementation.

Definitions and evidence must be stored separately so a run cannot redefine its governing contract.

Synthetic fixtures must live under a separate test-fixture path and must be structurally impossible for the production matrix generator to consume as real qualification evidence.

Real physical evidence paths must never be populated by foundation tests.

## 15. Validation

Repository validation must detect at least:

- malformed definition or evidence schema;
- unsupported schema versions;
- duplicate stable IDs;
- duplicate immutable run identities;
- missing referenced workflow IDs;
- missing referenced device-profile IDs;
- invalid workflow versions;
- invalid applicability state/reason combinations;
- incomplete compatibility fingerprints;
- inconsistent or non-deterministic fingerprint digests;
- invalid checkpoint IDs or outcomes;
- missing required checkpoint results;
- impossible workflow/result combinations;
- malformed run-validity classification;
- invalid supersession/current-evidence selection;
- synthetic evidence entering the production evidence set; and
- generated-matrix drift from the canonical definitions and evidence.

Validation must fail closed rather than silently dropping malformed evidence.

## 16. Generated qualification matrix

A deterministic generator produces a human-readable current qualification matrix from canonical definitions and real evidence.

The generated matrix includes at minimum:

- device target and observed configuration;
- authored device-profile context;
- overall `Qualified`, `Limited`, or `Unqualified` tier;
- every applicable canonical workflow;
- current workflow state;
- current evidence run identity and date where present;
- stale, deferred, missing, or failure reason where applicable; and
- documented support limitations relevant to the current projection.

The generated matrix is a projection, not an independent source of truth. CI verifies that the committed/generated matrix matches the canonical inputs exactly.

## 17. Operator workflow

The foundation must document the exact supported operator workflow for later physical qualification, including how to:

1. identify the canonical workflow and device target;
2. validate prerequisites;
3. start or invoke the qualification harness;
4. complete any typed human checkpoints;
5. validate the resulting evidence package;
6. persist an immutable run record;
7. regenerate the human-readable matrix; and
8. run repository validation before committing evidence.

This design does not require the first foundation task to perform these steps against a real device.

## 18. Testing strategy

The implementation should use test-first development and synthetic fixtures to prove the evidence model without fabricating real qualification.

Automated tests should cover at least:

- schema acceptance and rejection;
- workflow applicability derivation;
- compatibility classification;
- stale-evidence derivation;
- immutable run identity handling;
- current-evidence selection across multiple historical runs;
- required human-checkpoint semantics;
- distinction between invalid run and qualification failure;
- workflow-level state derivation;
- `Qualified`, `Limited`, and `Unqualified` device-tier derivation;
- narrow target-wide escalation rules;
- deterministic matrix generation;
- generator rejection of synthetic fixture paths as production evidence; and
- matrix drift detection.

No test fixture may be presented as physical evidence in generated product documentation.

## 19. Product and authority invariants

The Phase 6F foundation preserves these invariants:

- Rust remains product authority for planning, execution, device facts, filesystem behavior, and validation where those responsibilities already exist.
- Qualification infrastructure observes production behavior rather than creating alternate product behavior.
- Authored device-profile matching is not equivalent to physical support.
- Phase 6E automated recipe qualification does not satisfy Phase 6F physical evidence requirements.
- Support tiers are derived, never manually asserted in run evidence.
- Historical evidence is immutable.
- Staleness changes interpretation, not history.
- A workflow failure does not erase unrelated successful evidence.
- Only explicitly modeled prerequisite/safety failures may invalidate a whole target.
- Deferred work is visible and never counted as passing.
- Synthetic fixtures cannot contaminate real qualification state.
- This task makes no new physical-device qualification claim.

## 20. Completion criteria for the foundation task

The Phase 6F qualification evidence foundation is complete when:

1. versioned workflow, device-target, fingerprint, evidence, checkpoint, applicability, and projection contracts exist;
2. deterministic applicability and compatibility rules are implemented and tested;
3. immutable run records can be validated without performing a physical run;
4. workflow state and device support tiers derive deterministically from synthetic fixtures;
5. the validator rejects malformed, ambiguous, or synthetic-as-real evidence;
6. the generated human-readable matrix is deterministic and drift-checked;
7. the operator workflow is documented;
8. existing production planning/execution authority is unchanged; and
9. no real physical qualification record or support claim is added by this task.
