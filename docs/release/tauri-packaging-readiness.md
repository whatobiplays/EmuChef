# Tauri Packaging Readiness

This document records the current cross-platform packaging readiness status for
the Tauri config editor in `apps/config-editor`. It is a release-readiness status
document only. It does not certify untested platforms, real packaged GUI E2E,
signing, notarization, updater support, or public release readiness.

The active package path is the Rust-sidecar Tauri editor path. It does not use a
Python runtime for packaged editor backend execution, but the repository still
contains Python legacy/reference/developer/golden tooling.

## Readiness Boundaries

These checks prove different things and must not be treated as equivalent:

| Boundary | Command or source | What it proves | What it does not prove |
| --- | --- | --- | --- |
| Host-target sidecar preparation | `npm run sidecar:build` | Builds the Rust backend for the current host target and prepares the Tauri v2 `externalBin` input name. | Cross-compilation, installer output, GUI launch, signing, notarization, or updater readiness. |
| Bundle-input inspection | `npm run check:sidecar:bundle-input` | Runs bundle-input unit checks, prepares the release sidecar, and inspects the host-target `externalBin` source artifact and metadata. | Real packaged app behavior, generated installer correctness, GUI E2E, signing, notarization, or updater readiness. |
| Simulated packaged sidecar smoke | `npm run smoke:sidecar:simulated-packaged` | Exercises packaged-mode sidecar path resolution against a temporary simulated bundle directory. | A real packaged GUI artifact, installed app, installer, signing, notarization, updater, or GUI E2E. |
| Tauri package build | `cd apps/config-editor && npm run tauri build` | Runs the configured Tauri build for the current host target. | Cross-platform support from another host, signed/notarized release readiness, updater readiness, or completed GUI E2E. |
| Generated platform artifacts | Tauri build output | Produces platform-specific artifacts for the host target when the local platform prerequisites are present. | Artifact quality on untested targets or release readiness without platform-local verification. |
| Real packaged GUI E2E | [`docs/manual/packaged-gui-e2e.md`](../manual/packaged-gui-e2e.md) | Manually validates a built packaged GUI artifact through the checklist when a tester records evidence. | Completion by merely running sidecar inspections, simulated packaged smoke, or direct sidecar `hello`. |
| Signing, notarization, updater readiness | Not implemented in this repo status | Nothing is certified here. | Public release readiness. |

All package and sidecar preparation commands described here are host-target
commands. They do not prove cross-compilation.

## Readiness Matrix

