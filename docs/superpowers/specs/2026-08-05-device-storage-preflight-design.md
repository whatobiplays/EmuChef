# Device Storage Preflight Design

## Decision

Add a dependency-free, repository-tracked Node CLI that prepares and cleans up reversible device-local storage pressure for reviewed physical qualification scenarios.

The utility is profile-based rather than accepting arbitrary storage thresholds. The initial `phase-6d6-low-storage` profile targets the exact Phase 6D.6 low-storage preflight window while leaving production executor, evidence, deadline, and cleanup semantics unchanged.

## Commands

```sh
node tools/device-storage-preflight.mjs status --serial <exact-serial> --profile phase-6d6-low-storage
node tools/device-storage-preflight.mjs prepare --serial <exact-serial> --profile phase-6d6-low-storage --yes
node tools/device-storage-preflight.mjs cleanup --serial <exact-serial> --profile phase-6d6-low-storage
```

`status` and `prepare --dry-run` are non-mutating. `prepare` requires `--yes`. `cleanup` requires the exact ownership marker and removes only the profile-owned directory.

## Initial profile

- Required package: `com.emuchef.fixture`
- Qualification filesystem probe: `/sdcard/EmuChefQualification/com.emuchef.fixture/output`
- Allocation parent: `/sdcard/Download`
- Owned allocation directory: `/sdcard/Download/EmuChefStoragePreflight/phase-6d6-low-storage`
- Accepted available space: `4,194,304` through `5,308,416` KiB
- Target available space: `4,751,360` KiB
- Maximum chunk: `2,097,152` KiB
- Ownership marker: `.emuchef-storage-preflight-owner`

The qualification destination and allocation parent must resolve to the same reported filesystem and mount. Profile paths and marker contents are immutable constants and are validated before use.

## Safety and ownership

The utility must fail closed unless exactly one ADB row exists, that row matches the selected serial byte-for-byte, its state is `device`, and the fixture package is installed. Serial values must contain no whitespace or shell metacharacters.

Preparation writes non-sparse zero-filled files directly on the device using `dd`. It never creates a corresponding multi-gigabyte host payload. Allocation is split into deterministic `chunk-NNNN.bin` files so an interrupted operation can resume without overwriting prior chunks. Every iteration rechecks free space and verifies that observed consumption is within the profile tolerance of the requested chunk.

The allocation directory is accepted only when its marker has the exact profile value and every other entry is a recognized chunk filename. An absent marker, unexpected entry, wrong path type, filesystem mismatch, or unexplained storage delta blocks preparation and cleanup.

Cleanup records available space before and after deletion, removes only the exact profile-owned directory, and verifies it is absent. It never removes another Downloads entry, the qualification fixture directory, user data, or an arbitrary caller-supplied path.

## Scenario relationship

The preflight allocation is operator-owned setup outside the Phase 6D.6 run-scoped fixture directory. The physical harness still creates and verifies its own one-GiB reserve and bounded filler, reports the same storage evidence, and cleans only its run-owned fixture state. After both low-storage repetitions, the operator uses this utility's cleanup command to restore the preflight allocation.

## Testing

Pure and fake-device tests cover Android `df -k` parsing, ADB inventory validation, profile validation, allocation planning, large-to-small chunk selection, target-window readiness, below-minimum refusal, filesystem matching, exact ownership markers, unknown-entry rejection, resumable chunk numbering, dry-run behavior, observed-consumption bounds, exact cleanup scope, and absence verification.
