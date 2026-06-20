# Rust Production-Equivalent Route Implementation Plan

## Purpose

This document records the implementation plan and current state for the
explicit production-equivalent Rust planner route. P8AI wires the route through
the existing Python-to-Rust subprocess plumbing. P8AJ adds optional/manual smoke
tooling for the live-probe evidence bar. P8AK adds optional/manual fixture-backed
smoke tooling for mismatch-warning parity evidence. P8AL adds supplied-report
evidence intake to the readiness gate for manually saved P8AJ/P8AK reports.
P8AN verifies those reports as accepted evidence before default-route
implementation. P8AO then reuses the production-equivalent subprocess path as
the default no-backend `emuchef plan` route.

The plan identifies the smallest safe path toward route evidence that can
eventually satisfy:

- the production-equivalent live probe smoke evidence bar in
  `docs/rust-production-equivalent-live-probe-smoke.md`;
- the default-route mismatch-warning parity evidence bar in
  `docs/rust-default-route-mismatch-warning-parity.md`.

P8AG is documentation-only. P8AH recognizes the backend name
`rust-production-equivalent` in the Python CLI parser and reserves it with a
validation error. P8AI makes that backend executable only when explicitly
selected with `--rust-planner-bin`, reuses the supplied Rust shadow binary,
always uses Python-compatible output, and allows the same Rust-owned
detected-facts fixture and live-probe wrapper inputs as `rust-experimental`.
P8AI is route plumbing only. P8AJ adds
`tools/smoke_rust_production_equivalent_live_adb_probe.py` as optional/manual
smoke tooling for the live-probe wrapper route. The tool can produce evidence
when run manually with real device inputs, but its existence alone does not
clear `real_device_probing_not_cut_over`.
P8AK adds `tools/smoke_rust_production_equivalent_mismatch_warning.py` as
optional/manual fixture-backed smoke tooling for detected-device profile
mismatch-warning parity through the same explicit production-equivalent route.
That tool can produce evidence for
`detected_device_profile_mismatch_warning_not_cut_over` when run manually, but
tool existence alone does not clear the blocker.
P8AL updates the readiness gate to read only explicitly supplied P8AJ and P8AK
JSON reports. Accepted reports move the relevant blocker entry to
`evidence_accepted`, but the top-level readiness status remains `blocked` and
executor/apply plus Python planner deletion remain blocked.

`docs/rust-default-planner-cutover-contract.md` records the default-backend
cutover contract. P8AM performs no cutover, P8AN is evidence preflight only, and
P8AO is the default-route implementation phase after required evidence is
accepted. P8AP updates readiness-gate current-state classification only by
preserving the historical `default_cli_backend_still_python` entry with status
`resolved`; it does not change runtime route behavior, smoke tools,
executor/apply, Tauri/protocol, or Python planner deletion behavior.

## Current Route Inventory

The current route surface is split by migration purpose:

- Default planner backend: no-backend `emuchef plan` is Rust-owned through the
  production-equivalent subprocess path. It requires a supplied
  `--rust-planner-bin` during this transition and emits Python-compatible output.
- Explicit Python backend: `--planner-backend python` remains the transition
  fallback for the previous Python planning path.
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
- Python `rust-production-equivalent`: an explicit non-default route to the
  supplied Rust shadow binary. It requires `--rust-planner-bin`, always uses
  Python-compatible output, accepts `--rust-detected-facts-json <path>` and the
  complete live-probe wrapper flag set, and does not use Python ADB/device
  probing, planner session construction, or apply work.
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
- Default `emuchef plan` risk: P8AO now reuses this route for no-backend
  planning; before P8AO, the route was explicit-only.
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

- remove the Python planner;
- reclassify readiness blockers;
- modify readiness gate behavior;
- add Tauri/protocol integration;
- add executor/apply integration;
- add readiness-gate evidence in P8AI, P8AJ, or P8AK.

## Required Test Evidence

P8AI adds focused tests proving executable-route plumbing:

- the route forwards or invokes Rust-owned probing without Python parsing
  `adb shell getprop`;
- `--rust-planner-bin` is required;
- `--rust-shadow-output` stays scoped to `rust-shadow`;
- fixture and live-probe wrapper inputs are forwarded to the supplied Rust
  shadow binary;
- fixture and live-probe inputs stay mutually exclusive;
- output and exit behavior preserve ADR 0002.

## Required Manual Evidence

Future blocker reclassification requires manual evidence that is not added to
the readiness gate in P8AI, P8AJ, or P8AK. P8AL consumes only explicitly
supplied reports:

- production-equivalent live probe smoke satisfying P8AE;
- mismatch-warning parity evidence satisfying P8AF;
- readiness gate intake only after the production-equivalent evidence exists.

## Risks

- Confusing executable `rust-production-equivalent` route plumbing with
  production-equivalent smoke or readiness evidence.
- Confusing a production-equivalent explicit route with default cutover.
- Accidental blocker reclassification before production-equivalent evidence
  exists.
- Python wrapper leakage into probe interpretation.
- Duplicated semantics with `rust-experimental` if the future route does not
  define a sharper purpose.

## Follow-Up Phases

- P8AH recognizes and reserves the explicit production-equivalent backend name
  with validation only.
- P8AI wires the production-equivalent backend to Rust-owned probe route
  plumbing.
- P8AJ adds optional/manual production-equivalent live-probe smoke tooling.
- P8AK adds optional/manual production-equivalent mismatch-warning parity smoke
  tooling.
- P8AL updates the readiness gate to classify explicitly supplied P8AJ/P8AK
  evidence reports without clearing executor/apply or Python planner deletion
  blockers.
- P8AN verifies accepted P8AJ/P8AK report evidence without editing repo files.
- P8AO makes no-backend `emuchef plan` route through the existing
  production-equivalent subprocess path.
