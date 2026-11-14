# KONKR Pocket Advance RetroArch configuration qualification plan

## 1. Scope and status

This plan covers physical qualification of RetroArch configuration creation on the owner-confirmed KONKR Pocket Advance. The device uses the authored profile `konkr.pocket_advance` and the explicit qualification target `konkr-pocket-advance-retroarch-config-create`. Its status remains `pending_physical_qualification`, outside the four canonical device workflows.

The host candidate is based on `281d0929c253f187ef622e8be1dd6f438c566f5d`. The pinned RetroArch APK has SHA-256 `361c7ed7b510cf14dbc37a434dace8d201f0f445887c89735d45788df7affc11` and package ID `com.retroarch.aarch64`.

No Android command has been run as part of this plan. APK installation, creation of missing directories, a protected-write probe, configuration creation, and device cleanup each require explicit owner authorization.

The sidecar request carries legacy root facts and the verified root route. It rejects caller-supplied `configCreateWriteEvidence`. Tauri retains destination-scoped evidence in native state and does not send it across IPC. The backend has no production source for this evidence, so real configuration creation remains fail-closed. A trusted evidence bridge and KONKR device-session binding are still required.

## 2. Device identity and selection

The owner identifies the retail device as a KONKR Pocket Advance. Read-only observations recorded ARBOR / GT78-VN, Android 12 / API 31, `arm64-v8a`, board `k85v1_64`, build ID `SP1A.210812.016`, and SELinux enforcing. A fresh `su -c id` check returned UID 0. The firmware fingerprint contains Android 11 release text. Keep the Android version and fingerprint observations separate until fresh read-only observations explain the discrepancy.

The AYANEO Pocket Air Mini can report the same ARBOR / GT78-VN identity. Select the KONKR target explicitly; do not use shared model facts or root state to choose a profile. Preserve the Air Mini profile and its evidence separately. The target record contains no serial. Brand, product, device, and connection-speed facts must be re-observed before canonical registration.

Prepare host artifacts for the explicit target:

```sh
npm run device-qualification:retroarch-config-create:prepare -- \
  --target-id konkr-pocket-advance-retroarch-config-create
```

Preparation checks the sidecar and Tauri package. It does not launch EmuChef or select an ADB device. The KONKR profile has no authored device plan, and the current device-session contract accepts only registered canonical targets. Physical execution remains blocked until a reviewed plan and session contract bind this target to the selected device and current authority.

## 3. Build identity, pins, and recovery

The source gate requires a clean worktree, including non-ignored untracked files. It records `sourceWorktreeClean` from `git status --porcelain --untracked-files=all` and stops sidecar publication while source files are dirty. Qualification builds require `EMUCHEF_CONFIG_CREATE_QUALIFICATION=1` and the exact `EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID`.

The qualification backend enables `real-execution` and `android-copy-commit-helper` together. Ordinary debug and production profiles keep real execution disabled. The qualification launcher uses Tauri's development profile; release packaging with real execution is unsupported. Runtime status reports qualification mode as enabled only when debug mode, the compiled feature, runtime opt-in, and embedded build identity all agree.

The build manifest is `crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json`. It binds the source commit, canonical material digest, material-input digest, target ID, backend feature list, embedded ARM64 helper digest, and both sidecar SHA-256 digests. The Tauri external binary is `apps/emuchef-app/src-tauri/binaries/emuchef-<host-target>`. The same qualified sidecar bytes are placed at `crates/emuchef-rust-backend/target/debug/emuchef`; the debug runtime resolves that path before the repository-level fallback. The configured `externalBin` path supplies the packaged sidecar.

All sidecar hooks use the Git-admin lock returned by:

```sh
git rev-parse --git-path emuchef/retroarch-config-create-sidecar-build.lock
```

Build and release operations keep the owner lock for the full protected operation. A short-lived coordination record serializes borrower registration with owner release and recovery. The coordination file is `<lock-path>.coordination`; the borrower registry is `<lock-path>.borrowers`.

A release hook registers a complete borrower record with an atomic no-clobber link. The record binds the owner token to the lock file's device and inode. Its output paths must stay under an artifact root listed by the owner. The hook keeps the record open until its build finishes and passes the open descriptor to Cargo. A normal hook removes only its own record after checking its token and inode. An interrupted hook leaves its record for recovery. Owner release holds the coordination gate while it checks the registry and retires the owner record. Any borrower entry, including malformed or temporary files, blocks normal release.

The Tauri packaging process inherits the owner lock descriptor. Sidecar build processes inherit their owner or borrower descriptor. This lets recovery find active children by file identity if the recorded parent process has ended. The process ID alone cannot establish that a borrower has finished.

Preparation stages both sidecar copies, writes the manifest and state marker through unique flushed sibling files, and publishes each without replacing an existing pin. If temporary-link cleanup fails after publication, the canonical lock stays active and its release handle retries cleanup. A failed retry leaves the temporary record for inspection and emits a warning. If publication or rollback cannot finish, preserve the build lock and its evidence for recovery.

The development launcher runs:

