# EmuChef End-User App

## 1. Purpose

`apps/emuchef-app` is the guided React/Tauri workflow for end users. It guides
a user through connecting one Android device, confirming detected facts,
choosing a backend-approved device plan, selecting recipes, resolving recipe
inputs, reviewing a target-bound plan, and running the retained plan through a
fake-device simulation. Guarded real-device execution is also implemented but
is absent from ordinary builds because the Cargo feature `real-execution` is
default-disabled.

The app also creates, opens, edits, saves, and reuses named schema-v1 portable
configurations. Saved files retain only a generated configuration identity,
name, selected device-plan reference, recipe IDs, and user bindings. Every open
requires a fresh device probe, current catalog validation, fresh description,
fresh plan generation, and fresh review. Generated plans, digests, device facts,
serials, and review or execution authority are never saved.

The Support & Storage panel exports a sanitized, bounded local diagnostics ZIP
and inventories the app-owned artifact cache without exposing filesystem
authority to React. Tauri fixes `<app-data>/artifact-cache` at startup and
passes it explicitly to this app's sidecar. Backend defaults and other sidecar
clients are unaffected.

Phase 3C makes every frontend surface keyboard and screen-reader operable. It
adds stable landmarks, validation summaries, bounded live announcements,
determinate execution progress, focus-contained dialogs, deterministic focus
restoration, reduced-motion and forced-colors support, and narrow-window/zoom
resilience. Promise-backed prompts settle exactly once and always use a safe
cancel result during teardown. These behaviors are presentation-only and do
not move authority out of Rust or trusted Tauri. See the
[Phase 3C contract](../../docs/product/phase-3c-accessibility-and-interaction-resilience.md).

The app keeps at most one bounded recovery record for dirty portable intent.
Tauri owns its fixed application-data path, validation, atomic replacement,
generation checks, and removal. Startup offers an accessible Restore, Discard,
or Not now choice before device selection. Not now keeps the same draft for a
future launch; a later valid dirty edit atomically supersedes it. Authored
`sensitive` input metadata is the only secret classification: sensitive or
unknown values are omitted and must be re-entered after restore. See the
[Phase 3D contract](../../docs/product/phase-3d-crash-safe-draft-restoration.md).

The implemented, default-disabled real-device trust boundary is documented in the
[Phase 2B guarded real-execution contract](../../docs/product/phase-2b-guarded-real-execution.md).
The feature is not release approval: each platform still requires packaged
disposable-device evidence, privacy/security review, an operator runbook, and
an explicit release decision. Phase 2A behavior remains unchanged.

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

Cache payloads and their optional safe metadata sidecars are one logical entry.
React receives only an opaque entry handle, safe classification, combined size,
age bucket, integrity state, and in-use/removable flags. Tauri revalidates both
components and the approved root before deletion. Cleanup is disabled while an
execution is starting or active. Diagnostics exports contain aggregate
allowlisted data only and never include configuration names or contents, paths,
serials, URLs, logs, process output, or credentials.

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
