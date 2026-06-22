# Rust Readiness Fixture Maintenance

## Purpose

`tests/fixtures/readiness/current_static_readiness_report.json` captures the
current static readiness report shape for the no-evidence synthetic report used
by `tests.test_check_rust_planner_cutover_readiness`.

The fixture is a review guard for report-shape changes. It is not readiness
evidence, does not clear blockers, and does not change readiness semantics.
P8BG itself does not refresh or modify
`tests/fixtures/readiness/current_static_readiness_report.json`.

## When to refresh

Refresh the fixture only when the static readiness report contract or top-level
report shape intentionally changes. Do not refresh it to hide an unexpected test
failure, incidental ordering churn, host-specific data, local evidence leakage,
or readiness-status drift.

Any fixture refresh must be part of the same change that intentionally updates
the report contract. The docs for that report-shape change must explain the new
or changed fields before the fixture update is accepted.

## Manual refresh workflow

Run the readiness fixture test before updating the fixture:

```bash
PYTHONPATH=src rtk python3 -m unittest tests.test_check_rust_planner_cutover_readiness
```

If the fixture comparison fails, review the reported serialized readiness output
against the checked-in fixture before changing any fixture content. The failure
must be treated as a contract review point, not as automatic approval to
regenerate the fixture.

After the report-shape change and fixture update are reviewed together, rerun
the same command and confirm the fixture comparison passes.

## Review checklist

- Confirm the top-level shape change is intentional.
- Confirm no host-specific absolute paths were introduced.
- Confirm no `.local` references were introduced.
- Confirm no stdout/stderr/environment/device serial leakage was introduced.
- Confirm readiness semantics and blocker status changes are intentional.
- Confirm docs mention the report-shape change.

## Scope boundaries

Fixture maintenance must not change readiness tooling, smoke tooling,
CLI/runtime behavior, packaging, Rust backend code, executor/apply behavior, or
`.local` evidence.

The static readiness gate remains a deterministic source-text and supplied-report
check. Fixture maintenance does not run smoke tools, ADB, Cargo, npm, Tauri,
executor/apply, network checks, device probing, or planner runtime paths.
