# EmuChef Config Editor

The config editor is a React/Tauri application backed exclusively by the Rust
`emuchef --sidecar` runtime.

## Development

```bash
npm install
npm run check:rust-runtime
npm run tauri dev
```

`tauri dev` runs `npm run sidecar:dev` before Vite. The sidecar preparation
script builds the Rust binary and writes the target-triple-suffixed Tauri
`externalBin` input under `src-tauri/binaries/`.

## Checks

```bash
npm run typecheck
npm run test:logic
npm run build
npm run check:rust-runtime
cargo test --manifest-path src-tauri/Cargo.toml
```

`check:rust-runtime` validates sidecar packaging metadata, rejects Python
product/runtime code and alternate-backend surfaces through the repository-wide
retirement guard, typechecks the frontend, and runs frontend logic tests.

```bash
npm run check:python-runtime-retirement
```

## Packaging

```bash
npm run check:sidecar:bundle-input
npm run smoke:sidecar:simulated-packaged
npm run tauri build
```

The simulated smoke proves packaged path resolution and sidecar protocol
behavior but is not a real packaged GUI E2E result. Record real package evidence
with [the packaged GUI checklist](../../docs/manual/packaged-gui-e2e.md).
