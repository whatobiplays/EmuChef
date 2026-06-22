# Rust Planner Readiness Docs Index

P8BH is documentation-only. This index provides navigation for Rust planner
readiness and cutover evidence docs.

This index does not change readiness semantics, evidence acceptance rules,
smoke tooling, CLI/runtime behavior, packaging, Rust backend behavior,
executor/apply behavior, or `.local` evidence.

## Readiness Docs

- [rust-planner-cutover-readiness.md](rust-planner-cutover-readiness.md): Current cutover evidence and remaining blockers.
- [rust-launcher-injected-planner-smoke-contract.md](rust-launcher-injected-planner-smoke-contract.md): Launcher-supplied `--rust-planner-bin <path>` smoke goal and assertions.
- [rust-launcher-injected-planner-smoke-report-schema.md](rust-launcher-injected-planner-smoke-report-schema.md): `rust_launcher_injected_planner_smoke` schema and redaction shape.
- [rust-launcher-injected-planner-readiness-intake-design.md](rust-launcher-injected-planner-readiness-intake-design.md): Acceptance/rejection rules for explicitly supplied smoke reports.
- [rust-packaged-readiness-blocker-taxonomy.md](rust-packaged-readiness-blocker-taxonomy.md): Separation between launcher evidence, packaged release readiness, executor/apply readiness, Python planner deletion readiness, and top-level readiness.
- [rust-readiness-fixture-maintenance.md](rust-readiness-fixture-maintenance.md): Static readiness fixture refresh rules.
- [rust-packaged-planner-binary-location.md](rust-packaged-planner-binary-location.md): Current explicit-path binary behavior and future packaged/runtime location boundaries.
- [rust-packaged-planner-resolver-implementation-design.md](rust-packaged-planner-resolver-implementation-design.md): Future package/runtime-provided absolute path resolver design and current inert resolver state.
