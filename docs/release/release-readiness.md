# Release Readiness

## Available Evidence

- Rust backend format, check, tests, and clippy run locally.
- Frontend typechecking, logic tests, and production build run locally.
- Tauri shell format, check, tests, and clippy run locally.
- The [Phase 4A Python runtime retirement](../product/phase-4a-python-runtime-retirement.md)
  guard rejects Python product code, package metadata, runtime commands,
  sidecars, and backend selectors.
- Host-target sidecar preparation and bundle-input inspection are automated.
- A simulated packaged sidecar smoke exercises protocol behavior.
- Local HTTP/TLS tests and process-level CLI tests cover network artifact
  downloading, strict TLS, bounded redirects/timeouts, cache reuse, and cleanup.
- [Phase 5B APK verification and permission-automation evidence](evidence/phase-5b-apk-verification-2026-07-18.md)
  records the backend, Tauri, frontend, release-script-test, and repository
  integrity gates for commit
  `eb1f0d7c0a5d0c1f7c6e1ff9bbd4cde0ca2fa2d1`. It is automated
  documentation-release evidence only; it does not establish packaged,
  real-device, signed, notarized, clean-Mac, or public-release readiness.
- [Real-device RetroArch evidence](evidence/real-device-retroarch-2026-07-11.md)
  records the local baseline, HTTP(S) cold cache, warm cache, offline warm
  cache, matching cache manifests, and successful device provisioning on
  commit `5dca50603cf3a4831867c229157a94906151cbb7`.
- The production Tauri CSP is local-only, permits the two required IPC sources,
  and has negative regression coverage for disabled, wildcard, eval, inline,
  and broad-connect policies.
- [macOS signing and notarization evidence](evidence/macos-signing-notarization-2026-07-11.md)
  records completed Developer ID signing, hardened runtime, app and DMG
  notarization and stapling, and local Gatekeeper acceptance for commit
  `93f816fc1ea59cd034a40432e4e2a269e11eead7`.

## Required Before Public Release

- Record the
  [macOS packaged Config Editor checklist](../manual/macos-packaged-gui-validation.md)
  and the corresponding checklist for every other supported target.
- Validate the signed and notarized application on a separate clean Mac.
- Validate Phase 5B package enforcement and selected permission actions against
  representative rooted and unrooted disposable Android devices.
- Execute the maintained
  [macOS signing and notarization runbook](../manual/macos-signing-notarization.md)
  and retain external Apple submission evidence for each release.
- Pin reviewed production Phase 4B manifest/DMG origins and the metadata public
  key, publish credentialed signed release metadata, and record clean-Mac manual
  DMG replacement evidence under the
  [Phase 4B contract](../product/phase-4b-secure-end-user-update-delivery.md).
- Add cross-platform release automation and artifact inspection.

Completed local signing and notarization for an older recorded commit do not
establish current-commit signing, clean-Mac packaged-GUI, or public-release
readiness. Signing, notarization, and artifact verification must be repeated for
each release.

Phase 4B signed discovery and manual browser handoff are implemented but
fail-closed in production because trust remains unconfigured. This is not an
in-place updater: EmuChef never downloads or verifies the local browser DMG,
installs an app, or restarts itself.
