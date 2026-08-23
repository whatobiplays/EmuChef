# Task 6 implementation report

## Scope

Task 6 exposes the production observations required by the device qualification
harness while keeping EmuChef's existing production workflow as the system under
test.

- The production ADB probe parses `ro.build.fingerprint` into the optional Rust
  fact `firmware_build` and exposes it as nullable `firmwareBuild` at the React
  boundary.
- The selected-device command and the future qualification orchestration share
  the same production probe helper. No qualification-only ADB probe was added.
- Terminal production report and runtime metadata are retained with the existing
  execution handle store. `production_execution_report_bytes` is the single
  report projection, redaction, serialization, and trailing-newline authority.
  The native export dialog delegates to that helper and retains its existing
  dialog/write workflow.
- `CONTEXT.md` documents the current firmware fact, shared probe boundary, and
  report-capture semantics.

No physical device qualification or physical-device evidence was run or
created. No SDD ledger, `.chatgpt` file, generated qualification artifact, or
external system was modified.

## TDD evidence

### RED: firmware build fact

Added the backend `device_probe` regression first, including a
`ro.build.fingerprint` getprop line and an assertion on
`facts.firmware_build`. The required focused command failed to compile because
`DetectedDeviceFacts` did not yet have the field:

```text
error[E0609]: no field `firmware_build` on type `device_probe::DetectedDeviceFacts`
```

### GREEN: firmware build fact

Added the optional Rust field, parser mapping, existing serialization path, the
Tauri public projection, the TypeScript contract, and the production probe
helper. The focused backend command then passed:

```text
cargo test: 35 passed, 839 filtered out
```

### RED: production report bytes

Added the terminal-fixture report equivalence test before the production helper.
The required execution test command failed to compile because both the report
retention method and `production_execution_report_bytes` were absent:

```text
no method named `mark_terminal_with_report`
cannot find function `production_execution_report_bytes`
```

### GREEN: production report bytes

Added terminal report/runtime retention, the production report-byte helper, and
the export delegation. The helper test verifies byte-for-byte equality with the
sanitized report document and successful JSON deserialization. The focused
execution command then passed:

```text
cargo test: 61 passed, 202 filtered out
```

## Validation

Required Task 6 validation passed after the final changes:

1. `cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml device_probe -- --nocapture` — 35 passed.
2. `cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml execution -- --nocapture` — 61 passed.
3. `cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml commands -- --nocapture` — 26 passed.
4. `npm --prefix apps/emuchef-app run typecheck` — passed (`ok`).

Additional checks passed:

1. Backend full suite serialized — 857 passed, 17 ignored.
2. Default Tauri suite — 261 passed, 2 ignored.
3. Real-execution `execution::tests` — 57 passed.
4. Frontend tests — 74 passed across 6 files.
5. Frontend lint — passed (`ok`).
6. Rust format checks for both manifests — passed.
7. `git diff --check` — passed.

The broader real-execution module filter also selects the existing
`phase6d6_ui_smoke` test. That test fails because its checked-in
`docs/testing/phase-6d6/ui-binding-index.json` has a stale source digest for the
necessarily modified `execution.rs`; it cannot load the candidate from that
index. The historical generated index was not regenerated because doing so is
outside Task 6 and would modify unrelated evidence. The Task 6 execution tests
and the real-execution execution-test subset pass.

## Changed files

1. `crates/emuchef-rust-backend/src/device_probe.rs`
2. `apps/emuchef-app/src-tauri/src/commands.rs`
3. `apps/emuchef-app/src-tauri/src/execution.rs`
4. `apps/emuchef-app/src/types.ts`
5. `apps/emuchef-app/tests/workflow.test.ts`
6. `CONTEXT.md`
7. `.superpowers/sdd/2026-08-23-device-qualification-harness/task-6-report.md`

The final local commit SHA is reported in the task handoff; this report is
included in that commit.
