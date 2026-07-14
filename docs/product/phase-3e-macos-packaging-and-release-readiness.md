# Phase 3E macOS Packaging and Release Readiness

## 1. Scope and qualified target

Phase 3E defines the maintained packaging and qualification contract for the
end-user `apps/emuchef-app` desktop application. The qualified target is a thin
Apple Silicon release bundle for `aarch64-apple-darwin`, with macOS 11.0 as the
minimum system version. Intel and universal bundles are not qualified.

The application bundle contains the release `emuchef --sidecar` executable,
the authored apps, device plans, device profiles, and recipes, and a checked-in
qualification policy. Android SDK Platform-Tools, user files, credentials,
certificates, API keys, provisioning profiles, ROMs, and BIOS files are never
bundled. Writable recovery, saved-configuration, cache, diagnostics, and
managed Platform-Tools state remains under Tauri-selected application-data
locations.

## 2. Local qualification

Run the complete non-credentialed qualification on an Apple Silicon Mac:

```bash
npm --prefix apps/emuchef-app run package:macos:qualify
```

The command performs a fail-closed preflight, builds the release sidecar and
frontend, creates the `.app` and DMG, verifies the bundle without executing
untrusted content, copies the app beneath a canonical temporary root, executes
the exact `--qualification-probe`, and writes
`src-tauri/target/release/bundle/emuchef-macos-arm64-qualification.json`.

Local qualification is credential-agnostic. It does not validate or reject
ambient Apple credentials. Before invoking Tauri it removes the fixed Apple
credential allowlist from the child environment and explicitly selects ad-hoc
signing with `APPLE_SIGNING_IDENTITY=-`. Its result is suitable for local
content qualification only. It does not prove Developer ID identity,
notarization, ticket stapling, Gatekeeper acceptance, or public distribution
readiness.

Individual maintained stages are available as
`package:macos:preflight`, `package:macos:build`,
`package:macos:verify`, `package:macos:smoke`, and
`package:macos:manifest`. A caller-supplied app and DMG must be supplied
together with `--app` and `--dmg`.

## 3. Bundle verification and safe smoke

The static verifier requires exactly one app and DMG and rejects:

1. An unexpected identifier, version, display name, executable name, package
   type, minimum system version, architecture, or signing state.
2. A missing, non-executable, universal, or non-arm64 main executable or
   sidecar.
3. Invalid nested or deep code signatures.
4. Missing or malformed catalog or qualification-policy resources, symlinks,
   unsupported catalog content, development files, source maps, credential
   containers, Platform-Tools, source trees, development URLs, or concrete
   developer/repository paths embedded in executable content.
5. A capability, CSP, external-binary, resource, or policy configuration that
   differs from the qualified source or enables real execution.

The smoke runs only the exact `--qualification-probe` from a copied bundle with
a temporary home, temporary data/cache locations, an unrelated working
directory, and no Apple credential variables. The trusted Tauri process starts
the normally packaged sidecar, negotiates the normal protocol, loads the
bundled catalog, performs `describeCatalog`, emits a path-free bounded report,
and exits. It requires no ADB, device, network, Platform-Tools, BIOS, ROM, or
real execution. The smoke also proves that the app copy is unchanged and the
sidecar does not survive probe exit.

## 4. Reproducibility contract

Phase 3E claims reproducible instructions and normalized semantic content, not
byte-identical signed packages. Inputs are constrained by checked-in npm and
Cargo lockfiles, a fixed target and release mode, exact resource selection,
consistent package/Tauri/Cargo versions, fixed product metadata, an explicit
ad-hoc signing default, and Rust path-prefix remapping. The manifest records
the source commit, tracked dirty state, target, build mode, qualification-policy
version, and Node, npm, Rust, Cargo, and Tauri versions.

Code signatures, signing and notarization timestamps, DMG container metadata,
filesystem mtimes, temporary names, and caller-specific absolute paths are
volatile packaging layers. They are excluded or normalized. The normalized
content digest still covers:

1. Signature-removed content hashes for the main executable and sidecar, with
   their bundle-relative identities.
