# Device Observation and Qualification Session Implementation Specification

**Date:** 2026-10-01  
**Status:** Approved implementation contract  
**Depends on:** `2026-10-01-device-observation-and-qualification-session-design.md`  
**Delivery:** One cohesive PR; no staged/committed changes by the implementation agent unless separately instructed.

## 1. Outcome

Deepen the trusted Rust/Tauri architecture so passive device interpretation and qualification-session lifecycle no longer leak into React orchestration.

At completion:

- production device observation is typed and centralized behind `device_observation`;
- one `qualification_session` module owns evidence-session lifecycle and persistence decisions;
- ordinary authoritative product transitions synchronously notify the active qualification session;
- qualification-only capture failures fail closed for evidence without changing product success/failure;
- React exposes only qualification presentation and explicit operator actions;
- old lifecycle IPC and effect-driven binding/retry/finalization code are gone; and
- existing canonical target/evidence repository authority remains unchanged.

## 2. Required implementation order inside the single PR

This is one review unit, but implementation must respect these dependency stages.

### Stage 1 — Establish typed device observation

1. Extract passive device probing/retention/matching/support projection into a Rust `device_observation` module.
2. Replace internal `serde_json::Value` use at the observation seam with typed Rust structures.
3. Migrate existing ordinary commands and passive qualification consumers to the module.
4. Keep explicit root check implementation and ownership separate.
5. Preserve current public/sanitized TypeScript DTO shapes unless a compile-safe cleanup is required by the typed seam.

This stage must not change qualification lifecycle semantics by itself.

### Stage 2 — Establish qualification-session lifecycle module

1. Extract the live evidence session from the broad existing qualification implementation into `qualification_session`.
2. Give it one closed lifecycle observation interface plus separate explicit operator actions.
3. Enforce one active session.
4. Make the module own persistence/recovery orchestration through the existing qualification repository adapter.
5. Add a new persisted-session version.
6. Introduce a closed typed invalidation reason model.
7. Make invalidation monotonic.

Do not merge target registration, canonical evidence recording, matrix generation, or Node authority into this module.

### Stage 3 — Wire authoritative observations

At the trusted Tauri orchestration seam, after each product operation commits and before its result is returned/published, notify the active qualification session of the exact typed committed result.

Required observation points:

- authoritative selected/probed device observation;
- explicit root check completion;
- authoritative review creation;
- real-execution admission/start; and
- real-execution terminal retention/classification.

Do not observe terminal state from polling/export/UI reads.

If qualification handling fails after product commit:

- return the product result normally;
- poison/invalidate qualification evidence;
- project qualification failure only to the development qualification UI.

When qualification is inactive, each hook must be a no-persistence no-op.

### Stage 4 — Implement checkpoint and candidate state machine

Required semantics:

- required prerequisite checkpoints must be `pass` before real-execution admission;
- prerequisite `fail` or `unable_to_verify` invalidates immediately;
- real execution admitted before required prerequisites pass invalidates qualification but does not block product execution;
- simulated execution has no qualification lifecycle effect;
- terminal real execution with missing required non-prerequisite checkpoints becomes terminal-awaiting-evidence;
- terminal-awaiting-evidence may accept only remaining declared checkpoint observations, not another execution;
- once all required evidence exists, materialize the candidate automatically;
- terminal product success may yield valid/passed;
- terminal product failure or failed non-prerequisite product evidence may yield valid/failed;
- lifecycle/evidence integrity failure yields invalid/not-observed;
- operator abandonment yields invalid/not-observed;
- candidate creation closes the active session;
- candidates are immutable.

No operator candidate-finalization action remains.

### Stage 5 — Fail-closed recovery and migration

1. Treat pre-refactor active persisted sessions as incompatible with the new lifecycle contract.
2. Preserve already-materialized candidates and canonical recorded evidence.
3. Convert incompatible old active sessions into invalid/not-observed state rather than migrating them to valid sessions.
4. Resume a valid new-version active session only after proven clean application handoff and a matching captured build identity.
5. Keep a valid session from another build deferred and unchanged until its captured build is running again; then it may resume after a proven clean handoff. Never relabel execution evidence from one build as evidence from another. An already-invalid session may be recovered under another build only as a non-promotable `invalid/not_observed` audit candidate.
6. Crash/forced/ambiguous prior termination invalidates the active attempt and retains a non-promotable `invalid/not_observed` audit candidate.
7. After clean restart, automatically reassociate on the first trusted device observation; never reuse the prior process-local handle.
8. Compatible observation continues the session; conflicting observation invalidates it.
9. If invalid-candidate persistence fails, keep the session poisoned/non-recordable in memory and never claim the candidate exists.

Do not add a qualification event journal.

