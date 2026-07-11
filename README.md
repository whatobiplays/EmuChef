# EmuChef

EmuChef is a Rust application for planning and applying reproducible Android
handheld configurations. The repository also contains a React/Tauri editor for
authored recipe YAML.

## Runtime

Rust is the sole product runtime. It owns the `emuchef` CLI, authored-data
validation, planning, execution, real-ADB apply, the editor document protocol,
and the JSONL sidecar used by Tauri. The Python source under `src/` is frozen
reference code pending deletion and has no executable entrypoint.

Build and test the CLI:

```bash
cargo build --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
./crates/emuchef-rust-backend/target/debug/emuchef --help
```

Typical CLI flow:

```bash
./crates/emuchef-rust-backend/target/debug/emuchef validate --authored-root authored
./crates/emuchef-rust-backend/target/debug/emuchef plan \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --output /tmp/emuchef-plan.yaml
./crates/emuchef-rust-backend/target/debug/emuchef apply \
  --plan-file /tmp/emuchef-plan.yaml \
  --dry-run
```

Mutating apply requires ADB and should be run only against a safe test device.

## Config Editor

```bash
cd apps/config-editor
npm install
npm run check:rust-runtime
npm run tauri dev
```

The editor launches the Rust `emuchef --sidecar` process. It does not use a
Python backend or backend selector.

## Current Limitations

HTTP(S) artifact downloading is not implemented and is the highest-priority
runtime feature. Local paths and `file://` artifact sources are supported.
Release signing, notarization, updater support, CSP hardening, cross-platform
release automation, and recorded real-device validation remain future work.

See [runtime ownership](docs/architecture/runtime-ownership.md), the
[planner/executor architecture](docs/architecture/planner-executor.md), and
[release readiness](docs/release/release-readiness.md).
