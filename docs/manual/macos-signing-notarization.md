# macOS Signing and Notarization

This runbook produces and validates a Developer-ID-signed, notarized macOS
application and disk image. It keeps signing credentials and Apple submission
records outside the repository. Commands assume macOS and zsh.

## A. Preconditions

Before starting, confirm all of the following:

1. The operator has active Apple Developer Program access.
2. A valid Developer ID Application certificate and private key are installed
   in the login Keychain.
3. A valid App Store Connect API key is available.
4. The `.p8` private key is stored outside the repository and outside Downloads
   in a protected user-controlled directory.
5. Xcode command-line tools are installed and `xcrun` is available.
6. The tracked worktree is clean at the intended release commit.
7. Node, npm, Rust, Cargo, and the frontend dependencies are available.
8. No credential value has been placed in shell history or a repository file.

Do not paste credential values, private paths, certificate subjects, team
identifiers, submission identifiers, or raw Apple logs into repository
evidence.

## B. Verify the Signing Identity

List locally available code-signing identities:

```bash
security find-identity -v -p codesigning
```

Select a Developer ID Application identity. Do not paste its subject into a
committed file.

## C. Configure and Validate the Environment

Set the release variables only in the active shell:

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: REDACTED"
export APPLE_API_ISSUER="REDACTED"
export APPLE_API_KEY="REDACTED"
export APPLE_API_KEY_PATH="/absolute/private/path/AuthKey_REDACTED.p8"
```

Validate presence without printing values:

```bash
test -n "$APPLE_SIGNING_IDENTITY"
test -n "$APPLE_API_ISSUER"
test -n "$APPLE_API_KEY"
test -f "$APPLE_API_KEY_PATH"
stat -f '%Sp %N' "$APPLE_API_KEY_PATH"
```

The key must have no group or world permissions; mode `600` is preferred. Run
the maintained checker before building:

```bash
cd apps/config-editor
npm run check:apple-release-env
```

The checker reports policy results without printing credential values, the
identity subject, or the private path.

## D. Build

From the Config Editor directory:

```bash
npm ci
npm run check:rust-runtime
npm run check:apple-release-env
npm run tauri build
```

With the Apple variables configured, Tauri builds the Rust sidecar and
frontend, signs the application and nested executable, submits and staples the
application, then creates and signs the disk image. The current disk image must
still be submitted and stapled separately as described below.

## E. Discover the Artifacts

Do not hard-code generated artifact names in automation. From
`apps/config-editor`, discover exactly one normal Tauri application and disk
image:

```bash
setopt null_glob
app_candidates=(src-tauri/target/release/bundle/macos/*.app)
dmg_candidates=(src-tauri/target/release/bundle/dmg/*.dmg)

(( ${#app_candidates[@]} == 1 ))
(( ${#dmg_candidates[@]} == 1 ))

export APP_PATH="${app_candidates[1]:A}"
export DMG_PATH="${dmg_candidates[1]:A}"
```

Stop if either count is not exactly one.

## F. Verify the Application

```bash
codesign --verify --deep --strict --verbose=4 "$APP_PATH"
codesign -dv --verbose=4 "$APP_PATH"
spctl --assess --type execute --verbose=4 "$APP_PATH"
xcrun stapler validate "$APP_PATH"
```

The application passes only when it is valid on disk, satisfies its designated
requirement, uses Developer ID authority, has hardened runtime and a secure
timestamp, has a valid stapled ticket, and Gatekeeper accepts it with source
`Notarized Developer ID`.

## G. Submit and Staple the Disk Image

Current Tauri behavior notarizes and staples the application, but the generated
disk image may require a separate submission:

```bash
xcrun notarytool submit "$DMG_PATH" \
  --key "$APPLE_API_KEY_PATH" \
  --key-id "$APPLE_API_KEY" \
  --issuer "$APPLE_API_ISSUER" \
  --wait
```

Continue only when Apple reports `status: Accepted`. Then staple and validate
the disk image:

```bash
xcrun stapler staple "$DMG_PATH"
xcrun stapler validate "$DMG_PATH"
```

Submission and stapling are deliberate operator steps. Repository automation
must not perform them.

## H. Verify the Disk Image

```bash
codesign --verify --verbose=4 "$DMG_PATH"

spctl --assess \
  --type open \
  --context context:primary-signature \
  --verbose=4 \
  "$DMG_PATH"

xcrun stapler validate "$DMG_PATH"
```

Treat notarization and stapling as independent results. `notarized` passes only
when Gatekeeper succeeds and reports `Notarized Developer ID`; `stapled` passes
only when `stapler validate` succeeds.

## I. Install and Launch Validation

1. Open the disk image.
2. Copy the application to `/Applications`.
3. Assess the installed application:

   ```bash
   spctl --assess \
     --type execute \
     --verbose=4 \
     "/Applications/EmuChef Config Editor.app"
   ```

4. Launch the application normally.
5. Confirm no unidentified-developer warning appears.
6. Confirm the application starts and its bundled sidecar functions.

## J. Hash the Release Artifacts

Hash the two executables and disk image:

```bash
shasum -a 256 "$APP_PATH/Contents/MacOS/emuchef-config-editor"
shasum -a 256 "$APP_PATH/Contents/MacOS/emuchef"
shasum -a 256 "$DMG_PATH"
```

Build a canonical app-tree digest from file digests ordered by ascending
bytewise UTF-8 order of normalized relative paths. This is equivalent to
`LC_ALL=C sort` for the path stream used below:

```bash
app_tree_records="$(mktemp)"
(
  cd "$APP_PATH"
  find . -type f -print | LC_ALL=C sort | while IFS= read -r relative_path; do
    file_digest="$(shasum -a 256 "$relative_path" | awk '{print $1}')"
    printf '%s  %s\n' "$file_digest" "${relative_path#./}"
  done
) > "$app_tree_records"
shasum -a 256 "$app_tree_records"
rm -f "$app_tree_records"
```

The maintained manifest generator applies the same record format and refuses
to write until signed-release verification passes.

## K. Maintained Verification and Manifest Flow

Discovery mode finds exactly one normal Tauri application and disk image:

```bash
export RELEASE_MANIFEST_PATH="release-artifacts/emuchef-config-editor-0.1.0-macos-arm64.json"
npm run release:macos:verify -- --output "$RELEASE_MANIFEST_PATH"
```

Explicit mode bypasses discovery:

```bash
npm run release:macos:verify -- \
  "$APP_PATH" \
  "$DMG_PATH" \
  "$RELEASE_MANIFEST_PATH"
```

Use `--build-commit <full-40-character-sha>` when verifying artifacts built
from a different local commit. Without the option, the tracked worktree must be
clean and the current full `HEAD` is recorded. After credentials have been
unset, `--skip-env` skips credential-presence and identity checks while retaining
the repository credential-file scan.

The command verifies but never submits, staples, uploads, or publishes.

## L. Cleanup

If the `.p8` file is still in Downloads, move it to a protected directory
outside the repository, update `APPLE_API_KEY_PATH`, and restrict it:

```bash
chmod 600 "$APPLE_API_KEY_PATH"
```

After all validation and external evidence capture, remove the credentials from
the shell:

```bash
unset APPLE_API_ISSUER
unset APPLE_API_KEY
unset APPLE_API_KEY_PATH
unset APPLE_SIGNING_IDENTITY
```

Do not remove the Developer ID certificate from Keychain. Do not delete signed
artifacts until their digests and manifest have been recorded externally.

## M. Evidence Table

Use this table in a release evidence record. Repository evidence must use
artifact basenames rather than personal absolute paths.

| Field | Result |
| --- | --- |
| Date | |
| Commit SHA | |
| macOS version | |
| Architecture | |
| App version | |
| App path or basename | |
| DMG path or basename | |
| Main executable digest | |
| Sidecar digest | |
| DMG digest | |
| App-tree digest | |
| Developer ID signing | |
| Hardened runtime | |
| App notarization | |
| App stapling | |
| App Gatekeeper | |
| DMG notarization | |
| DMG stapling | |
| DMG Gatekeeper | |
| Installed application | |
| External evidence location | |
| Deviations | |
| Blockers | |
| Final disposition | |

## N. Pass Criteria

A release passes only when all of the following are true:

1. The application, main executable, and sidecar have valid Developer ID
   signatures.
2. Hardened runtime and a secure timestamp are present.
3. Application notarization is accepted by Gatekeeper as Notarized Developer ID.
4. The application ticket passes stapler validation.
5. The disk image has a valid signature.
6. Disk-image notarization is accepted by Gatekeeper as Notarized Developer ID.
7. The disk-image ticket passes stapler validation.
8. The installed application is accepted by Gatekeeper.
9. Credentials and sensitive identifiers are not committed or logged.
10. The evidence record and manifest are complete.
11. No signing or notarization error remains unresolved.
