# macOS Packaged Config Editor Readiness Result

Date: 2026-07-11

This report records automated release-bundle, packaged-sidecar, packaged-runtime,
and CSP evidence. No interactive editor workflow is claimed as passed.

## 1. Commits

| SHA | Purpose |
| --- | --- |
| `13b596d54a9a58e48b8a2fb2fd41f920644dcac0` | Record the completed real-device artifact validation. |
| `8c62776da6be229528d5fc26682c6e9c1a6f65df` | Correct the real-device validation runbook. |
| `1a888d0d783065bcfc13e299faae05b964046cd2` | Add the macOS packaged Config Editor runbook. |
| `820a15e944612d4ee88e9fa12380d757228bd0f4` | Add bundle, packaged-app, sidecar, and packaged-runtime inspection automation. |
| `02725e69263fffd31794e10226be3361bd8b4d15` | Harden the production Tauri CSP and isolate development settings. |

The real-device trusted HTTP(S) validation tested product commit
`5dca50603cf3a4831867c229157a94906151cbb7`.

## 2. Build Environment and Command

| Field | Result |
| --- | --- |
| Host | macOS 26.5, build 25F71 |
| Host architecture | `arm64` |
| Release command | `npm run tauri build` from `apps/config-editor` |
| Sidecar preparation | Tauri `beforeBuildCommand` ran `npm run sidecar:build` before the frontend build. |
| Signing input | No Developer ID identity or notarization input was supplied. |

The maintained build completed successfully and reported two bundle outputs.
The paths below were discovered from the actual Tauri output rather than used
as hard-coded build requirements:

1. App:
   `apps/config-editor/src-tauri/target/release/bundle/macos/EmuChef Config Editor.app`
2. DMG:
   `apps/config-editor/src-tauri/target/release/bundle/dmg/EmuChef Config Editor_0.1.0_aarch64.dmg`

## 3. Bundle Identity, Layout, and Digests

| Field | Result |
| --- | --- |
| Product name | `EmuChef Config Editor` |
| Application identifier | `com.emuchef.configeditor` |
| Version | `0.1.0` |
| Main executable | `Contents/MacOS/emuchef-config-editor` |
| Bundled Rust sidecar | `Contents/MacOS/emuchef` |
| Minimum macOS value | `10.13` from `LSMinimumSystemVersion` |
| Main architecture | Mach-O 64-bit `arm64` |
| Sidecar architecture | Mach-O 64-bit `arm64` |
| App tree digest | SHA-256 `5d04454cd29ed3aa8505819a8d907d1af57ff26159b0abec28c242260a533397` over sorted relative paths and file digests |
| Main executable digest | SHA-256 `108de76e456ec741fee0de374603ccc30ca82b1918f4a2d0a53e743e7ffe6cec` |
| Sidecar digest | SHA-256 `2450459db392a2d912000e40df1ccd48a03d70818696aaa988444d68cd6d3945` |
| DMG digest | SHA-256 `b38acf674860efa80a05d9bfadc1d55dfdd6365221b50e6cb844ffc2f599222b` |

Tauri embedded the production frontend in the main executable. The app did not
contain a standalone `Contents/Resources` tree; the required Info.plist, main
executable, sidecar, and embedded `index.html`/product markers were present.

## 4. Info.plist and Signing Inspection

Info.plist contained the expected `APPL` package type, display name, executable,
identifier, short version, and bundle version. Both executables were regular,
executable files.

`codesign -dv --verbose=4` reported:

- linker-generated ad-hoc signing;
- signing identifier `emuchef_config_editor-a0a30304717671ef`;
- no TeamIdentifier; and
- no Developer ID authority.

No quarantine attribute was observed during the local bundle inspection. This
does not establish notarization, Gatekeeper acceptance, or behavior on another
machine.

## 5. Static Bundle Inspection

`npm run check:macos-bundle -- <resolved-app-path>` passed. It verified:

1. Info.plist identity and version.
2. Main and sidecar presence, executable bits, and host-matching architecture.
3. Embedded production frontend markers.
4. Unsigned/ad-hoc local signing policy.
5. Dynamic-library dependencies.
6. Absence of Python files, executables, frameworks, and libraries.
7. Absence of `emuchef-python-legacy`, `emuchef-plan-shadow`, and `plan_shadow`.
8. Absence of a Vite or numbered loopback development-server URL.

The required repository searches found development URLs only in the explicit
development configuration and CSP negative-test material. No development URL,
Python runtime, legacy runtime, or shadow planner was found in the release
bundle.

## 6. Packaged Application and Sidecar Smoke

`npm run smoke:macos-packaged-app -- <resolved-app-path>` passed:

1. The main application remained running during the bounded process check.
2. The exact bundled `Contents/MacOS/emuchef --sidecar` process was observed.
3. Direct JSONL `hello` and `ping` requests to that exact executable returned
   two successful responses with the expected transport ids.
4. Direct sidecar stderr was empty.
5. Application stdout and stderr were empty during the bounded launch.
6. The application and sidecar were terminated after the smoke, and no bundled
   sidecar process remained.

This is process and protocol evidence. It is not a visual launch, window,
menu, dialog, or document-workflow pass.

The existing simulated packaged-sidecar Rust test also remained in the Tauri
test matrix and passed.

## 7. Packaged-Runtime HTTP and TLS Evidence

`npm run smoke:packaged-runtime-network -- <resolved-app-path>` targeted the
exact bundled `Contents/MacOS/emuchef` executable and passed three tests:

1. Local HTTP cold apply downloaded once; warm apply made no additional
   request; offline warm-cache apply succeeded after the server stopped; cache
   bytes remained identical; no partial file remained.
2. A local HTTP 404 produced `artifact_http_status`, retained executor blocking
   semantics, redacted the query secret and response body, and published no
   cache file.
