# macOS Signing, Notarization, and Release Workflow Result

Date: 2026-07-12

This report records the repository workflow built from the completed
2026-07-11 Developer ID signing and notarization evidence. Apple credential
values, personal paths, certificate subjects, team identifiers, submission
identifiers, and raw Apple output are intentionally excluded.

## 1. Commits

| SHA | Purpose |
| --- | --- |
| `696fd1d9e224f4873cde0750cc5275bed4b829d0` | Record redacted macOS signing and notarization evidence. |
| `cb176e43b4350e3e24663f741a71c149bea8cbbb` | Add the canonical operator runbook. |
| `aa0891a652600672c280f4967b3cd96202074a90` | Add secret-safe Apple release environment validation. |
| `4139a18cdde32aaf7d90b9036b4bd1c355f5c2c1` | Add signed application and disk-image verification. |
| `43cbae18e68954848f888fd18157ae211f0e1f43` | Add deterministic release manifest generation. |
| `c5667b50b3276d4cfd5d90a8aba80a20c4705729` | Add maintained release verification orchestration. |

The already signed artifacts were built from clean source commit
`93f816fc1ea59cd034a40432e4e2a269e11eead7`.

## 2. Manual Evidence

The supplied manual evidence records successful Developer ID signing,
hardened runtime, app notarization and stapling, disk-image notarization and
stapling, local Gatekeeper acceptance, and installed-application Gatekeeper
acceptance. The redacted repository record is
`docs/release/evidence/macos-signing-notarization-2026-07-11.md`.

The repository does not contain the App Store Connect private key, credential
values, Apple account address, certificate subject, team identifier,
notarization submission identifier, or raw notarization logs. The four Apple
release variables were unset after the manual release work.

## 3. Maintained Operator Procedure

`docs/manual/macos-signing-notarization.md` is the canonical release runbook. It
covers prerequisites, identity selection, secret-safe environment setup,
release building, zsh artifact discovery, app verification, explicit disk-image
submission and stapling, Gatekeeper checks, installation, hashing, evidence
capture, cleanup, and pass criteria.

Submission and stapling remain explicit operator actions. Repository automation
does not submit, staple, upload, or publish an artifact.

## 4. Environment Checker

`npm run check:apple-release-env` validates:

1. Presence of the four supported Apple variables.
2. A regular, non-symlink private key file outside the repository.
3. No group or world permissions on the private key.
4. An installed Developer ID Application identity matching the configured
   identity.
5. Absence of credential-shaped files from the repository.

It never prints variable values, identity output, or the private path. Its unit
suite passed 6 tests. The live credential check was not rerun after credentials
had been removed; post-build orchestration used `--skip-env`, which retained the
repository credential-file scan.

## 5. Signed Artifact Checker

`npm run check:signed-macos-release -- "$APP_PATH" "$DMG_PATH"` verifies the
bundle layout, main and sidecar executable bits and signatures, Developer ID
metadata, hardened runtime, timestamps, Gatekeeper assessments, and stapler
validation. The disk image receives its own signature, Gatekeeper, and stapler
checks.

Notarization and stapling are independent:

- `notarized` is true only after a successful Gatekeeper assessment whose source
  is `Notarized Developer ID`.
- `stapled` is true only after a successful `xcrun stapler validate` command.

The checker emits safe booleans only. Its unit suite passed 7 tests, and the
retained signed application and disk image passed the real command.

## 6. Release Manifest

`npm run release:macos:manifest -- "$APP_PATH" "$DMG_PATH" "$MANIFEST_PATH"`
generates schema version 1 only after signed-release verification passes. It
records product identity and version, a locally resolvable full build commit
SHA, host architecture, artifact basenames, main/sidecar/DMG SHA-256 digests, a
canonical app-tree digest ordered by ascending bytewise UTF-8 normalized
relative paths (equivalent to `LC_ALL=C sort`), independent verification
booleans, and a UTC generation timestamp.