| Platform | Sidecar input status | Tauri bundle status | Packaged GUI E2E | Signing/notarization | Release-ready? | Evidence | Blockers |
| --- | --- | --- | --- | --- | --- | --- | --- |
| macOS arm64 | Previously evidenced for host-target release sidecar input. | Previously evidenced for a macOS arm64 app bundle and DMG output. | Not completed by current evidence. | Not implemented. | No. | [`docs/rust-backend-cutover-readiness.md` Parity Status Table](../rust-backend-cutover-readiness.md#parity-status-table) records the Packaging / distribution evidence for `npm run tauri build` on macOS aarch64, an `EmuChef Config Editor.app` output, a DMG output, bundled `Contents/MacOS/emuchef-rust-backend`, and a bundled backend `hello` success. [`docs/rust-backend-cutover-readiness.md` Test Evidence](../rust-backend-cutover-readiness.md#test-evidence) records host-target bundle-input inspection. [`apps/config-editor/README.md` Packaged Build](../../apps/config-editor/README.md#packaged-build) describes the host-target package path. | Real packaged GUI E2E evidence is absent; signing, notarization, updater validation, and public release readiness are incomplete. |
| macOS x64 | Expected host-target flow only; unverified. | Expected host-target Tauri macOS artifacts only; unverified. | Not run. | Not implemented. | No. | [`sidecar-packaging.mjs`](../../apps/config-editor/scripts/sidecar-packaging.mjs) defines target-triple naming; [`prepare-rust-sidecar.mjs`](../../apps/config-editor/scripts/prepare-rust-sidecar.mjs) rejects non-host target requests. No macOS x64 build evidence is recorded here. | Needs a macOS x64 host-target run, bundle-input inspection, generated artifact inspection, real packaged GUI E2E, and signing/notarization decision. |
| Windows x64 | Naming convention covered by pure tests; real Windows host-target input unverified. | Windows Tauri bundle unverified. | Not run. | Not implemented. | No. | [`sidecar-packaging.mjs`](../../apps/config-editor/scripts/sidecar-packaging.mjs) defines Windows `.exe` naming; [`package.json` scripts](../../apps/config-editor/package.json) expose the sidecar and smoke commands. No Windows package build evidence is recorded here. | Needs a Windows x64 host-target run, installer/artifact inspection, real packaged GUI E2E, code-signing decision, and updater decision. |
| Linux x64 | Expected host-target flow only; unverified. | Linux Tauri bundle unverified. | Not run. | Not implemented. | No. | [`sidecar-packaging.mjs`](../../apps/config-editor/scripts/sidecar-packaging.mjs) defines Unix-like naming; [`tauri.conf.json`](../../apps/config-editor/src-tauri/tauri.conf.json) enables bundling with `externalBin`. No Linux package build evidence is recorded here. | Needs a Linux x64 host-target run, Linux artifact inspection, real packaged GUI E2E, distribution/package expectations, and updater decision. |

## Platform Notes

### macOS arm64

- Expected build command: `cd apps/config-editor && npm run tauri build`
- Sidecar preparation command: `npm run sidecar:build`
- Bundle-input inspection command: `npm run check:sidecar:bundle-input`
- Simulated packaged smoke command: `npm run smoke:sidecar:simulated-packaged`
- Host-target requirement: run on a macOS arm64 host for macOS arm64 evidence.
- Sidecar input name: `emuchef-rust-backend-aarch64-apple-darwin`
- Packaged sidecar name: `emuchef-rust-backend`
- Expected artifact types: macOS `.app` bundle and DMG output when Tauri's macOS bundler prerequisites are installed.
- Required prerequisites: frontend npm dependencies, Rust toolchain, Tauri macOS prerequisites, matching macOS arm64 host/target, and the packaged Tauri Rust-sidecar path. Python is not required for the packaged Tauri editor runtime.
- Current blockers and unknowns: the readiness audit records prior macOS arm64 package and bundled-sidecar evidence, but this document does not contain a completed real packaged GUI E2E result record, signing evidence, notarization evidence, updater evidence, or public release approval.

### macOS x64

- Expected build command: `cd apps/config-editor && npm run tauri build`
- Sidecar preparation command: `npm run sidecar:build`
- Bundle-input inspection command: `npm run check:sidecar:bundle-input`
- Simulated packaged smoke command: `npm run smoke:sidecar:simulated-packaged`
- Host-target requirement: run on a macOS x64 host for macOS x64 evidence. Current scripts do not certify cross-compilation from another host.
- Sidecar input name: `emuchef-rust-backend-x86_64-apple-darwin`
- Packaged sidecar name: `emuchef-rust-backend`
- Expected artifact types: macOS `.app` bundle and DMG output when Tauri's macOS bundler prerequisites are installed.
- Required prerequisites: frontend npm dependencies, Rust toolchain, Tauri macOS prerequisites, matching macOS x64 host/target, and the packaged Tauri Rust-sidecar path. Python is not required for the packaged Tauri editor runtime.
- Current blockers and unknowns: no macOS x64 sidecar input inspection, Tauri bundle build, generated artifact inspection, packaged GUI E2E, signing, notarization, or updater evidence is recorded here.

### Windows x64

- Expected build command: `cd apps/config-editor && npm run tauri build`
- Sidecar preparation command: `npm run sidecar:build`
- Bundle-input inspection command: `npm run check:sidecar:bundle-input`
- Simulated packaged smoke command: `npm run smoke:sidecar:simulated-packaged`
- Host-target requirement: run on a Windows x64 host for Windows x64 evidence. Current scripts do not certify cross-compilation from another host.
- Sidecar input name: `emuchef-rust-backend-x86_64-pc-windows-msvc.exe`
- Packaged sidecar name: `emuchef-rust-backend.exe`
- Expected artifact types: Windows bundle or installer outputs configured by apps/config-editor/src-tauri/tauri.conf.json and supported by the local Tauri toolchain. This document does not claim that any Windows artifact has been produced or verified.
- Required prerequisites: frontend npm dependencies, Rust toolchain, Tauri Windows prerequisites, matching Windows x64 host/target, and the packaged Tauri Rust-sidecar path. Python is not required for the packaged Tauri editor runtime.
- Current blockers and unknowns: pure naming logic covers the `.exe` convention, but no Windows sidecar input inspection, Tauri bundle build, generated artifact inspection, packaged GUI E2E, code-signing, installer behavior, or updater evidence is recorded here.

### Linux x64

- Expected build command: `cd apps/config-editor && npm run tauri build`
- Sidecar preparation command: `npm run sidecar:build`
- Bundle-input inspection command: `npm run check:sidecar:bundle-input`
- Simulated packaged smoke command: `npm run smoke:sidecar:simulated-packaged`
- Host-target requirement: run on a Linux x64 host for Linux x64 evidence. Current scripts do not certify cross-compilation from another host.
- Sidecar input name: `emuchef-rust-backend-x86_64-unknown-linux-gnu`
- Packaged sidecar name: `emuchef-rust-backend`
- Expected artifact types: Linux bundle outputs configured by apps/config-editor/src-tauri/tauri.conf.json and supported by the local Tauri toolchain. This document does not claim that any specific Linux artifact type has been produced or verified.
- Required prerequisites: frontend npm dependencies, Rust toolchain, Tauri Linux prerequisites, matching Linux x64 host/target, and the packaged Tauri Rust-sidecar path. Python is not required for the packaged Tauri editor runtime.
- Current blockers and unknowns: no Linux sidecar input inspection, Tauri bundle build, generated artifact inspection, packaged GUI E2E, distribution package policy, signing policy, or updater evidence is recorded here.

## Evidence Sources

- [`apps/config-editor/README.md` Packaged Build](../../apps/config-editor/README.md#packaged-build) describes `npm run tauri build`, `sidecar:build`, Tauri v2 `externalBin` source names, target-triple stripping in packaged apps, bundle-input checks, simulated packaged smoke, and the manual packaged GUI E2E boundary.
- [`apps/config-editor/package.json`](../../apps/config-editor/package.json) defines `sidecar:build`, `check:sidecar:bundle-input`, `smoke:sidecar:simulated-packaged`, `test:sidecar-packaging`, `test:sidecar-bundle-inspection`, and related runtime checks.
- [`apps/config-editor/scripts/sidecar-packaging.mjs`](../../apps/config-editor/scripts/sidecar-packaging.mjs) defines the sidecar base name, target-triple-suffixed `externalBin` source name, Windows `.exe` behavior, and packaged sidecar launch name.
- [`apps/config-editor/scripts/prepare-rust-sidecar.mjs`](../../apps/config-editor/scripts/prepare-rust-sidecar.mjs) builds the Rust backend, reads `rustc --print host-tuple`, rejects non-host target requests, copies the host-target sidecar to `src-tauri/binaries`, and writes metadata.
- [`apps/config-editor/src-tauri/tauri.conf.json`](../../apps/config-editor/src-tauri/tauri.conf.json) configures `beforeBuildCommand` as `npm run sidecar:build && npm run build` and enables bundling with `externalBin`.
- [`docs/rust-backend-cutover-readiness.md` Phase 6X Cross-Platform And Packaging Confidence](../rust-backend-cutover-readiness.md#phase-6x-cross-platform-and-packaging-confidence) distinguishes pure naming checks, host-target bundle-input inspection, simulated packaged smoke, and remaining public release work.
- [`docs/rust-backend-cutover-readiness.md` Final Verdict](../rust-backend-cutover-readiness.md#final-verdict) records prior macOS aarch64 package evidence and states that cross-platform release automation, real packaged GUI E2E, signing/notarization, updater support, and public release readiness remain incomplete.
- [`docs/manual/packaged-gui-e2e.md`](../manual/packaged-gui-e2e.md) is the manual checklist for real packaged GUI E2E and states that unchecked or failed runs are not release readiness evidence.

## Non-goals

This document does not certify:

- Cross-compilation support
- Windows packaging
- Linux packaging
- macOS x64 packaging
- Real packaged GUI E2E
- Signing
- Notarization
- Updater support
- Full repo Python-free operation
