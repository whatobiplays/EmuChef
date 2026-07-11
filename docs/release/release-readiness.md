# Release Readiness

## Available Evidence

- Rust backend format, check, tests, and clippy run locally.
- Frontend typechecking, logic tests, and production build run locally.
- Tauri shell format, check, tests, and clippy run locally.
- Host-target sidecar preparation and bundle-input inspection are automated.
- A simulated packaged sidecar smoke exercises protocol behavior.
- Local HTTP/TLS tests and process-level CLI tests cover network artifact
  downloading, strict TLS, bounded redirects/timeouts, cache reuse, and cleanup.
- [Real-device RetroArch evidence](evidence/real-device-retroarch-2026-07-11.md)
  records the local baseline, HTTP(S) cold cache, warm cache, offline warm
  cache, matching cache manifests, and successful device provisioning on
  commit `5dca50603cf3a4831867c229157a94906151cbb7`.

## Required Before Public Release

- Record the real packaged GUI checklist on every supported target.
- Define and automate code signing and macOS notarization.
- Decide and implement updater support.
- Replace the development CSP setting with a hardened policy.
- Add cross-platform release automation and artifact inspection.

Passing automated tests and one real-device evidence run does not establish
packaged-GUI or public-release readiness.
