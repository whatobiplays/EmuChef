> **Release scope authority:** This document describes existing historical
> release evidence and continuing release checks. The approved initial product
> scope and new OEM/configuration requirements live in
> [EmuChef requirements](../product/emuchef-requirements.md); exact open
> obligations and source contracts are indexed by
> [qualification obligations](../product/qualification-obligations.md).
> Evidence recorded for prior Config Editor builds, ad-hoc EmuChef packages,
> or specific device workflows is not release approval for current EmuChef
> proper. Current production `real-execution` remains default-off. T17Q
> evaluates applicable obligations for a separately approved release scope;
> T17 requires exact-build qualification and explicit release authorization.
>
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

## Mandatory for the Initial EmuChef Proper Apple Silicon macOS Release

The initial approved **EmuChef proper** release/support scope is native Apple Silicon macOS. The following checks remain **mandatory for that scope**; T17 and a separate explicit release decision govern readiness. The older Config Editor evidence is preserved but does not attest the current EmuChef proper build.

- Inspect the **current EmuChef proper macOS app/DMG artifact** (sidecar identity, metadata, entitlements, hardened runtime, signatures and path independence) and run the relevant [macOS packaged GUI validation runbook](../manual/macos-packaged-gui-validation.md) with an **EmuChef proper-specific** checklist. Prior Config Editor checklist success is not a substitute.
- Complete Developer ID signing, hardened runtime, notarization, stapling and Gatekeeper verification for the **exact release app/DMG** under the [macOS signing and notarization runbook](../manual/macos-signing-notarization.md); retain external Apple submission evidence and validate on a separate **clean Mac**.
- Qualify the selected rooted/non-rooted package, permission, configuration, copy, OEM and target-app workflows included in the **actual approved support scope**. T17Q must classify every applicable inherited/manual/physical obligation by its governing evidence contract; absence of required evidence blocks that claimed capability or release. Individual physical runs still require separate authorization.
- Review and configure the Phase 4B manifest/DMG origin and trusted metadata key, publish signed production metadata and prove clean-Mac manual DMG replacement as required by the [Phase 4B contract](../product/phase-4b-secure-end-user-update-delivery.md) before claiming that update path production-ready. Existing unconfigured/fail-closed trust remains in force until separately authorized.
- Preserve applicable macOS build reproducibility, artifact inspection, accessibility, privacy, device/identity and release-scope proof. Do not enable normal production `real-execution` merely because tracking tickets close.

## Separately Scoped and Deferred Releases

- **Config Editor:** New feature/refinement work and its public release remain owner-deferred. Historical Config Editor Developer ID/notarization and packaged-GUI evidence remain valid **for those recorded artifacts only**; Config Editor release is **not** an unconditional prerequisite for EmuChef proper.
- **Additional host platforms:** Intel/universal macOS, Windows, Linux and their cross-platform release automation/inspection/clean-host requirements remain separately scoped, not unconditional blockers for the initial Apple Silicon EmuChef proper release. If support is later explicitly approved, their own applicable evidence becomes mandatory.
- **Additional Android models/firmware:** OEM runner feature eligibility for AYN/Retroid/AYANEO is Device Profile-based, not firmware-allowlisted or per-model rollout gated. Exact supported *workflow* claims still require applicable physical and emulator-usage evidence; unsupported targets and deferred product scopes do not become automatic release obligations.


Completed local signing and notarization for an older recorded commit do not
establish current-commit signing, clean-Mac packaged-GUI, or public-release
readiness. Signing, notarization, and artifact verification must be repeated for
each release.

Phase 4B signed discovery and manual browser handoff are implemented but
fail-closed in production because trust remains unconfigured. This is not an
in-place updater: EmuChef never downloads or verifies the local browser DMG,
installs an app, or restarts itself.
