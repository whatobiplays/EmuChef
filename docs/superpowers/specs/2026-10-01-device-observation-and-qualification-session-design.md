# Device Observation and Qualification Session Architecture

**Date:** 2026-10-01  
**Status:** Approved design  
**Scope:** EmuChef proper's trusted passive device-observation seam and the development-only physical qualification-session lifecycle.

## 1. Purpose

The existing device-qualification harness is intentionally layered over the ordinary EmuChef product workflow, but part of its lifecycle is currently reconstructed in React. `useDeviceQualificationMode.ts` observes workflow state and separately coordinates device reassociation, review binding, execution binding, retries, deduplication, and terminal candidate finalization. The trusted Rust implementation then re-reads production state to validate those requests.

That shape has two problems:

1. lifecycle ordering is distributed across React effects, opaque handles, retry maps, and Rust validation; and
2. passive production device observation is split across command functions and qualification-specific consumers, leaving trusted facts represented as untyped JSON at an internal seam.

This design deepens two modules in one cohesive architecture:

- **`device_observation`** owns typed passive device observation and interpretation used by ordinary product paths and internal qualification tooling.
- **`qualification_session`** owns the lifecycle of one internal evidence-capture attempt and observes exact authoritative product transitions at the trusted Tauri orchestration seam.

The ordinary EmuChef workflow remains authoritative. Qualification never becomes a second product workflow, planner, device authority, review authority, execution authority, or root authority.

## 2. Goals

The implementation must:

1. remove qualification lifecycle synchronization from React;
2. make authoritative Rust/Tauri product transitions the source of qualification lifecycle observations;
3. represent trusted passive device observations with typed internal Rust values rather than `serde_json::Value`;
4. keep explicit root probing under its existing production authority;
5. keep at most one active qualification session;
6. persist and recover qualification-session state through one deep module;
7. make evidence validity fail closed if any authoritative transition cannot be captured durably;
8. preserve ordinary product outcomes when qualification-only capture fails;
9. automatically materialize immutable qualification candidates when terminal evidence is complete or an attempt becomes invalid;
10. remove lifecycle-shaped qualification IPC that exists only to let React coordinate trusted state;
11. preserve simulation-first product behavior; and
12. keep qualification inactive behavior effectively free of qualification persistence or repository work.

## 3. Non-goals

This work does not:

- redesign EmuChef's planner, review, executor, or root authority;
- introduce an internal event bus;
- introduce a background qualification queue or write-ahead event journal;
- add multiple simultaneous qualification sessions;
- make qualification a product persona or end-user workflow;
- change the canonical repository evidence authority in `tools/device-qualification.mjs`;
- change device-target identity, evidence bundle schemas, deterministic IDs, or matrix projection except for the qualification-session persistence contract owned by the app;
- migrate an old active session into a valid new session when its lifecycle history cannot be proven;
- make simulated execution qualification evidence;
- perform or claim physical qualification;
- broaden the work into the larger `App.tsx` orchestration refactor; or
- split the approved implementation into multiple PRs.

The broader `App.tsx` orchestration architecture should be reassessed after this change lands.

## 4. Domain model

### 4.1 Device observation

A **device observation** is the trusted passive interpretation of the currently selected device used by ordinary EmuChef product behavior. It includes typed device facts, profile matching, and passive support/capability interpretation.

A device observation may consume already-established root state where a caller needs to project it, but the module must never initiate a root check.

### 4.2 Qualification session

A **qualification session** is one internal evidence-capture attempt bound to:

- one registered qualification target;
- one canonical qualification workflow;
- one ordinary product device-plan intent;
- the workflow's required recipe intent;
- one trusted build/runtime identity; and
- one sequence of authoritative product observations.

There may be at most one active qualification session in EmuChef proper.

A qualification session observes the ordinary workflow. It does not authorize the ordinary workflow.

### 4.3 Qualification candidate

A **qualification candidate** is an immutable stored result produced from a closed qualification attempt. It is either:

- valid/passed;
- valid/failed; or
- invalid/not-observed.

Candidate creation is classification of already-observed facts, not an operator judgment. Recording a candidate into canonical repository evidence remains an explicit operator action through the existing repository authority.

## 5. Trust and ownership

### 5.1 Ordinary product operations remain authoritative

Device selection/probing, review creation, explicit root checking, real-execution admission, and execution terminal state remain ordinary production operations.

When qualification is active, the trusted Tauri orchestration layer performs a direct qualification-session observation **after the authoritative product operation has committed and before that product result is returned or published to React**.

The qualification observer receives the exact typed authoritative result that just committed. It must not reconstruct that result from React state and should not re-query a second authority merely to rebuild historical state.

### 5.2 Qualification failure cannot rewrite product truth

If the ordinary product operation succeeds but qualification observation or persistence fails:

