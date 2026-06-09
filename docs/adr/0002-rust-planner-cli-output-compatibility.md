# ADR 0002: Rust Planner CLI Output Compatibility

## Status

Accepted

## Context

Python currently owns the default `emuchef plan` CLI contract. That contract
includes default concise planning summaries, structured YAML output for
`--verbose`, YAML file emission for `--output`, stdout/stderr behavior, and exit
codes.

The current Rust planner route is the explicit developer-only shadow path:
`emuchef plan --planner-backend rust-shadow --rust-planner-bin <path>`. It
requires an already-built `emuchef-plan-shadow` binary. The route forwards only
the supported shadow inputs to Rust and preserves Rust stdout, stderr, and exit
code as JSON passthrough. It rejects Python-only output and device-context
options such as `--output`, `--verbose`, `--ops`, ADB flags, and explicit device
fact flags.

The Rust shadow route is migration evidence only. It is not a default planner
route, a formatter layer, a Cargo fallback, an executor/apply route, a Tauri
protocol route, or a real-device behavior change.

## Decision

When Rust eventually becomes the default planner backend for `emuchef plan`, the
default CLI output and exit-code behavior should remain compatible with the
current Python-owned `emuchef plan` contract unless a separate accepted
breaking-change decision says otherwise.

The compatibility targets are:

1. Python concise summary output remains the default output compatibility target.
2. Python `--verbose` structured YAML behavior remains the compatibility target.
3. Python `--output` YAML file behavior remains the compatibility target.
4. Python stdout/stderr and exit-code behavior remain the compatibility target.

Rust-native JSON may be exposed later only through an explicit structured-output
mode, such as a future `--format json`. It must not silently replace the current
default Python CLI output contract.

This decision does not implement the Rust-to-Python CLI formatter or translation
layer. It does not make Rust the default planner backend. It does not change the
current `rust-shadow` JSON passthrough contract. It does not affect executor,
apply, Tauri UI, Tauri protocol, sidecar protocol, real-device probing, ADB,
network, artifact materialization, fixture/golden ownership, or normal runtime
checks.

## Consequences

Default Rust planner cutover remains blocked until the future route can preserve
the current Python CLI output and exit-code contract, or until a separate
accepted breaking-change decision replaces that target.

The current `rust-shadow` route remains explicit opt-in, developer-only, and
JSON passthrough. It may continue to be useful for planner migration inspection,
but it is not evidence that default CLI output compatibility is implemented.

