# Release Readiness

## Available Evidence

- Rust backend format, check, tests, and clippy run locally.
- Frontend typechecking, logic tests, and production build run locally.
- Tauri shell format, check, tests, and clippy run locally.
- Host-target sidecar preparation and bundle-input inspection are automated.
- A simulated packaged sidecar smoke exercises protocol behavior.

## Required Before Public Release

- Record the real packaged GUI checklist on every supported target.
- Record the real-device RetroArch validation matrix on a safe device.
- Implement and validate HTTP(S) artifact downloading.
- Define and automate code signing and macOS notarization.
- Decide and implement updater support.
- Replace the development CSP setting with a hardened policy.
- Add cross-platform release automation and artifact inspection.

Passing automated tests alone does not establish real-device or public-release
readiness.