- the ordinary product operation remains successful;
- the qualification session becomes permanently non-recordable;
- an invalid/not-observed candidate is materialized when durable persistence is available; and
- the development qualification surface shows the qualification failure.

Qualification tooling must never turn a successful review, root check, or real execution transition into a product failure solely because evidence capture failed.

### 5.3 Inactive behavior

When qualification mode is unavailable or no active session exists, observation hooks are allocation-light no-ops:

- no qualification repository I/O;
- no qualification-session creation;
- no hidden evidence mutation; and
- no change to ordinary command results.

The same ordinary orchestration path is used in development and production; qualification behavior is dormant rather than implemented through a second product path.

## 6. `device_observation` module

### 6.1 Responsibilities

The module owns the complete passive device-observation path:

- live device fact probing through the existing trusted ADB/runtime authority;
- retained typed device facts for the current native session;
- profile matching;
- passive support/capability interpretation;
- consistency rules between those representations; and
- sanitized projections for Tauri/React callers.

The module does not own:

- explicit root probing;
- review creation;
- planning;
- execution; or
- qualification-session state.

### 6.2 Interface shape

The interface should expose a small set of semantically distinct operations rather than one parameterized mega-operation. The exact Rust names may follow existing conventions, but the interface must preserve these conceptual operations:

1. refresh/probe typed device facts;
2. match a retained trusted observation against authored device profiles; and
3. project current passive support/capability state.

Callers should not know:

- where facts are retained;
- how sidecar/runtime requests are formed;
- how JSON responses are decoded;
- how profile matching is assembled; or
- how passive support/capability consistency is maintained.

JSON/DTO conversion belongs only at IPC, sidecar, or durable repository/tooling seams. Qualification consumes typed trusted observations directly.

### 6.3 Root is a separate authority

The existing explicit production root check remains separate.

The device-observation module may accept or expose already-authorized typed root state when needed for a projection, but it must not:

- invoke `su`;
- trigger a root check;
- infer root from passive device facts; or
- duplicate root classification.

This preserves the established distinction between passive device observation and explicit root authority.

## 7. `qualification_session` module

### 7.1 Responsibilities

The module owns:

- active-session cardinality;
- session start validation;
- persisted-session versioning;
- device association and clean restart reassociation;
- canonical workflow and target binding;
- declared intent locking;
- ordered lifecycle observations;
- required checkpoint state;
- review/execution association;
- terminal execution classification;
- automatic candidate materialization;
- monotonic invalidation;
- clean-shutdown resumability;
- crash/unproven-shutdown invalidation;
- abandonment;
- persistence/recovery orchestration; and
- sanitized session status presented to React.

It does **not** own canonical target/evidence recording rules. `QualificationRepository` and `tools/device-qualification.mjs` retain those responsibilities.

### 7.2 Session start

Beginning a session remains an explicit operator action.

The request continues to carry:

- `deviceHandle`;
- `devicePlan`;
- `targetId`; and
- `workflowId`.

These are requested intent, not trusted restatements of product facts.

On begin, Rust must:

1. resolve the registered target from canonical repository state;
2. resolve the canonical qualification workflow;
3. use `device_observation` to validate the current selected device;
4. validate the selected device plan against the target/workflow contract;
5. derive required recipes from the canonical workflow definition;
6. capture current trusted build/runtime identity; and
7. persist the authoritative session before returning it.

A new opaque ordinary-workflow-intent handle must not be introduced solely for qualification.

### 7.3 Closed lifecycle observation interface

Trusted Tauri orchestration sends one closed qualification-specific observation type to the module, conceptually:

```text
observe(QualificationLifecycleObservation)
```

The closed variants must cover the authoritative transitions needed by the session, including:

- a selected/probed device observation;
- an explicit root-check result;
- creation of the authoritative review used by the workflow;
- admission/start of a real execution; and
- authoritative terminal real-execution state.

This is a direct module call, not a generic event bus.

Explicit operator actions stay separate interface entries:

- begin session;
- record checkpoint;
- abandon attempt;
- record candidate;
- discard candidate;
- capture/register target; and
- refresh/read sanitized status.

### 7.4 Ordering

Each observation is synchronous with its authoritative product transition:

1. product operation validates and commits;
2. Tauri invokes the qualification-session observer with the exact typed committed result;
3. the session persists its new state or becomes poisoned/non-recordable;
4. only then is the product result returned/published to React.

A qualification failure in steps 2-3 does not change step 1's product result.

Execution terminal capture occurs at the authoritative transition where the terminal execution report/state is retained. Polling, exporting, or opening execution UI must never advance qualification lifecycle state.

### 7.5 Simulation

Simulated execution remains ordinary product behavior and is ignored as qualification lifecycle evidence.

A qualification session may coexist with simulation. Simulation:

