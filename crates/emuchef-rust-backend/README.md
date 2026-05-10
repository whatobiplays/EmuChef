# EmuChef Rust Backend Skeleton

This package is an experimental Rust backend skeleton for the EmuChef config
editor protocol. It is standalone and runnable independently of the Tauri
editor.

Phase 6D implements only:

- `hello`
- `listStepSpecs`
- stable success and error envelopes
- structured API errors
- one-shot JSON request handling
- JSON Lines sidecar request handling

It reports only the capability that is implemented in this crate:

```json
{
  "protocolVersion": 1,
  "capabilities": ["listStepSpecs"]
}
```

Reporting `listStepSpecs` does not make this backend compatible with the Tauri
editor. The current Tauri compatibility gate still rejects it because document
capabilities such as `openRecipe`, `getDocument`, `applyRecipeCommand`, `undo`,
`redo`, `saveRecipe`, `validate`, `emitYaml`, and `getRefIndex` are missing.

The Python backend remains the reference implementation. This Rust package is
not a replacement backend and is not selected by the Tauri editor.

This package does not load recipes, parse YAML, validate authored data, create
document sessions, apply editor commands, save files, run planner behavior, run
executor behavior, bundle Python, or provide production packaging.

## StepSpec Source

`listStepSpecs` returns a static parity copy of the current Python built-in step
spec metadata. The fixture lives at:

```text
crates/emuchef-rust-backend/tests/fixtures/python_step_specs.json
```

That fixture is temporary Phase 6D scaffolding, not the final Rust step
registry. Later phases should replace it with Rust-native schema builders before
planner or executor behavior is ported. Regenerate and compare the fixture any
time Python step specs change.

Regenerate the fixture from the repo root with:

```bash
PYTHONPATH=src uv run --no-project --native-tls --with PyYAML python -m emuchef_editor.api.server '{"type":"listStepSpecs"}' \
  | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["result"], indent=2, sort_keys=True))' \
  > crates/emuchef-rust-backend/tests/fixtures/python_step_specs.json
```

The fixture stores only the Python response `result` object, not the outer
`{"ok": true, "result": ...}` envelope.

## One-Shot Mode

Run one request as a single JSON argument:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"hello"}'
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"listStepSpecs"}'
```

Expected `hello` stdout is one JSON response envelope:

```json
{"ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs"]}}
```

`listStepSpecs` returns `{"stepSpecs":[...]}` inside the success envelope.

## JSONL Sidecar Mode

Run the sidecar loop with `--sidecar`:

```bash
printf '%s\n' '{"id":"hello-1","type":"hello","payload":{}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
printf '%s\n' '{"id":"specs-1","type":"listStepSpecs","payload":{}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

Expected `hello` stdout is one JSON response line:

```json
{"id":"hello-1","ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs"]}}
```

Request-level errors are returned as API envelopes and do not terminate the
sidecar:

```bash
printf '%s\n%s\n' '{"id":"bad","type":"unknown"}' '{"id":"specs-2","type":"listStepSpecs"}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

Mixed sidecar and one-shot CLI usage is a process-level usage error:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar '{"type":"hello"}'
```

That command exits non-zero, writes the usage error to stderr, and emits no API
envelope on stdout.

## Validation

Run the crate tests with:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
```
