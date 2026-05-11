# EmuChef Config Editor

This is the Tauri v2 React editor for authored EmuChef recipe files. The editor
runtime launches the Rust JSONL sidecar directly; it has no backend selector,
backend toggle, environment-variable backend choice, protocol negotiation path,
or Python fallback.

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

`tauri.conf.json` does not point at a custom app icon in Phase 6V. The previous
placeholder `icons/icon.png` path had no corresponding app-local file, so the
packaging config stays minimal until real branding assets are added.
