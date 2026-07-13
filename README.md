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
./crates/emuchef-rust-backend/target/debug/emuchef validate \
  authored/recipes/app.retroarch.provision.yaml \
  --authored-root authored
./crates/emuchef-rust-backend/target/debug/emuchef plan \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --output /tmp/emuchef-plan.yaml
./crates/emuchef-rust-backend/target/debug/emuchef apply \
  --plan-file /tmp/emuchef-plan.yaml \
  --dry-run
```

Mutating apply requires ADB and should be run only against a safe test device.

Runtime recipe inputs can be supplied directly or loaded from a persisted user
configuration. The canonical contract, CLI examples, configuration-root rules,
and side-effect-free discovery and planning operations are documented in
[runtime recipe configuration](docs/architecture/runtime-recipe-configuration.md).

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

Artifact resolution supports absolute `file://`, HTTP, and HTTPS URLs. Network
downloads use strict Rustls verification, bounded redirects and timeouts,
same-directory partial files, and no-clobber cache publication. Developer ID
signing, hardened runtime, app and DMG notarization, stapling, and local
Gatekeeper validation are complete for the recorded macOS release artifacts.
Clean-Mac validation, updater support, and cross-platform release automation
remain future work. The packaged frontend uses a local-only production CSP. The
completed local and HTTP(S) device run is
recorded in the
[2026-07-11 RetroArch evidence](docs/release/evidence/real-device-retroarch-2026-07-11.md).

See [runtime ownership](docs/architecture/runtime-ownership.md), the
[planner/executor architecture](docs/architecture/planner-executor.md),
[runtime recipe configuration](docs/architecture/runtime-recipe-configuration.md),
and [release readiness](docs/release/release-readiness.md). Real-device evidence is
collected with the [RetroArch validation runbook](docs/manual/real-device-retroarch-validation.md),
and packaged macOS evidence uses the
[Config Editor validation runbook](docs/manual/macos-packaged-gui-validation.md).
The completed Developer ID and notarization result is recorded in the
[macOS signing evidence](docs/release/evidence/macos-signing-notarization-2026-07-11.md),
and releases follow the
[signing and notarization runbook](docs/manual/macos-signing-notarization.md).