2. Required `Info.plist` semantics.
3. Every authored catalog file and its relative path.
4. The qualification policy, including default-disabled real execution.
5. The complete Tauri capability document and production CSP, external binary,
   resources, hardened-runtime, and minimum-system policy.

Raw app-tree, final executable, sidecar, and DMG SHA-256 values are recorded
only as identities for that produced build. They are not expected to remain
equal after another signing or packaging run and are never compared as though
they were the unsigned hashes.

To compare two builds, retain each manifest and run:

```bash
npm --prefix apps/emuchef-app run package:macos:compare -- first.json second.json
```

The comparison passes only when normalized content digests match. Raw artifact
hash differences are reported as permitted volatility. No byte-for-byte app or
DMG reproducibility claim exists without a separate repeat-build experiment
that proves it.

## 5. Explicit credentialed release operations

Credentialed mode is selected explicitly:

```bash
npm --prefix apps/emuchef-app run package:macos:qualify -- --mode developer-id
```

Only then does the wrapper validate the fixed allowlist. It requires a
`Developer ID Application:` signing identity and either the App Store Connect
API set (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`) or the Apple
ID set (`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`). Optional supported Tauri
variables are `APPLE_PROVIDER_SHORT_NAME`, `APPLE_CERTIFICATE`, and
`APPLE_CERTIFICATE_PASSWORD`. Values remain in the caller environment or
keychain; they are never printed, placed on command lines, written to generated
configuration, serialized into manifests, or recorded as release evidence.

After the external signing/notarization operation completes, verify exact
artifacts without submitting, stapling, uploading, or publishing them:

```bash
npm --prefix apps/emuchef-app run release:macos:verify -- \
  --app /path/to/EmuChef.app --dmg /path/to/EmuChef.dmg
```

This stage requires Developer ID authority and hardened runtime on the app,
deep/strict signature validity, Gatekeeper acceptance, and valid stapled
tickets for both the app and DMG. The current application requires no custom
entitlements; broad filesystem, shell, process, unsigned-code, debugger, or
library-validation exceptions are prohibited without a separate architecture
and security decision.

## 6. Coverage and integration gates

Packaging release assurance has two mandatory, non-substitutable gates:

1. `npm --prefix apps/emuchef-app run test:packaging` enforces at least 95%
   line, branch, and function coverage over
   `macos-packaging-policy.mjs` and `sidecar-packaging.mjs`. This is the
   production pure-policy surface used by the packaging adapter. It includes
   credential isolation and validation, configuration and metadata checks,
   normalized-manifest semantics, path-leak rules, signing classification,
   artifact selection, and Developer ID metadata requirements.
2. `npm --prefix apps/emuchef-app run package:macos:qualify` is the mandatory
   integration gate for `macos-package.mjs` and `macos-packaging.mjs`. Those
   files orchestrate real filesystem traversal, Tauri, Cargo, `codesign`,
   `plutil`, `file`, `otool`, `strings`, `ditto`, process lifecycle, app/DMG
   discovery, and the copied-app probe. They are excluded from percentage
   measurement because mocks cannot establish the macOS behavior that release
   qualification requires.

Passing the percentage gate never substitutes for integration qualification,
and passing integration qualification never waives the 95% pure-policy gate.
CI runs the policy gate, performs two complete non-credentialed qualifications,
and compares their normalized manifests.

## 7. Release checklist

1. Use the recorded toolchains and a clean, reviewed source commit; verify all
   package, Tauri, and Cargo versions match.
2. Run frontend, security, packaging, default-feature, real-feature, backend,
   and shared-runtime verification.
3. Run local qualification and retain its sanitized manifest as local content
   evidence.
4. If repeatability evidence is required, run two clean local builds and compare
   normalized manifests. Do not require raw app or DMG hashes to match.
5. Select credentialed mode explicitly, keep credentials external, and run the
   signing/notarization operation.
6. Run `release:macos:verify` on the exact release app and DMG. Record a stage as
   passed only when its command actually passed.
7. Complete clean-Mac installation and launch validation before declaring a
   public release ready. Intel, universal, updater, non-macOS, and distribution
   hosting remain out of scope.
