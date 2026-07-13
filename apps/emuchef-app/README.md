# EmuChef End-User App

## 1. Purpose

`apps/emuchef-app` is the read-only React/Tauri workflow for end users. It
guides a user through connecting one Android device, confirming detected facts,
choosing a backend-approved device plan, selecting recipes, resolving recipe
inputs, and reviewing a target-bound plan. Phase 1 stops at review and has no
execution, dry-run, cancellation, apply, device-write, catalog-networking, or
artifact-resolution path.

The application is independent from `apps/config-editor`. Both applications
package the same Rust `emuchef --sidecar` runtime, but they do not import each
other's frontend modules or share runtime state.

## 2. Runtime and privacy boundary

Tauri starts and negotiates the Rust sidecar during application startup,
regardless of ADB availability. A missing ADB installation blocks only the
device workflow; the runtime and bundled catalog can still initialize and
report their status.

Sidecar and internal Tauri DTOs may contain exact device serials and resolved
filesystem roots. React receives separate DTOs containing opaque session
device handles, opaque review handles, masked serial presentation, catalog
identity, plan digest, human-readable review data, and actionable errors. Exact
serials, ADB executable paths, full reviewed plans, and catalog roots never
cross the React IPC boundary or enter frontend state, logs, storage, or markup.

The trusted Tauri backend retains the complete immutable reviewed-plan response,
exact target binding, catalog identity and digest, and canonical plan digest.
Reviews are held in memory only. At most 16 live reviews are retained. A review
expires after 30 minutes without access or two hours after creation. Device
disappearance, changed probed facts, catalog-digest change, Platform-Tools
replacement/removal, explicit discard, or bounded eviction invalidates the
review. Stale, expired, and unknown handles use `review_stale`,
`review_expired`, and `review_unknown` respectively.

Opaque device handles remain stable while the same exact serial is present in
polling results during one application session. The handle is invalidated when
the device disappears. A later reappearance receives a new handle.

## 3. Platform-Tools setup

The app does not bundle, vendor, redistribute, mirror, proxy, download, update,
or automate license acceptance for Android SDK Platform-Tools. When ADB is not
available, the device area presents a blocking setup screen with two actions:

1. Open Google's official [SDK Platform-Tools release page](https://developer.android.com/tools/releases/platform-tools) in the default browser.
2. Open the native macOS picker and import a user-selected Platform-Tools ZIP.

React cannot provide a ZIP path to the import command. Tauri owns the picker,
validation, temporary extraction, activation, replacement preservation, and
cleanup. See [Platform-Tools import and trust policy](../../docs/product/platform-tools-import.md).

## 4. Development

Prerequisites are Node.js, npm, Rust, and the normal Tauri macOS build tools.
ADB is not required to start or inspect runtime/catalog status.

```bash
npm --prefix apps/emuchef-app install
npm --prefix apps/emuchef-app run tauri:dev
```

The development command builds a debug `emuchef` sidecar, copies it to the
Tauri external-binary location, starts Vite on port 5174, and launches Tauri
with the development-only config. Production builds use the local-only CSP and
package the release Rust sidecar plus the catalog snapshot:

```bash
npm --prefix apps/emuchef-app run tauri:build
```

Debug builds may opt into an explicit ADB override with `EMUCHEF_ADB_PATH`.
They may deliberately enable system-PATH lookup with
`EMUCHEF_ALLOW_SYSTEM_ADB=1`. Release builds compile out both behaviors and
never depend on `PATH` for ADB resolution.

## 5. Verification

```bash
npm --prefix apps/emuchef-app run typecheck
npm --prefix apps/emuchef-app run test:logic
npm --prefix apps/emuchef-app run test:security
npm --prefix apps/emuchef-app run check:packaged-resources
npm --prefix apps/emuchef-app run build
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --all-targets -- -D warnings
```