```sh
tauri dev --config src-tauri/tauri.dev.conf.json --features real-execution
```

Its `beforeDevCommand` verifies and reuses the pinned sidecar. `get_device_qualification_mode_status` must report `enabled: true` before qualification use.

The state marker and process lease live in Git's administrative directory. Before sidecar verification, the launcher obtains the shared lock and creates a target-bound lease containing the host, process ID, and unique token. It marks `sessionLaunchAttempted` before starting Tauri. An interrupted initial lease write leaves no final lease path. If lease creation fails, the marker remains false. If marking the attempt fails after lease creation, the lease stays in place. A verification failure before Tauri starts leaves a cancelled-before-start lease for review and archival. The native app claims the verified lease before setup, records its process ID, and marks a clean exit after the Tauri event loop returns. A wrong token or target, replaced lease, or incomplete artifact identity blocks startup.

### Recovering an interrupted build lock

Acquire the coordination gate before inspecting the lock or borrower registry. If a live gate blocks you, wait for that operation to finish. If the gate times out, preserve its exact record. Confirm that its host matches the current machine, its recorded process has ended, and `lsof` reports no process holding the gate file. Copy the gate record to a new evidence directory outside the repository and verify the copied bytes. With other build, launch, and recovery operations kept from starting, recheck the gate's device, inode, and token, then remove only that gate record. Do not use elapsed time as evidence that a gate is stale. Leave malformed or uncertain gate records in place.

With the gate held, preserve the exact owner lock bytes and every listed regular artifact that remains available. Inspect the owner lock and each borrower record without following symlinks. Require the owner process ID to return `ESRCH` and require `lsof` to report no owner-lock descriptor holders. For each borrower, require a local host, a matching owner token and lock device/inode, no open descriptor holders, and an ended recorded process. A live process ID without an open descriptor is uncertain because the process ID may have been reused; leave the lock in place.

Copy the owner lock, all borrower records, and each available listed artifact into a new evidence directory outside the repository. Read every copy back and record its SHA-256. Reject paths outside the repository and symlinks. Missing artifacts make recovery incomplete. Write and verify a recovery receipt before removing anything. Then recheck the original lock and borrower record bytes and inodes while still holding the gate. Remove only those verified borrower records and the identified owner lock. Preserve temporary `*.tmp-*` files and unexpected registry entries for inspection; do not remove them automatically. If any read, identity, process, or descriptor check is uncertain, leave all source records in place and do not report complete recovery.

If the manifest or state marker remains after lock recovery, use the guarded release command:

```sh
npm run device-qualification:retroarch-config-create:release-state -- \
  --target-id konkre-pocket-advance-retroarch-config-create \
  --confirm-ended-target konkre-pocket-advance-retroarch-config-create \
  --disposition finished \
  --evidence-dir /absolute/path/to/qualification-evidence
```

For an abandoned session, use `--disposition abandoned` and provide a non-empty `--reason`. The release command holds the shared lock from artifact verification through pin removal. It writes and reads back a complete receipt in a unique staging directory, then installs the archive before deleting pins. The manifest and lease are removed before the state marker. Retrying receipt installation requires the same target, disposition, reason, and evidence path; the command verifies the receipt and archived ownership record before completing cleanup. A directory without a valid receipt is preserved. An incomplete `.partial-*` directory remains evidence and does not clear the pin.

A session can be abandoned without a lease only when the marker proves launch never began. A missing lease after an attempted or ambiguous launch blocks release. Finished release requires a clean native exit and both the app and launcher processes to have ended. Abandoned release requires an explicit reason and ended recorded processes. Sidecar binaries remain in the worktree. After release, ordinary release hooks use the release-profile sidecar.

## 4. Physical qualification sequence

1. Discover connected ADB devices and explicitly select exactly one online serial. Refresh manufacturer, brand, model, device, product, board, Android version/API, ABI, firmware fingerprint, SELinux mode, RetroArch package state, and expected directory state. Sanitize the serial in retained evidence.
2. Confirm the owner-attested KONKR identity. Record Android 12 / API 31 and the Android 11 fingerprint discrepancy unless fresh observations resolve it. Run a fresh `su -c id` check and require UID 0 with SELinux enforcing.
3. Install only the pinned APK. Do not launch RetroArch. Verify the installed package and APK identity.
4. Inspect `/storage/emulated/0/Android/data/com.retroarch.aarch64/files` and its parents, ownership, and labels. If a required component is absent, stop until the owner approves creation of the exact missing directories and the preparation method establishes Android-compatible ownership and labels. Do not apply recursive permission or label changes.
5. Run the separately authorized protected-write probe described below. Root access alone does not grant write evidence. If the probe cannot return exact bytes and remove its own marker, leave `appDataWrite` unavailable and stop.
6. Review and create `retroarch.cfg` through the production configuration path at `/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg`. Create it only after confirming the exact target is absent. The production adapter must verify the staged digest and use exclusive, no-follow creation. An existing target must remain unchanged. Current backend admission blocks real creation until the trusted evidence bridge and session binding exist.
7. Read the completed file independently through a fresh root-shell operation. Compare its exact bytes with the reviewed rendered configuration and calculate SHA-256 on the host. Retain the review, adapter result, readback digest, and sanitized device identity together.
8. Inspect probe residue and unexpected target changes. Keep RetroArch, the successful configuration, and required parent directories installed for later qualification. Routine cleanup must not remove them.

