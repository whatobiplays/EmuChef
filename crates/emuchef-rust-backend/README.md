# EmuChef Rust CLI and Runtime

This crate is EmuChef's product runtime. It builds one executable, `emuchef`,
and provides CLI, planning, validation, execution, real-ADB apply, and editor
sidecar behavior.

## Commands

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --help
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- validate --authored-root authored
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- plan \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --output /tmp/emuchef-plan.yaml
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- apply \
  --plan-file /tmp/emuchef-plan.yaml \
  --dry-run
```

`--adb` selects an exact ADB executable and `--serial` selects a device. Do not
run non-dry-run apply against a device that is not safe to reset or modify.

## Sidecar

`emuchef --sidecar` reads JSONL requests on stdin and writes JSONL responses on
stdout. The sidecar maintains in-memory document sessions used by the Tauri
editor. One-shot requests remain available for stateless protocol operations.

## Artifact Boundary

Local paths and `file://` URLs are supported. HTTP(S) artifact downloading is
not implemented; the executor returns
`network_artifact_download_unsupported`. Network downloading is the next
runtime feature.

## Verification

```bash
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all -- --check
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets -- -D warnings
```

Frozen v1 expected results live under
`tests/fixtures/compatibility_goldens_v1`. See that directory's README before
changing them.