### Stage 6 — Delete frontend lifecycle orchestration

Remove lifecycle IPC and frontend calls for:

- `refresh_qualification_session`;
- `bind_qualification_review`;
- `bind_qualification_execution`; and
- `finalize_qualification_candidate`.

Simplify `useDeviceQualificationMode` so it no longer contains:

- workflow snapshot comparison keys;
- process-local associated-device refs used to infer trust;
- review/execution binding sets;
- lifecycle promise maps;
- retry-revision machinery;
- microtask lifecycle retries;
- execution finalization maps; or
- effects whose purpose is to reconstruct authoritative lifecycle ordering.

The controller should retain:

- status/session/candidate projection;
- ordinary busy/error presentation;
- explicit begin;
- target capture/register;
- checkpoint recording;
- abandon;
- candidate record/discard; and
- explicit refresh/read behavior where useful for presentation.

Intent/device-selection locks come from sanitized Rust session state, not frontend inference.

### Stage 7 — Operator UI and documentation

The development qualification surface must:

- keep target/workflow selection explicit;
- expose active session state;
- show sanitized invalidation/non-recordable reason;
- show terminal-awaiting-evidence state when applicable;
- expose `Abandon qualification attempt`;
- preserve explicit target registration and candidate record/discard actions; and
- never expose internal paths, serials, raw ADB output, repository paths, or arbitrary Rust errors.

Update `docs/manual/device-qualification-operator.md` for the automatic lifecycle semantics and abandonment behavior.

Do not change end-user compatibility language away from `supported` / `not supported`.

## 3. Authority invariants

The implementation is incorrect if any of these become false:

1. Rust/Tauri remains sole trusted product authority.
2. Qualification remains a development-only observer of the ordinary product workflow.
3. React does not decide or reconstruct trusted lifecycle ordering.
4. Root checking remains an explicit separate production authority.
5. `tools/device-qualification.mjs` remains the sole canonical repository evidence authority.
6. Qualification failure cannot fail an already-successful ordinary product transition.
7. Qualification evidence cannot remain valid after a missed/unpersisted authoritative transition.
8. A later observation cannot repair a monotonically invalidated session.
9. Process-local device handles are never trusted across restart.
10. Simulation remains outside physical qualification evidence.
11. Product review and explicit real-execution confirmation remain mandatory.
12. No physical qualification claim is created by this refactor.

## 4. Persistence contract

The session persistence representation must contain enough typed state to prove:

- persisted-session contract version;
- session identity;
- target/workflow identity and version;
- declared product intent;
- build/runtime binding;
- trusted current device association identity independent of a process-local handle;
- recorded checkpoints;
- review/execution bindings;
- terminal execution classification when present;
- lifecycle phase;
- monotonic invalidation state; and
- clean-handoff resumability state as needed by the established native lifecycle contract.

Do not persist raw secrets, arbitrary errors, raw ADB output, native paths, or process-local handles as durable authority.

The exact serialized shape is implementation-defined and may reuse existing repository structures where safe.

## 5. Lifecycle observation behavior matrix

| Scenario | Product outcome | Qualification outcome |
|---|---|---|
| Qualification inactive | unchanged | no-op, no persistence |
| Begin with valid target/workflow/device | normal | one active persisted session |
| Second begin while session active | normal product state unchanged | reject operator action |
| Compatible device observation | normal | associate/refresh session |
| Conflicting device observation | normal | invalid/not-observed |
| Clean restart + compatible selected device | normal | automatic reassociation |
| Unproven prior shutdown | normal | invalid/not-observed |
| Explicit root check completes | normal root result | capture exact typed result |
| Review creation commits | review succeeds | capture exact review binding |
| Qualification persistence fails after review commit | review still succeeds | poison/invalidate qualification |
| Simulation starts/completes | simulation behaves normally | no lifecycle change |
| Real execution admitted with prerequisites passed | execution starts | bind execution |
| Real execution admitted with missing/failed prerequisite | execution still starts | invalid/not-observed |
| Real execution becomes terminal without UI polling | terminal report retained | terminal captured immediately |
| Terminal success + all required evidence | success | automatic valid/passed candidate |
| Terminal product failure + all required evidence | failure | automatic valid/failed candidate |
| Terminal execution + missing required post checkpoint | terminal product state | terminal-awaiting-evidence |
| Required post checkpoint later recorded | product unchanged | candidate materializes when complete |
| Operator abandons active attempt | product unchanged | invalid/not-observed candidate |
| Invalid-candidate persistence fails | product unchanged | poisoned non-recordable session; no claimed candidate |

## 6. Test migration

### 6.1 Add/strengthen Rust tests

The primary lifecycle test suite must move to `qualification_session` and prove:

