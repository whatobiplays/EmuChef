# EmuChef Rust Backend Skeleton

This package is an experimental Rust backend skeleton for the EmuChef config
editor protocol. It is standalone and runnable independently of the Tauri
editor.

Phase 6C implements only:

- `hello`
- stable success and error envelopes
- structured API errors
- one-shot JSON request handling
- JSON Lines sidecar request handling

It intentionally reports no editor capabilities yet:

```json
{
  "protocolVersion": 1,
  "capabilities": []
}
```

The Python backend remains the reference implementation. This Rust package is
not a replacement backend, is not selected by the Tauri editor, and is expected
to fail the current Tauri compatibility gate because it reports no document
editing capabilities.

This package does not load recipes, parse YAML, validate authored data, create
document sessions, apply editor commands, save files, run planner behavior, run
executor behavior, bundle Python, or provide production packaging.

## One-Shot Mode

Run one request as a single JSON argument:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"hello"}'
```

Expected stdout is one JSON response envelope:

```json
{"ok":true,"result":{"protocolVersion":1,"capabilities":[]}}
```

## JSONL Sidecar Mode

Run the sidecar loop with `--sidecar`:

```bash
printf '%s\n' '{"id":"hello-1","type":"hello","payload":{}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

Expected stdout is one JSON response line:

```json
{"id":"hello-1","ok":true,"result":{"protocolVersion":1,"capabilities":[]}}
```

Request-level errors are returned as API envelopes and do not terminate the
sidecar:

```bash
printf '%s\n%s\n' '{"id":"bad","type":"unknown"}' '{"id":"hello-2","type":"hello"}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
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