- does not bind a qualification execution;
- does not close the session;
- does not invalidate the session; and
- does not satisfy an automated execution-report observation.

The operator may subsequently perform the explicitly confirmed real execution from the ordinary product workflow.

## 8. Checkpoint semantics

### 8.1 Prerequisite checkpoints

A human checkpoint that is both required and declared as a workflow prerequisite must be recorded as `pass` before authoritative real-execution admission.

Recording `fail` or `unable_to_verify` for a prerequisite checkpoint immediately invalidates the attempt.

If the ordinary product begins real execution before every required prerequisite checkpoint has passed:

- the product execution still proceeds according to ordinary product rules;
- qualification does not block or cancel it;
- the qualification attempt becomes invalid/not-observed.

Qualification remains an observer, not execution authority.

### 8.2 Non-prerequisite required checkpoints

A required checkpoint that is not a prerequisite may be recorded after terminal execution.

If terminal execution arrives while required non-prerequisite checkpoints are still missing, the session enters a **terminal-awaiting-evidence** state:

- execution identity and terminal report are fixed;
- no new execution may bind to that session;
- the operator may record the remaining declared checkpoints; and
- candidate materialization occurs automatically once all required evidence is present.

A truthful `fail` result for a non-prerequisite checkpoint may contribute to a valid failed qualification outcome. It is not automatically an infrastructure-invalid attempt.

## 9. Candidate materialization

### 9.1 Automatic terminal materialization

When authoritative terminal execution is present and every required observation/checkpoint is complete, the module automatically creates an immutable candidate and closes the active session.

Classification is:

- successful product execution + passing required evidence -> valid/passed;
- terminal product failure or failed non-prerequisite product evidence -> valid/failed;
- missing/corrupted/untrusted lifecycle evidence -> invalid/not-observed.

There is no operator `finalizeQualificationCandidate` lifecycle step.

### 9.2 Invalid attempts

When an attempt becomes invalid before normal terminal completion, the module should automatically materialize an immutable invalid/not-observed candidate when enough session identity exists and durable candidate persistence succeeds.

Examples include:

- incompatible device observation;
- product intent drift;
- missed authoritative transition;
- qualification persistence failure;
- failed/unverifiable prerequisite;
- incompatible persisted-session version;
- unproven prior shutdown;
- operator abandonment.

The operator may explicitly record the invalid candidate as audit evidence or discard it.

### 9.3 Persistence failure while invalidating

If invalid-candidate persistence itself fails:

- poison the active session in memory immediately;
- mark it non-recordable;
- never report that an invalid candidate exists unless durable storage proves it;
- never affect the successful ordinary product operation; and
- rely on persisted-session version/clean-handoff recovery rules so the old session cannot later become valid evidence.

Evidence integrity is mandatory. Completeness of internal invalid-attempt audit history is secondary.

## 10. Invalidation model

The implementation must use a closed typed invalidation-reason model internally. Arbitrary exception text is not durable state.

The reason model must distinguish at least the semantic classes needed to test:

- incompatible device observation;
- changed product intent;
- missing or out-of-order required prerequisite;
- missed authoritative lifecycle transition;
- qualification observation/persistence failure;
- incompatible persisted-session version;
- unproven prior application shutdown; and
- operator abandonment.

The exact enum case names are implementation details.

React receives only a stable sanitized operator explanation and recordability state. Raw filesystem, ADB, repository, exception, or arbitrary backend text must not cross IPC.

Invalidation is monotonic for a session. A later successful observation cannot restore a valid evidence attempt.

## 11. Persistence and recovery

### 11.1 Ownership

`qualification_session` owns **when and why** session state is:

- persisted;
- loaded;
- version-checked;
- reassociated;
- invalidated;
- closed; and
- converted into a candidate.

`QualificationRepository` remains the storage adapter and repository-evidence interface. Callers outside the session module do not coordinate session persistence.

### 11.2 Persisted-session version

The new lifecycle contract requires a new persisted-session version.

Pre-refactor active sessions cannot prove that every authoritative transition was captured under the new observation contract. They must therefore fail closed:

- already-materialized candidates remain intact;
- already-recorded canonical evidence remains intact;
- an old resumable active session is not migrated into a recordable new session;
- it becomes invalid/not-observed and requires a new attempt.

### 11.3 Clean handoff requirement

An active session may resume only when the application can prove the prior process ended through the existing native clean-handoff/termination contract.

On next launch:

- proven clean handoff and matching captured build identity -> a valid,
  version-compatible active session may resume;
- a valid session captured by another build remains deferred and unchanged;
- returning to the matching captured build may resume that valid session; recovery
  never relabels execution evidence from one build as evidence from another;
- an already-invalid session recovered under another build can only become a
  non-promotable `invalid/not_observed` audit candidate;
