# EmuChef

EmuChef is a Rust application for planning and applying reproducible Android
handheld configurations. The repository contains separate React/Tauri apps for
the guided end-user workflow and authored recipe editing.

## Runtime

Rust is the sole product runtime. It owns the `emuchef` CLI, authored-data
validation, planning, execution, real-ADB apply, the editor document protocol,
and the JSONL sidecar used by Tauri. The retired Python implementation, tests,
package metadata, and entrypoints are absent from the repository. Python is not
a product, development, test, packaging, or release prerequisite.
The canonical repository policy and evidence checklist are documented in
[Phase 4A Python runtime retirement](docs/product/phase-4a-python-runtime-retirement.md).

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

The Rust sidecar also implements the additive Phase 0 contract used by the
end-user application: resolved catalog inventory, canonical reviewed-plan
digests, target-bound real and simulated execution, retained recipe-grouped
reports, ordered incremental events, and cooperative cancellation. Filesystem
roots are configured when the sidecar starts. Execution has no rollback or
device-state undo. See the
[Phase 0 runtime contracts](docs/product/phase-0-runtime-contracts.md).

## End-User App

```bash
npm --prefix apps/emuchef-app install
npm --prefix apps/emuchef-app run tauri:dev
```

`apps/emuchef-app` launches and negotiates the Rust sidecar independently of
ADB availability. The app provides Connect Device, Confirm Device, Choose
Setup, Provide Inputs, Review Plan, and Simulated Run. Ordinary builds execute
only the retained reviewed plan through fake-device dry-run adapters; this
makes no real device changes and is not real-device evidence. Guarded
real-device execution is implemented but unavailable unless a release build is
explicitly compiled with the default-off `real-execution` feature.

The implemented, default-disabled trust boundary for the real-device workflow
is documented in the
[Phase 2B guarded real-execution contract](docs/product/phase-2b-guarded-real-execution.md).
Platform-specific packaged-device evidence, privacy/security approval, an
operator runbook, and a separate release decision remain required before any
release build enables it. Execution-history persistence, resume, rollback, and
remote catalog paths remain unavailable.

Phase 3A adds named portable saved configurations and a private recent-file
index to the end-user app. A saved configuration contains a selected
device-plan reference, selected recipes, and user bindings, not a generated
execution plan or runtime authority. Opening or reusing one always performs a
fresh device probe, validation, description, plan generation, and review. See
the [Phase 3A contract](docs/product/phase-3a-saved-configurations.md).

Phase 3B adds a Support & Storage panel with a bounded sanitized local
diagnostics ZIP and opaque management of the app-owned artifact cache. The
end-user app injects its application-data cache root without changing backend,
CLI, or config-editor defaults. Cache payloads and optional metadata are one
logical entry. See the
[Phase 3B contract](docs/product/phase-3b-support-diagnostics-and-cache.md).

Phase 3C adds keyboard-complete interaction, semantic landmarks and
collections, accessible validation and progress, bounded live announcements,
focus-contained dialogs with exactly-once safe teardown, deterministic focus
fallbacks, reduced-motion and forced-colors handling, and narrow-window/zoom
resilience. It changes React presentation only; Rust and trusted Tauri retain
all runtime, device, file, diagnostics, cache, and execution authority. See the
[Phase 3C contract](docs/product/phase-3c-accessibility-and-interaction-resilience.md).

Phase 3D adds one bounded, atomic, app-owned recovery draft for dirty portable
intent. Startup offers Restore, Discard, or Not now before device selection;
Not now retains the draft for a future launch, while newer dirty intent
atomically supersedes it. Recovery excludes all runtime authority and omits
sensitive binding values according to authored input metadata. See the
[Phase 3D contract](docs/product/phase-3d-crash-safe-draft-restoration.md).

Phase 3E qualifies the end-user app's thin Apple Silicon macOS package. The
maintained local path builds an ad-hoc-signed app and DMG, verifies their
sidecar, resources, metadata, signatures, capabilities, and path independence,
and runs a clean copied-app probe. Its sanitized manifest distinguishes
normalized semantic repeatability from per-build raw artifact hashes; it does
not claim byte-identical signed apps or DMGs. See the
[Phase 3E contract](docs/product/phase-3e-macos-packaging-and-release-readiness.md).

Android SDK Platform-Tools is a user-supplied prerequisite. EmuChef does not
bundle or download it. The app links to Google's official page and can import a
macOS Platform-Tools ZIP through a native picker into validated application
data. See the [Phase 1 review contract](docs/product/phase-1-read-only-app.md),
the [Phase 2A simulated execution contract](docs/product/phase-2a-simulated-execution.md),
and [Platform-Tools import policy](docs/product/platform-tools-import.md).

## Config Editor

```bash
cd apps/config-editor
npm install
npm run check:rust-runtime
npm run tauri dev
```

The editor launches the Rust `emuchef --sidecar` process. No alternate backend
or backend selector exists.

## Current Limitations

Artifact resolution supports absolute `file://`, HTTP, and HTTPS URLs. Network
downloads use strict Rustls verification, bounded redirects and timeouts,
same-directory partial files, and no-clobber cache publication. Developer ID
signing, hardened runtime, app and DMG notarization, stapling, and local
Gatekeeper validation are complete for the recorded Config Editor macOS release
artifacts. The end-user app has qualified Apple Silicon ad-hoc packaging and an
explicit credentialed verification path, but it has no recorded Developer ID,
notarization, stapling, Gatekeeper, or clean-Mac evidence. Updater support and
cross-platform release automation remain future work. The packaged frontend
uses a local-only production CSP. The
completed local and HTTP(S) device run is
recorded in the
[2026-07-11 RetroArch evidence](docs/release/evidence/real-device-retroarch-2026-07-11.md).

See [runtime ownership](docs/architecture/runtime-ownership.md), the
[planner/executor architecture](docs/architecture/planner-executor.md),
[runtime recipe configuration](docs/architecture/runtime-recipe-configuration.md),
[Phase 1 read-only app](docs/product/phase-1-read-only-app.md),
[Phase 2A simulated execution](docs/product/phase-2a-simulated-execution.md),
[Phase 2B guarded real execution](docs/product/phase-2b-guarded-real-execution.md),
and [release readiness](docs/release/release-readiness.md). Real-device evidence is
collected with the [RetroArch validation runbook](docs/manual/real-device-retroarch-validation.md),
and packaged macOS evidence uses the
[Config Editor validation runbook](docs/manual/macos-packaged-gui-validation.md).
End-user app packaging follows the
[Phase 3E packaging contract](docs/product/phase-3e-macos-packaging-and-release-readiness.md).
The completed Developer ID and notarization result is recorded in the
[macOS signing evidence](docs/release/evidence/macos-signing-notarization-2026-07-11.md),
and releases follow the
[signing and notarization runbook](docs/manual/macos-signing-notarization.md).
