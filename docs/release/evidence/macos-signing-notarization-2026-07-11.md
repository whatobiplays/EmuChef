# macOS Signing and Notarization Evidence

Date: 2026-07-11

This record summarizes the completed local Developer ID signing and Apple
notarization validation. Sensitive Apple identifiers, credential values,
submission identifiers, personal paths, and raw command output are intentionally
excluded.

## 1. Source and Artifacts

| Field | Value |
| --- | --- |
| Source commit | `93f816fc1ea59cd034a40432e4e2a269e11eead7` |
| Application | `EmuChef Config Editor.app` |
| Disk image | `EmuChef Config Editor_0.1.0_aarch64.dmg` |
| Application version | `0.1.0` |
| Host architecture | `arm64` |

The release build was produced from the clean `main` commit above with the
maintained Tauri build command.

## 2. Application Result

| Check | Result |
| --- | --- |
| Developer ID Application signature | Passed |
| Nested executable signature validation | Passed |
| Hardened runtime | Enabled |
| Secure signing timestamp | Present |
| Apple notarization | Accepted |
| Stapled notarization ticket | Valid |
| Gatekeeper assessment | Accepted as Notarized Developer ID |

The signing metadata contained the expected Developer ID certificate chain and
a valid team identifier. Their values are not recorded in the repository.

## 3. Disk Image Result

| Check | Result |
| --- | --- |
| Developer ID Application signature | Passed |
| Separate Apple notarization submission | Accepted |
| Stapled notarization ticket | Valid |
| Gatekeeper open assessment | Accepted as Notarized Developer ID |

The disk image was submitted separately after the Tauri build, then stapled and
validated. The submission identifier and raw Apple logs remain external.

## 4. Installed Application Result

The disk image was opened locally, the application was copied to
`/Applications`, and Gatekeeper accepted the installed application as
Notarized Developer ID. The application launched without an
unidentified-developer warning. This is local-Mac evidence and does not claim a
separate clean-Mac validation.

## 5. Credential Disposition

- The App Store Connect private key remains outside the repository in a
  protected user-controlled location with restrictive permissions.
- `APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`, and
  `APPLE_SIGNING_IDENTITY` were removed from the shell after the release work.
- No credential value, private key, Apple account address, certificate subject,
  team identifier, or notarization submission identifier is recorded here.
- Raw notarization logs remain external to the repository.

## 6. Limitations

- Validation was performed on the local Mac only; no separate clean Mac was
  tested.
- No GitHub release was created or automated.
- No updater is implemented.
- Windows and Linux packaging were not performed.
- Cross-platform release automation remains future work.

## 7. Disposition

Developer ID signing, hardened runtime, application and disk-image
notarization, stapling, local Gatekeeper assessment, and installed-application
assessment are complete for the recorded source commit and artifacts.
