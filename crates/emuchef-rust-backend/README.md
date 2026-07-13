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
editor and Phase 0 product execution sessions for a future end-user app.
One-shot requests remain available for stateless protocol operations.

Sidecar filesystem and ADB policy can be fixed at process startup:

```bash
emuchef --sidecar \
  --runtime-root /private/emuchef-runtime \
  --cache-root /shared/emuchef-artifacts \
  --adb /path/to/adb
```

The additive `phase0_end_user_runtime` extension must be negotiated before use.
It provides resolved catalog description, canonical JSON SHA-256 plan digests,
target preflight, one active real or dry-run attempt, complete retained
snapshots, ordered sequence-numbered events, and cooperative cancellation.
Dry-run reports are explicitly simulated. Retry creates a new attempt from a
fresh reviewed plan; execution rollback and undo do not exist. The stable DTOs
and matching rules are documented in
[`docs/product/phase-0-runtime-contracts.md`](../../docs/product/phase-0-runtime-contracts.md).

## Artifact Boundary

Absolute `file://`, HTTP, and HTTPS URLs resolve during the existing
`resolve_artifacts` step. HTTPS uses strict Rustls validation. Redirects are
limited to five and cannot downgrade HTTPS to HTTP. Connections time out after
15 seconds, the complete transfer has a five-minute deadline, and failures are
not retried automatically.

Successful bodies are streamed without transparent decompression into unique
same-directory partial files. Files are flushed, synced, and published with
`persist_noclobber` semantics; exact no-clobber mechanics remain
platform-dependent. Complete default-cache files bypass network setup.
`cache: none` always transfers and selects a unique runtime path on collision.
There is no resume support or authored checksum field. Reqwest uses standard
system proxy discovery without EmuChef-specific proxy settings.

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
