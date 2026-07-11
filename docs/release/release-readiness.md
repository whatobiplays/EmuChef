# Release Readiness

## Available Evidence

- Rust backend format, check, tests, and clippy run locally.
- Frontend typechecking, logic tests, and production build run locally.
- Tauri shell format, check, tests, and clippy run locally.
- Host-target sidecar preparation and bundle-input inspection are automated.
- A simulated packaged sidecar smoke exercises protocol behavior.
- Local HTTP/TLS tests and process-level CLI tests cover network artifact
  downloading, strict TLS, bounded redirects/timeouts, cache reuse, and cleanup.

## Required Before Public Release

- Record the real packaged GUI checklist on every supported target.
- Record the [real-device RetroArch validation runbook](../manual/real-device-retroarch-validation.md)
  on a safe device.
- Record clean-cache, warm-cache, and network-unavailable warm-cache HTTP(S)
  artifact evidence on the real test device.
- Define and automate code signing and macOS notarization.
- Decide and implement updater support.
- Replace the development CSP setting with a hardened policy.
- Add cross-platform release automation and artifact inspection.

Passing automated tests alone does not establish real-device or public-release
readiness.