No absolute path or Apple signing identifier is included. Implicit `HEAD`
selection requires a clean tracked worktree. `--build-commit` requires a full
40-character hexadecimal SHA and successful local
`git cat-file -e "${SHA}^{commit}"` resolution. Its unit suite passed 6 tests.
Manifest generation passed against the retained artifacts with the recorded
source commit override.

Generated manifests remain ignored under `release-artifacts/` and are not
committed.

## 7. Release Orchestration

`npm run release:macos:verify` supports:

1. Discovery mode for exactly one normal Tauri `.app` and `.dmg` output.
2. Explicit app, DMG, and manifest paths, which bypass discovery.
3. `--output`, `--build-commit`, and `--skip-env` as documented in the runbook.

The command validates the environment or retained repository scan, verifies
signed artifacts, runs the existing bundle inspection, runs packaged
application/sidecar and packaged-runtime network smokes, and generates the
manifest. Child output is captured so raw command output and private absolute
paths do not enter the final summary.

Its unit suite passed 6 tests. Both explicit mode and discovery mode passed
against the retained signed artifacts with `--skip-env` and the recorded build
commit override.

## 8. Verification Commands and Results

The following commands passed:

```bash
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all -- --check
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets -- -D warnings

cd apps/config-editor
npm ci
npm run check:rust-runtime
npm run typecheck
npm run test:logic
npm run build
npm run test:apple-release-env
npm run test:signed-macos-release
npm run test:macos-release-manifest
npm run test:macos-release-verify

cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

npm run check:signed-macos-release -- "$APP_PATH" "$DMG_PATH"
npm run release:macos:manifest -- \
  "$APP_PATH" "$DMG_PATH" "$MANIFEST_PATH" \
  --build-commit 93f816fc1ea59cd034a40432e4e2a269e11eead7

git diff --check
```

| Verification | Result |
| --- | --- |
| Rust backend format and check | Passed |
| Rust backend tests | 352 passed, 7 ignored |
| Rust backend clippy | Passed with `-D warnings` |
| npm clean install | Passed; 110 packages audited, 0 vulnerabilities reported |
| Rust-runtime and packaging checks | Passed |
| Frontend typecheck and production build | Passed |
| Frontend logic tests | 94 passed |
| New release unit tests | 25 passed |
| Tauri format and check | Passed |
| Tauri tests | 29 passed |
| Tauri clippy | Passed with `-D warnings` |
| Real signed app/DMG checker | Passed |
| Real manifest generation | Passed |
| Explicit release orchestration | Passed |
| Discovery release orchestration | Passed |
| `git diff --check` | Passed |

The npm installation repeated the existing informational warning that two
dependency install scripts are not covered by npm's optional `allowScripts`
review. No dependency or lockfile change was made.

## 9. Credential and Identifier Searches

The required repository searches completed with these results:

- No `.p8`, `.p12`, `.cer`, or `.mobileprovision` file was found.
- No private-key contents were found.
- Apple environment-variable names occur only in documentation, tests, and
  release tooling.
- Developer ID text occurs only as a required prefix or redacted example.
- TeamIdentifier and submission text occur only as field names or redacted
  policy language; no value is recorded.
- No API key, issuer, certificate subject, team identifier, submission
  identifier, account address, or personal private-key path is committed.

## 10. Remaining Manual Work and Classification

| Item | Status |
| --- | --- |
| macOS Developer ID signing | Completed |
| macOS hardened runtime | Completed |
| App notarization and stapling | Completed |
| App local Gatekeeper validation | Completed |
| DMG notarization and stapling | Completed |
| DMG local Gatekeeper validation | Completed |
| Local installed-app validation | Completed |
| Clean separate Mac validation | Pending |
| DMG submission and stapling for each future release | Manual operator step |
| GitHub release publishing | Out of scope |
| Updater | Out of scope |
| Windows and Linux packaging | Out of scope |
| Cross-platform release automation | Pending |
| Frozen Python source deletion | Pending separate milestone |

## 11. Final Disposition

The local macOS signing, notarization, verification, hashing, and
release-preparation workflow is documented and automated without changing
product interfaces or behavior. Credentials remain external, secrets are not
logged, generated manifests remain untracked, and nothing was pushed.
