# Rust Production-Equivalent Route Implementation Plan

## Purpose

This document is an implementation plan for a future production-equivalent Rust
planner route. It does not implement route behavior, change runtime behavior, or
clear Rust planner cutover blockers.

The plan identifies the smallest safe path toward route evidence that can
eventually satisfy:

- the production-equivalent live probe smoke evidence bar in
  `docs/rust-production-equivalent-live-probe-smoke.md`;
- the default-route mismatch-warning parity evidence bar in
  `docs/rust-default-route-mismatch-warning-parity.md`.

P8AG is documentation-only. The future backend name
`rust-production-equivalent` is documentation-only here and is not accepted by
the CLI in P8AG.

## Current Route Inventory

The current route surface is split by migration purpose:

- Default Python backend: `emuchef plan` remains Python-owned and is the current
  production/reference planner route. Python resolves ADB/device facts before
  planning and owns the visible CLI output contract today.
- Python `rust-shadow`: an explicit developer-only bridge to a supplied
  `emuchef-plan-shadow` binary. It passes through Rust JSON by default, can use
  the explicit Python-compatible formatter, and rejects detected-facts and live
  probe wrapper flags that are not scoped to the migration route.
- Direct Rust shadow binary: `emuchef-plan-shadow` is a dev-only manual
  migration harness. It can run fixture-backed detected facts or explicit live
  `--probe-adb-getprop`, emits Rust `PlanningResult` JSON, and is not a
  production/default planner route.
- Python `rust-experimental`: an explicit non-default migration route. Python
  forwards local fixture paths or selected live-probe intent to the supplied
  Rust shadow binary without parsing `adb shell getprop`, and formats successful
  Rust `PlanningResult` JSON through the Python-compatible output path.
- Tauri/protocol: unchanged. Planner route cutover is not exposed through the
  active editor protocol by current evidence.
- Executor/apply: unchanged. Planner route cutover does not imply Rust
  executor/apply ownership.

## Candidate Route Options

### Extend `rust-experimental` Until It Is Production-Equivalent

This option would keep adding route behavior to the existing explicit migration
backend.

- P8AE fit: possible, because the route already forwards live-probe intent to
  Rust and uses Python-compatible output.
- P8AF fit: possible, if future wiring proves matched and mismatched scenarios
  through the route result.
- Default `emuchef plan` risk: low direct risk because the route remains
  explicit and non-default.
- Test complexity: moderate. Tests would need to distinguish migration evidence
  from production-equivalent evidence inside one backend name.
- User-facing semantics: weak. `rust-experimental` is explicitly unstable and
  migration-oriented, so production-equivalent evidence under that name can be
  confused with general experimentation.
- Migration/deletion implications: poor. Reusing the name makes it harder to
  tell which behavior can be promoted, deprecated, or deleted.

### Add a New Explicit `rust-production-equivalent` Route

This option adds a future explicit, non-default planner backend whose purpose is
to satisfy production-equivalent route evidence before default cutover.

- P8AE fit: strong. The route can be required to use Rust-owned probe execution,
  Rust-owned detected-fact interpretation, and ADR 0002-compatible output.
- P8AF fit: strong. The route can be required to emit matched and mismatched
  detected-device profile results through the planner result contract.
- Default `emuchef plan` risk: low. Python remains the default route until a
  separate cutover phase.
- Test complexity: moderate and cleaner than extending `rust-experimental`.
  Future tests can target a backend name whose purpose matches the evidence bar.
- User-facing semantics: clearer. The name signals production-equivalent
  behavior without implying default cutover.
- Migration/deletion implications: strong. `rust-experimental` can remain
  migration-only while the new route carries production-equivalent evidence.

### Promote Direct Rust Shadow Behavior Behind a Python Wrapper

This option would treat the existing direct Rust shadow behavior as the basis
for a production-equivalent wrapper.

- P8AE fit: possible for live probing, because direct shadow already has a
  Rust-owned live getprop path.
- P8AF fit: possible for mismatch warnings, because direct shadow can compose
  detected facts into a `PlanningResult`.
- Default `emuchef plan` risk: moderate. The route would need careful output and
  exit-code normalization because direct shadow is historically a raw Rust JSON
  harness.
- Test complexity: high. Tests would need to prove the wrapper is no longer
  relying on dev-shadow semantics even though it is built from them.
- User-facing semantics: weak. A production-equivalent route should not inherit
  the meaning of a dev-only shadow binary route.
- Migration/deletion implications: poor. This can make it harder to retire
  shadow-only behavior after cutover.

## Recommended Path

Add a new explicit non-default `rust-production-equivalent` planner backend
route before default cutover.

This is the smallest safe next implementation path because it:

- avoids changing default Python behavior;
- avoids overloading `rust-experimental`;
- creates a route whose purpose is specifically to satisfy
  production-equivalent evidence bars;
- allows route-specific smoke and evidence without implying default cutover;
- keeps blocker reclassification separate from route implementation.

The route should initially remain explicit and non-default. It should preserve
the ADR 0002 output and exit-code contract, follow the P8AD request/response
boundary, and keep ADR 0004 context precedence:

1. authored/profile-derived context is the fallback base;
2. detected facts establish live-device context;
3. explicit CLI context overrides detected facts.

## Non-Goals

This plan does not:

- change default `emuchef plan`;
- remove the Python planner;
- reclassify readiness blockers;
- modify readiness gate behavior;
- add Tauri/protocol integration;
- add executor/apply integration;
- implement route changes in P8AG.

## Required Test Evidence

Future implementation must add focused tests proving:

- CLI rejects production-equivalent flags outside the new backend;
- the route forwards or invokes Rust-owned probing without Python parsing
  `adb shell getprop`;
- detected facts affect `execution_plan.device_context`;
- explicit overrides win over detected facts;
- matched scenarios avoid `device_profile_mismatch`;
- mismatch scenarios emit a warning through `PlanningResult`;
- output and exit behavior preserve ADR 0002.

## Required Manual Evidence

Future blocker reclassification requires manual evidence that is not added to
the readiness gate in P8AG:

- production-equivalent live probe smoke satisfying P8AE;
- mismatch-warning parity evidence satisfying P8AF;
- readiness gate update only after the production-equivalent evidence exists.

## Risks

- Backend naming churn if `rust-production-equivalent` is accepted too early or
  renamed after evidence has accumulated.
- Confusing a production-equivalent explicit route with default cutover.
- Accidental blocker reclassification before production-equivalent evidence
  exists.
- Python wrapper leakage into probe interpretation.
- Duplicated semantics with `rust-experimental` if the future route does not
  define a sharper purpose.

## Follow-Up Phases

- P8AH add explicit production-equivalent backend flag validation only.
- P8AI wire production-equivalent backend to Rust-owned probe route.
- P8AJ add production-equivalent route smoke.
- P8AK add mismatch-warning parity evidence for production-equivalent route.
- P8AL update readiness gate only after evidence exists.