3. A local self-signed HTTPS server failed closed with
   `artifact_tls_verification_failed`, redacted the query secret and response
   content, and published no final or partial cache file.

No CA was added to a system or user trust store, and no product custom-CA or
verification bypass was added. Positive trusted-HTTPS evidence remains the
completed real-device run recorded in
`docs/release/evidence/real-device-retroarch-2026-07-11.md`.

This is packaged-runtime validation. The Config Editor has no plan/apply UI,
Tauri command, or JSONL sidecar request; GUI-through-sidecar plan/apply remains
an unsupported future product feature and is not a regression.

## 8. Content Security Policy

Before this milestone, `app.security.csp` was `null`, and the production binary
contained `http://localhost:5173` from the shared development configuration.

The production configuration now contains:

```text
default-src 'self'; base-uri 'none'; connect-src ipc: http://ipc.localhost; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self'
```

The release bundle contains that policy and no Vite URL, wildcard source,
`unsafe-eval`, or `unsafe-inline`. HTTP(S) artifact traffic remains in Rust, so
the frontend does not receive arbitrary remote origins.

Development-only Vite and HMR settings live in `tauri.dev.conf.json`. The only
development inline allowance is `style-src 'unsafe-inline'`; scripts remain
self-only, connections remain limited to self, Tauri IPC, and the exact Vite
HMR WebSocket, and `unsafe-eval` remains forbidden. A bounded `tauri dev`
smoke reached the Vite server and launched the debug application before being
intentionally terminated; no visual behavior is claimed.

The CSP regression suite passed positive and negative cases for disabled CSP,
wildcard/default sources, wildcard scripts or connections, `unsafe-eval`,
production inline sources, and development-setting leakage into production.

## 9. Automated Verification

| Verification | Result |
| --- | --- |
| Rust backend format | Passed. |
| Rust backend check | Passed. |
| Rust backend tests | 352 passed, 7 ignored. |
| Rust backend clippy | Passed with `-D warnings`. |
| `npm ci` | Passed; 110 packages audited, 0 vulnerabilities reported. |
| `npm run check:rust-runtime` | Passed, including packaging, CSP, Rust-only, type, and 94 frontend logic checks. |
| Frontend typecheck | Passed. |
| Frontend logic tests | 94 passed. |
| Frontend production build | Passed. |
| Release sidecar preparation | Passed for `aarch64-apple-darwin`. |
| Tauri format | Passed. |
| Tauri check | Passed. |
| Tauri tests | 29 passed. |
| Tauri clippy | Passed with `-D warnings`. |
| Tauri release build | Passed; app and DMG produced. |
| macOS bundle inspection | Passed. |
| Packaged application/sidecar smoke | Passed. |
| Packaged-runtime HTTP/TLS smoke | 3 passed. |
| `git diff --check` | Passed before each commit and before this report. |

The npm installation reported that `esbuild` and `fsevents` have install
scripts not covered by npm's optional `allowScripts` review. Installation,
builds, and tests still completed; no dependency or lockfile change was made in
this milestone.

## 10. Interactive GUI Status

No manual visual or interactive Config Editor test was completed in this
milestone. The following remain pending operator evidence:

1. First and second visible launches through Launch Services.
2. Authored-root selection and display.
3. Recipe open, validation, and all supported editor sections.
4. Undo, redo, Save, Save As, and reopen.
5. Dirty-open and dirty-close prompts.
6. In-flight operation guards.
7. Backend restart UI and document-session recovery.
8. Saved-content persistence across application relaunch.
9. The safe interactive failure and recovery cases in the runbook.

The exact operator procedure is
`docs/manual/macos-packaged-gui-validation.md`.

## 11. Remaining Release Classification

| Item | Classification | Evidence or boundary |
| --- | --- | --- |
| Real-device HTTP(S) artifact validation | Completed | 2026-07-11 evidence at tested commit `5dca50603cf3a4831867c229157a94906151cbb7`. |
| macOS release bundle build | Completed | App and DMG produced by the maintained Tauri command. |
| macOS static bundle inspection | Completed | Maintained inspector passed. |
| Packaged sidecar smoke test | Completed | Exact bundled sidecar hello/ping and launch observation passed. |
| Packaged interactive GUI validation | Pending manual validation | No visual/editor claims were made. |
| Hardened CSP | Completed | Production policy and regression tests passed. |
| Signing | Out of scope | Local bundle is ad-hoc signed only. |
| Notarization | Out of scope | Not configured or claimed. |
| Updater | Out of scope | Not implemented in this milestone. |
| Windows packaging | Out of scope | No Windows bundle was built. |
| Linux packaging | Out of scope | No Linux bundle was built. |
| Cross-platform release automation | Out of scope | Current automation covers the local macOS bundle. |
| Release artifact inspection | Completed for this macOS build | Other target artifacts remain future work. |
| Final frozen Python source deletion | Out of scope | Frozen reference source remains intentionally present. |

## 12. Cleanup and Final Disposition

After the bundle paths, digests, inspections, and smoke evidence were recorded,
ignored Rust targets, Tauri targets, frontend distribution files, generated
sidecar inputs, runtime/cache directories, `.DS_Store`, and `__pycache__`
outputs were removed. `.codegraph`, `.venv`, `node_modules`, lockfiles, authored
YAML, and fixtures were preserved.

Automated macOS packaging and runtime readiness checks passed. Public release
readiness remains blocked on the pending interactive packaged-GUI checklist and
the separately scoped signing, notarization, updater, and cross-platform release
work. Nothing in this report claims Developer ID signing, notarization,
Gatekeeper acceptance on another machine, Windows/Linux packaging, updater
readiness, or interactive GUI success.
