# EmuChef Config Editor

This is the Tauri v2 React editor for authored EmuChef recipe files. The editor
runtime launches the Rust JSONL sidecar directly; it has no backend selector,
backend toggle, environment-variable backend choice, protocol negotiation path,
or Python fallback.

The current runtime ownership decision is recorded in
[ADR 0001](../../docs/adr/0001-rust-tauri-editor-runtime-ownership.md).

## Development

Install frontend dependencies once:

```bash
npm install
```

Start the Tauri dev shell:

```bash
npm run tauri dev
```

`tauri dev` runs `npm run sidecar:dev && npm run dev` through
`beforeDevCommand`. The sidecar step builds the Rust backend with Cargo's debug
profile and prepares the Tauri v2 `externalBin` input under
`src-tauri/binaries/emuchef-rust-backend-$TARGET_TRIPLE`. Cargo incremental
builds make running this on each dev start acceptable, and the preparation
script avoids copying when the generated binary is already current.

To prepare the debug sidecar without starting Tauri:

```bash
npm run sidecar:dev
```

## Runtime Verification

Phase 6X adds app-local release-hardening checks for the Rust-sidecar runtime.
The fast aggregate check is deterministic and does not invoke Python, ADB, a
release build, golden regeneration, or a real Tauri package build:

```bash
npm run check:rust-runtime
```

It runs the pure sidecar naming tests, bundle-inspection unit tests, the
no-Python-runtime scan, TypeScript typecheck, and frontend logic tests. The
subcommands are also available directly:

```bash
npm run test:sidecar-packaging
npm run test:sidecar-bundle-inspection
npm run check:no-python-runtime
npm run typecheck
npm run test:logic
```

`check:no-python-runtime` scans only active Tauri runtime/build files for
forbidden command/runtime tokens, including Windows `.exe` spellings, or explicit
module names. It does not require Python to be absent from the repository; Python
remains available only for legacy/reference/developer/golden tooling outside the
Tauri runtime path.

## Packaged Build

Build the frontend, Rust Tauri shell, and bundled Rust sidecar:

```bash
npm run tauri build
```

`tauri build` runs `npm run sidecar:build && npm run build` through
`beforeBuildCommand`. The sidecar step builds the Rust backend with Cargo's
release profile, verifies `rustc --print host-tuple`, and writes the
target-triple-suffixed `externalBin` source expected by Tauri v2. Phase 6V
verifies host-target packaging only; cross-compilation is not configured here.

For the configured `externalBin` value `binaries/emuchef-rust-backend`, Tauri v2
expects these source names before bundling:

- macOS/Linux: `src-tauri/binaries/emuchef-rust-backend-$TARGET_TRIPLE`
- Windows: `src-tauri/binaries/emuchef-rust-backend-$TARGET_TRIPLE.exe`

Tauri strips the target triple in the packaged app. The config editor resolves
the packaged sidecar by looking for `emuchef-rust-backend` or
`emuchef-rust-backend.exe` beside the app executable. On macOS this is
`EmuChef Config Editor.app/Contents/MacOS/emuchef-rust-backend`.

The generated `src-tauri/binaries` artifacts are ignored source-control outputs.
The preparation script writes metadata next to the generated binary and verifies
the copied artifact is at least as fresh as the Cargo-built sidecar.

To inspect prepared sidecar bundle inputs without building a full Tauri package:

```bash
npm run check:sidecar:bundle-input:debug
npm run check:sidecar:bundle-input
```

The debug variant runs `sidecar:dev` and is intended for routine local checks.
The release variant runs `sidecar:build`, so it may perform a release Rust build
before inspecting the host-target `externalBin` source artifact, metadata,
packaged launch name, and Unix executable bit.

The targeted simulated-packaged sidecar smoke is:

```bash
npm run smoke:sidecar:simulated-packaged
```

This smoke copies the real Rust backend to a temporary simulated bundled
directory and runs the editor JSONL request sequence through packaged-mode
resolution. It is not a real packaged app, installed bundle, signing,
notarization, updater, or GUI E2E test.

`tauri.conf.json` does not point at a custom app icon in Phase 6V. The previous
placeholder `icons/icon.png` path had no corresponding app-local file, so the
packaging config stays minimal until real branding assets are added.
