# ADR 0001: Rust Tauri Editor Runtime Ownership

## Status

Accepted

## Context

The active Tauri config-editor runtime is the backend path used by
`apps/config-editor` for editor document sessions and editor commands. That path
is the runtime used by the Tauri app to open recipe documents, keep sidecar-owned
sessions, apply editor commands, validate, emit YAML, and save documents.

The active Tauri config-editor runtime is Rust-sidecar-only. It launches the
Rust JSONL sidecar through Tauri/Rust code and does not provide a Python
fallback, backend selector, backend toggle, environment-variable backend choice,
or protocol negotiation path.

Older documentation that describes Tauri calling a Python editor API is
historical migration context unless the current code still implements that path.
Python remains reference/developer/golden tooling and still owns unproven or
retained non-editor-runtime surfaces unless those surfaces are separately
replaced, intentionally retained, or retired.

For CLI, planner, executor, and real-device apply ownership/parity strategy, see
`docs/rust-cli-executor-parity.md`.

## Decision

The Tauri config editor must treat the Rust sidecar as the only active editor
backend runtime. The Tauri app must not add Python fallback behavior, backend
selection, or Python runtime calls for editor document sessions or editor
commands.

Python-owned surfaces must not be deleted without a separate parity or
retirement decision. Python remains available for reference, developer, golden,
legacy editor, and retained CLI/planner/executor workflows until each surface is
covered by verified Rust parity, explicitly kept in Python, or retired.

## Ownership

| Surface | Current owner | Notes |
|---|---|---|
| Tauri editor runtime | Rust sidecar | Active config-editor backend runtime; no Python fallback |
| Tauri UI | TypeScript/React + Tauri | Frontend invokes Rust/Tauri commands |
| Editor document session | Rust sidecar | Sessions are sidecar-owned and invalidated by sidecar restart |
| CLI plan/apply/detect | Python unless proven otherwise | Do not claim Rust ownership without verified parity or retirement |
| Planner/executor | Python unless proven otherwise | Rust parity may be partial/editor-specific unless separately proven |
| StepSpec fixture generation | Python/golden tooling unless proven otherwise | Ownership remains until the generation flow is replaced or retired |
| PySide6 editor | Legacy Python surface; cleanup pending | Not the active Tauri runtime |

## Python Deletion Blockers

Python-owned surfaces remain deletion blockers until each item is replaced with
verified Rust parity, intentionally retained as Python, or retired:

1. CLI parity or CLI retirement.
2. Planner/executor parity, or an explicit keep-Python decision.
3. Real-device apply strategy.
4. Template/create-from-template ownership decision.
5. StepSpec/golden fixture ownership.
6. PySide6 cleanup/removal decision.
7. Packaged Tauri release evidence.

## Consequences

The current editor runtime ownership is intentionally narrow: Rust owns the
active Tauri config-editor backend runtime, while Python continues to own or
support non-editor-runtime surfaces unless proven otherwise. This keeps the
Tauri runtime free of Python fallback paths without implying that Rust fully
replaces the Python CLI, planner, executor, template, golden, or legacy editor
surfaces.

Historical migration plans remain useful as evidence and context, but this ADR
is the current ownership decision for the active Tauri editor runtime.