APK installation, missing-parent preparation, the probe, and configuration creation are separate device mutations. Each requires owner authorization before execution.

## 5. Probe and race limits

The proposed probe uses a cryptographically random lowercase hexadecimal nonce in the exact app-data directory:

```text
/storage/emulated/0/Android/data/com.retroarch.aarch64/files/.emuchef-config-create-probe-<32-hex>.tmp
```

The probe creates one marker with exclusive, no-follow semantics, writes a fixed-format nonce payload, reads it through a separate root-shell operation, and compares exact bytes and a host-computed SHA-256. Cleanup may unlink only the exact nonce path after confirming it is a regular file with the expected payload. Retain the probe result and cleanup result before removing the marker. An incomplete probe does not enable `appDataWrite`.

The approved capability policy preserves profile restrictions, verified root routes, session checks, and execution guards for established rooted workflows. Legacy `appDataWrite` projection remains available when the profile and current root authority permit it. Managed and uploaded configuration creation through `app.retroarch.configure` and `app.retroarch.provision/seed_retroarch_cfg` requires separate current evidence for the resolved destination. That evidence does not grant general `appDataWrite`. Explicit `none` leaves unrelated provisioning outside the gate. A universal write probe would expand policy and disable established workflows, so it is outside this change.

The evidence store has no production probe writer. Root access alone cannot authorize configuration creation. Relevant device, session, root, catalog, or destination-authority changes invalidate stored evidence. Evidence for one package or App Target cannot authorize a different protected destination. Backend planning and real execution remain fail-closed until a reviewed evidence bridge and session binding share the required authority boundary.

Host tests cover existing-target and late-target behavior. Deterministic race injection is compiled only into the host test binary with `EMUCHEF_COPY_COMMIT_HELPER_TEST`; it inserts a competing file immediately before the exclusive final open. No safe physical synchronization mechanism has been qualified, so physical race testing remains blocked. Do not race an ADB process against the adapter or add a production test hook.

The successful physical configuration stays installed. A second create-only attempt against an existing target may verify no-clobber behavior only through the normal reviewed application path. The expected result is `already exists` with exact byte preservation. Planner admission may reject the request before it reaches the adapter; record that as a planner result, not an adapter test. Host fixtures create a fresh operation-owned `retroarch.cfg` for each test run and do not require device resets.

## 6. Owner authorization required for a physical run

1. Install the exact pinned APK with SHA-256 `361c7ed7b510cf14dbc37a434dace8d201f0f445887c89735d45788df7affc11`. Keep it installed and do not launch RetroArch.
2. If needed, create only the missing components of `/storage/emulated/0/Android/data/com.retroarch.aarch64/files` using an owner-approved method that preserves Android ownership and SELinux labels.
3. Create and read one nonce-named probe marker in `files`. Authorization must cover deletion of that exact test-owned marker after evidence has been retained.
4. Create `retroarch.cfg` only after confirming the exact target is absent, through the production adapter. Keep the APK, configuration, and required parent directories installed afterward.
5. Authorize an existing-target no-clobber attempt only if the normal reviewed application path can reach the adapter without bypassing planner or executor admission. Require `already exists` and byte-for-byte preservation.

The plan requests no RetroArch launch or uninstall, no overwrite or deletion of `retroarch.cfg`, no removal of parent directories, and no changes to Magisk or SELinux settings. Physical race testing remains blocked pending a separately reviewed synchronization design.

## 7. Tauri runtime path

Tauri debug mode resolves the backend first from `crates/emuchef-rust-backend/target/debug/emuchef`, then from the repository-level `target/debug` fallback. Preparation copies the qualification sidecar to the runtime path and the configured Tauri external-binary path. Runtime resolver tests and the host verifier bind the debug app to the qualified bytes.

Use the explicit qualification launcher for a future app session:

```sh
EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID=konkr-pocket-advance-retroarch-config-create \
  npm run device-qualification:retroarch-config-create:dev
```

The wrapper requires the exact target ID, passes it to preparation, and carries `EMUCHEF_CONFIG_CREATE_QUALIFICATION=1` and `EMUCHEF_DEVICE_QUALIFICATION=1` through the Tauri development command. Generic Tauri development and packaging hooks verify existing qualified artifacts when qualification mode is active. They refuse to overwrite pins while the state marker exists. The marker lives outside Cargo's target tree, so `cargo clean` does not remove the guard. A source or artifact mismatch stops the hook.

The candidate remains uncommitted, so the clean-source qualification build gate has not passed. No Tauri application has been launched for this qualification. Packaged mode resolves `emuchef` beside the application executable; the physical run must use the explicit development entry point whose debug runtime path has been exercised.
