# ADR 0003: Rust Real-Device Context Ownership for Planner Cutover

## Status

Accepted

## Context

Python currently owns the default `emuchef plan` CLI behavior. That route
resolves ADB, probes the selected or single connected device, constructs
detected-device context for planning, and emits detected-device profile mismatch
warnings when the connected device does not match the selected device plan's
profile.

The explicit Rust planner routes currently support synthetic/profile-derived
context and explicitly supplied context flags. `emuchef-plan-shadow`,
`rust-shadow`, and `rust-experimental` accept supplied manufacturer, model,
Android version, and ordered device tags for planner migration evidence. The
dev-only matrix tooling includes explicit-context evidence for supplied values.

The Rust planner routes do not yet probe real devices, create detected-device
facts, or emit detected-device profile mismatch warnings. The readiness gate
therefore keeps default Rust planner cutover blocked on
`real_device_probing_not_cut_over` and
`detected_device_profile_mismatch_warning_not_cut_over`.

Executor/apply parity and Python planner deletion remain separate blockers.
Tauri UI, Tauri protocol, sidecar protocol, Cargo fallback behavior, normal
runtime checks, fixture/golden ownership, network behavior, artifact
materialization, and Python executor/apply behavior are outside this decision.

## Decision

For future default Rust planner cutover, Rust should own real-device context
probing and detected-device profile mismatch warning parity.

Python remains the current default and reference implementation until a later
implementation and cutover phase. This ADR does not deprecate, remove, or change
Python probing behavior.

P8M does not implement device probing. Future Rust probing should be introduced
first behind explicit non-default Rust planner routes, with fake/non-live tests
before optional live ADB smoke. Live ADB smoke should remain optional and
developer-only until a later decision deliberately promotes it.

## Alternatives Considered

1. Keep Python probing permanently and pass detected facts to Rust.

   Rejected. This would preserve current user behavior in the short term, but it
   keeps Python in the default planning route and weakens the path toward Python
   planner deletion. It also leaves planner context ownership split across two
   runtimes after Rust becomes the default planner.

2. Let Rust own probing and planner context construction.

   Accepted for future default Rust planner cutover. This has the largest
   implementation cost because Rust needs a probing abstraction, fake probe
   tests, detected-context construction, and mismatch-warning parity. It best
   supports eventual Python retirement because the default route can become
   Rust-owned instead of depending on Python for device facts.

3. Skip probing and require explicit context for Rust cutover.

   Rejected. Explicit context remains useful for deterministic migration tests
   and developer workflows, but an explicit-context-only cutover would drop
   current default CLI behavior for users who rely on connected-device detection.

## Consequences

Rust must get a device-probing abstraction before default Rust planner cutover.
That abstraction should support fake/non-live tests before any optional live ADB
smoke is promoted.

Rust must construct detected-context planner input and cover detected-device
profile mismatch warning parity before default cutover. Explicit context support
remains useful, but it is separate from real-device probing and does not resolve
the probing or warning blockers.

Python default behavior remains unchanged for now. Python remains the current
default CLI/reference planner owner until a later implementation and cutover
phase.

The readiness gate remains blocked until implementation and evidence exist for
real-device probing and detected-device profile mismatch warning parity.

This decision does not change executor/apply behavior, Tauri UI, Tauri protocol,
sidecar protocol, Cargo fallback behavior, fixture/golden ownership, normal
runtime checks, network behavior, artifact materialization, or Python planner
deletion readiness.

## Future Work

1. Add a Rust device-probing abstraction.
2. Cover the abstraction with fake/non-live probe tests.
3. Build Rust detected-context planner input construction.
4. Add detected-device profile mismatch warning parity.
5. Add detected-context support to explicit non-default Rust planner routes.
6. Add optional live ADB smoke for developer-only validation.
7. Reclassify readiness gate blockers after implementation evidence exists.
8. Cut over the default planner backend in a later phase.
