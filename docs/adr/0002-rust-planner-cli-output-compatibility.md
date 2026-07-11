# ADR 0002: Rust Planner CLI Output Contract

## Status

Accepted

## Decision

`emuchef plan` emits a concise human-readable summary by default, structured
YAML with `--verbose`, and structured YAML to `--output`. Planning validation
results, stdout/stderr placement, and exit codes are stable CLI contracts.
Generated plan ids use `plan.<device-plan>.001`.