- crash, forced termination, missing marker, ambiguous termination, or other
  unproven shutdown -> the active session becomes invalid/not-observed and is
  retained only as a non-promotable audit candidate.

This avoids adding a qualification-specific event journal while preserving the invariant that no missed authoritative transition can later yield valid evidence.

### 11.4 Restart device reassociation

Device handles are process-local.

After a clean restart, the first authoritative selected/probed device observation automatically attempts reassociation:

- compatible target/workflow observation -> associate the new process-local handle and continue;
- conflicting observation -> monotonically invalidate the session.

React performs no special refresh/reassociation choreography.

## 12. Frontend contract

React remains presentation plus explicit operator intent.

The qualification controller may:

- load sanitized qualification status;
- show build/recordability state;
- select registered target and canonical workflow;
- begin a session;
- capture/register a target candidate;
- record declared human checkpoints;
- abandon an active attempt;
- record or discard an immutable candidate; and
- display projected intent/device-selection locks and invalidation state.

React must not:

- detect lifecycle transitions by comparing workflow snapshots;
- decide when a review is bound;
- decide when an execution is bound;
- finalize a candidate;
- retry trusted lifecycle transitions;
- deduplicate authoritative transition handling; or
- infer whether restored process-local handles remain valid.

The current lifecycle-shaped IPC is removed:

- `refresh_qualification_session`;
- `bind_qualification_review`;
- `bind_qualification_execution`; and
- `finalize_qualification_candidate`.

The current TypeScript counterparts are removed as well.

The hook should become a small presentation/operator-action adapter rather than a product-state synchronization engine.

## 13. Testing contract

### 13.1 Rust is the lifecycle test surface

Lifecycle behavior must be tested through the qualification-session module interface.

Tests must cover:

- one-active-session cardinality;
- begin validation against trusted device observation;
- exact ordered lifecycle observations;
- idempotent duplicate observation handling where product orchestration can legitimately replay the same committed result;
- incompatible/out-of-order observation invalidation;
- prerequisite checkpoint ordering;
- non-prerequisite terminal-awaiting-evidence behavior;
- valid passed candidate materialization;
- valid failed candidate materialization;
- invalid/not-observed materialization;
- operator abandonment;
- qualification persistence failure without product failure;
- invalid-candidate persistence failure and in-memory poisoning;
- clean restart recovery;
- unproven-shutdown invalidation;
- process-local device reassociation;
- pre-refactor persisted-session rejection; and
- inactive no-op behavior.

### 13.2 Device-observation module tests

Test the module through its production interface for:

- typed probe conversion;
- retained observation consistency;
- profile matching;
- passive support/capability projection;
- root-state consumption without initiating root checks; and
- sanitized IPC projection.

### 13.3 Frontend tests

Keep tests for:

- development-only visibility;
- status/candidate rendering;
- target/workflow selection;
- explicit begin/checkpoint/abandon/record/discard actions;
- projected intent/device locks;
- invalidation messaging;
- accessibility/focus behavior; and
- one small integration proof that ordinary authoritative operations update qualification state without frontend lifecycle calls.

Delete superseded tests whose only purpose is proving React effect ordering, retry maps, promise deduplication, or lifecycle binding calls.

## 14. Rejected alternatives

### Frontend synchronization

Rejected because it duplicates temporal knowledge outside the trusted authority and already produced reassociation/order complexity.

### Generic event bus

Rejected because there is only one qualification observer and no independently varying adapter. Direct typed calls provide more locality with less interface.

### Async queue or write-ahead qualification journal

Rejected for this scope. Synchronous post-commit observation plus clean-handoff fail-closed recovery is sufficient for internal repeatable evidence work.

### Multiple active sessions

Rejected because EmuChef proper has one ordinary connected-device workflow. Multiple sessions would add routing ambiguity with no product need.

### Re-querying authoritative modules after transitions

Rejected because the exact typed committed result is already available and is the strongest binding to what actually happened.

### Migrating old active sessions

Rejected because field compatibility cannot prove lifecycle completeness under the new invariant.

### Blocking ordinary product operations on qualification failure

Rejected because qualification is evidence tooling, not product authority.

## 15. Implementation unit

The approved implementation is **one PR**.

Within that PR, the implementation should still follow dependency order:

1. establish typed `device_observation`;
2. migrate ordinary production consumers onto it;
3. establish `qualification_session` using that typed seam;
4. wire synchronous authoritative lifecycle observations at Tauri orchestration points;
5. version/recover/fail-closed persisted sessions;
6. remove lifecycle IPC and React synchronization;
7. add abandonment and terminal-awaiting-evidence presentation;
8. replace superseded tests; and
9. update operator documentation.

The PR should be reviewed as one coherent authority/locality refactor. It must not expand into the broader `App.tsx` orchestration redesign.