- one active session;
- typed begin validation;
- direct exact-result observation;
- deterministic order handling;
- duplicate committed observation idempotency where needed;
- out-of-order/missing transition invalidation;
- all checkpoint rules;
- terminal-awaiting-evidence;
- valid passed/failed classification;
- invalid/not-observed classification;
- abandonment;
- persistence failure isolation;
- invalid-candidate persistence failure poisoning;
- old-version rejection;
- clean-handoff recovery;
- crash/unproven-handoff invalidation;
- automatic process-local device reassociation; and
- inactive no-op behavior.

Device-observation tests must prove:

- typed fact decoding/projection;
- retained observation behavior;
- matching;
- support/capability projection;
- root separation; and
- current public DTO compatibility.

Tauri integration tests must prove ordinary product operations trigger session observation without React lifecycle commands, including terminal execution occurring independently of polling.

### 6.2 Replace frontend orchestration tests

Delete or rewrite tests that exist only to prove:

- binding promise maps;
- effect ordering;
- retry revisions;
- React device reassociation;
- React execution finalization; or
- lifecycle IPC call ordering.

Keep frontend tests for:

- dev-only overlay;
- explicit operator commands;
- session/candidate rendering;
- projected locks;
- terminal-awaiting-evidence;
- invalidation copy;
- abandonment;
- accessibility; and
- one bounded integration proof that ordinary product state reflects automatic trusted lifecycle updates.

### 6.3 No physical hardware requirement

All acceptance tests for this architecture refactor must be deterministic and runnable without a physical Android device. Do not add or modify canonical physical evidence.

## 7. Expected code organization

Exact file splitting is left to implementation, but the resulting architecture should make these ownership lines obvious:

- `device_observation.rs`: passive typed observation module;
- `qualification_session.rs`: evidence-session lifecycle module;
- `qualification_repository.rs`: storage/repository adapter and canonical-tool integration;
- existing ordinary command/execution modules: product authority plus small calls into the qualification-session observer at the trusted seam;
- `qualification_mode.rs` or successor: development-mode gating, target registration, operator-facing command adapters, and composition, not lifecycle choreography;
- React qualification hook: presentation/operator adapter only.

Do not create pass-through modules merely to move line count.

## 8. Scope constraints

### Required connected areas

The implementation may need coherent changes across:

- Tauri device commands/observation;
- passive device qualification projection;
- explicit root-check orchestration hook;
- review creation orchestration;
- real-execution admission and terminal retention;
- qualification mode/session/repository code;
- Tauri command registration;
- frontend qualification API/types/hook/overlay;
- qualification-focused frontend and Rust tests; and
- operator documentation.

### Explicitly outside scope

Do not:

- refactor unrelated `App.tsx` orchestration;
- change planner/executor semantics;
- change normal saved-configuration semantics;
- add generic app-wide event infrastructure;
- add physical qualification evidence;
- change canonical device-target/evidence schemas or deterministic IDs;
- redesign support tiers/matrix projection;
- add app-definition work from separate catalog tasks;
- change ordinary production root semantics; or
- enable real execution in ordinary builds.

## 9. Acceptance criteria

1. Internal device observation is typed end to end until explicit IPC/tooling serialization seams.
2. Ordinary product and qualification consumers use one passive device-observation authority.
3. Explicit root check remains separate and qualification only captures its committed typed result.
4. Qualification lifecycle is owned by a Rust module with one closed observation interface and separate explicit operator actions.
5. At most one active qualification session exists.
6. Tauri synchronously observes authoritative device/review/root/real-execution transitions after commit and before product result publication.
7. Qualification capture failure does not change product outcome and permanently prevents that attempt from producing valid evidence.
8. Terminal real execution is captured at the authoritative transition, not through polling/export/UI reads.
9. Prerequisite and post-execution checkpoint semantics match the approved design.
10. Candidate materialization is automatic and immutable; explicit lifecycle finalization is removed.
11. Abandonment produces invalid/not-observed audit state when persistence succeeds.
12. Old active persisted sessions fail closed; clean compatible new sessions resume; unproven shutdown invalidates.
13. Invalid-candidate persistence failure cannot later resurrect a valid session.
14. Lifecycle IPC commands and React lifecycle orchestration are removed.
15. Simulation behavior and ordinary product review/confirmation/execution semantics remain unchanged.
16. Existing canonical Node evidence authority, registered targets, evidence bundles, and generated matrix are unchanged.
17. Deterministic Rust/Tauri/frontend tests prove the new module interfaces and replace superseded implementation-shaped tests.
18. The operator manual reflects automatic lifecycle capture and abandonment.
19. No physical evidence is created or modified.
20. The final diff stays within this architecture refactor and does not absorb the broader `App.tsx` redesign.
