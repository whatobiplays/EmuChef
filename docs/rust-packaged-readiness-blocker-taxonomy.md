# Rust Packaged Readiness Blocker Taxonomy

## Purpose

P8BD is documentation-only. It clarifies the readiness taxonomy around packaged
Rust planner evidence without changing readiness tooling, smoke tooling,
CLI/runtime behavior, Tauri/config-editor behavior, packaging scripts, Rust
backend code, tests, or `.local` evidence.

This document separates launcher-injection evidence from broader packaged
release readiness, executor/apply readiness, Python planner deletion readiness,
and top-level readiness.

## Evidence Status Semantics

`evidence_accepted` means the supplied evidence item passed validation. It does
not mean the broader release blocker is resolved unless the blocker is
specifically scoped to that evidence item.

P8BC accepts only explicitly supplied `rust_launcher_injected_planner_smoke`
reports with `schema_version: 1`. Accepted P8BC evidence can satisfy only the
packaged launcher-injection evidence item.

## Packaged Launcher-Injection Evidence

Packaged launcher-injection evidence proves that the CLI can be invoked with a
launcher-supplied `--rust-planner-bin` path and that the supplied entrypoint is
used.

The readiness blocker scoped to this evidence item is
`packaged_launcher_injection_evidence_not_accepted`. Accepted P8BC evidence may
move this specific evidence item to `evidence_accepted`.

This evidence does not prove packaged lookup, bundled executable location,
installer behavior, runtime distribution behavior, platform packaging, or real
packaged-app smoke.

## Packaged Release Readiness

Packaged release readiness is the broader release state covering packaging,
distribution, installer/runtime behavior, executable location, platform
packaging, and real packaged smoke.

The broader packaged release blocker is `packaged_release_not_ready`. Accepted
P8BC evidence does not clear broader packaged release readiness. Packaged
release readiness remains future work.

## Executor/Apply Readiness

Executor/apply readiness covers executor ownership, apply ownership, and
real-device apply readiness. Planner readiness does not imply executor/apply
ownership or real-device apply readiness.

The executor/apply blocker is `executor_apply_not_cut_over`. Accepted P8BC
evidence does not clear executor/apply readiness.

## Python Planner Deletion Readiness

Python planner deletion readiness covers the criteria required before the
Python planner reference/fallback implementation can be removed.

The Python planner deletion blocker is `python_planner_deletion_not_ready`.
Explicit Python fallback/reference planning remains available until separate
deletion-readiness criteria are met. Accepted P8BC evidence does not clear
Python planner deletion readiness.

## Top-Level Readiness

Top-level readiness is the aggregate readiness status emitted by the cutover
readiness gate. A single accepted evidence item does not make top-level
readiness ready when other blockers remain blocked.

Accepted P8BC evidence does not clear top-level readiness. Top-level readiness
remains `blocked` while broader packaged release readiness, executor/apply
readiness, Python planner deletion readiness, or other cutover blockers remain
blocked.

## Current P8BC/P8BD Facts

1. P8BD is documentation-only.
2. P8BC accepts only explicitly supplied
   `rust_launcher_injected_planner_smoke` reports with `schema_version: 1`.
3. Accepted P8BC evidence can satisfy only the packaged launcher-injection
   evidence item.
4. Accepted P8BC evidence does not clear broader packaged release readiness.
5. Accepted P8BC evidence does not clear executor/apply readiness.
6. Accepted P8BC evidence does not clear Python planner deletion readiness.
7. Accepted P8BC evidence does not clear top-level readiness.
8. Packaged release readiness remains future work.
