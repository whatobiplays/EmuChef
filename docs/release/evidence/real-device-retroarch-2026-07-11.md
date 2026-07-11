# Real-Device RetroArch Validation Evidence

Date: 2026-07-11  
Tested commit: `5dca50603cf3a4831867c229157a94906151cbb7`

## Validation Scope

An operator completed the manual RetroArch validation against a disposable
Android test device. The run covered the local `file://` baseline, real-device
execution, idempotency, and the checked-in remote HTTP(S) artifact sources.
Device identity, operator identity, exact host build, raw logs, screenshots,
and user-specific paths are intentionally not stored in the repository.

## Passed Scenarios

1. The local `file://` RetroArch artifact baseline completed successfully.
2. The first real-device RetroArch apply completed with no failed or blocked
   steps.
3. The same plan completed successfully on an idempotent rerun.
4. A clean-cache run resolved the HTTP(S) artifacts successfully.
5. A warm-cache rerun completed successfully.
6. An offline warm-cache rerun completed with Wi-Fi disabled.
7. RetroArch installed, launched, and exposed the required assets, cores,
   configuration data, and system files on the device.

## Cache and Partial-File Results

The cold, warm, and offline cache SHA-256 manifests matched. No leaked
`*.partial` file was found after any recorded run. The operator-held execution
evidence reported no failed steps and no dependency-blocked steps.

## Deviations Encountered and Corrected

1. The runbook used unsupported `apply --verbose` syntax. The validation was
   rerun with the supported `apply` arguments.
2. The initial remote `adb shell su -c` inspection loop used quoting that did
   not preserve the complete loop for the remote shell. The corrected command
   passed the loop as one remote command.
3. ZIP archives using Deflate initially failed because the Rust archive reader
   did not enable Deflate support. Commit
   `5dca50603cf3a4831867c229157a94906151cbb7` added Deflate extraction support,
   and the affected apply and rerun were validated successfully.

The runbook corrections are repository documentation fixes. They do not change
the product CLI or the evidence boundary of the tested product commit.

## Evidence Retention

Raw command output, device inspection logs, cache manifests, and screenshots
remain operator-held external evidence. They were not copied into the
repository. The repository record contains no device serial, secret, username,
home directory, or user-specific temporary path.

## Remaining Unvalidated Areas

1. Optional destructive failure-matrix cases that were not attempted remain
   unvalidated rather than implicitly passed.
2. Interactive packaged Config Editor behavior remains pending.
3. Signing, notarization, updater support, Windows packaging, Linux packaging,
   and cross-platform release automation remain outside this evidence record.

