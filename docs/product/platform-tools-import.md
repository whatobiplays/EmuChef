# Platform-Tools Import and Trust Policy

## 1. Distribution boundary

EmuChef does not bundle, vendor, redistribute, mirror, proxy, or automatically
download Android SDK Platform-Tools. It has no Platform-Tools network client,
updater, or automated license-acceptance flow. The setup UI links only to
Google's official [SDK Platform-Tools release page](https://developer.android.com/tools/releases/platform-tools), which opens in the user's default browser.

`import_platform_tools_zip` accepts no path argument from React. The command
opens the native Tauri file picker and treats the selected ZIP as untrusted.
The original ZIP remains in its user-owned location and is neither copied into
managed storage nor persisted by EmuChef.

## 2. Supported version and signer policy

The supported macOS policy is:

1. Platform-Tools 35.0.0 is the minimum supported version.
2. Releases through 37.0.0 are tested by the current Phase 1 policy.
3. A valid newer release is accepted with a visible warning that it is newer
   than the tested range.
4. A release below the minimum, an unreadable version, or disagreement between
   `source.properties` and `adb version` is rejected.
5. The binary must pass strict macOS code-signature verification with both the
   designated signer authority `Developer ID Application: Google LLC
   (EQHXZ8M8AV)` and Team Identifier `EQHXZ8M8AV`.

The signer pin was confirmed on 2026-07-13 before implementation. Google's
official release page identified Platform-Tools 37.0.0 as the current stable
release. A locally installed copy of that release reported
`Pkg.Revision=37.0.0` and `Android Debug Bridge version 1.0.41`, build
`37.0.0-14910828`. `/usr/bin/codesign --verify --strict` succeeded, and
`codesign -dvvv` reported `Authority=Developer ID Application: Google LLC
(EQHXZ8M8AV)` and `TeamIdentifier=EQHXZ8M8AV`. The imported files are still
verified independently on every import and application startup; the recorded
observation is the basis for the signer requirement, not a substitute for
runtime verification.

## 3. Accepted archive and validation sequence

Only the macOS ZIP layout with these regular files is accepted:

```text
platform-tools/adb
platform-tools/NOTICE.txt
platform-tools/source.properties
```

Before activation, the backend:

1. opens a regular ZIP without following a symlink and enforces a 128 MiB
   compressed limit;
2. rejects encryption, more than 256 entries, paths longer than 512 bytes,
   entries larger than 128 MiB, or total declared expansion above 256 MiB;
3. rejects invalid UTF-8 names, absolute paths, backslashes, traversal,
   duplicate or case-colliding names, symlinks, and special file types;
4. rejects files outside the single `platform-tools/` root and requires the
   three retained files;
5. extracts only those three files into a newly created private staging
   directory using fixed destination names and create-new semantics;
6. records SHA-256 for `adb`, `NOTICE.txt`, and `source.properties`;
7. parses the Mach-O binary and requires a native slice for the current macOS
   host architecture;
8. verifies the strict Google code signature and pinned signer identity;
9. enforces the supported version policy; and
10. runs the staged binary only as `adb version`, without a shell, in the
    staging directory, with a cleared environment, a minimal fixed `PATH`,
    closed stdin, bounded stdout/stderr, and a five-second timeout that kills
    and waits for the child.

An ADB binary is never run from the ZIP or the user's Downloads directory. It
is run only after controlled extraction, structural validation, architecture
validation, and signature validation.

## 4. Managed installation and verification

Validated files are moved atomically into a versioned directory below Tauri's
application-data directory, never into the application bundle or repository.
Backend-managed settings retain only a relative managed-install identifier,
validated version, architecture slices, signer Team Identifier, and the three
SHA-256 values. React receives only setup state, validated version, a supported
version warning when applicable, and actionable error codes/messages.

Every startup rechecks containment, regular-file identity, all three SHA-256
values, Mach-O compatibility, Google signature, version policy, and successful
controlled `adb version` execution. Managed paths are joined only after
component validation and canonical containment checks.

The normal lookup order is:

1. explicit `EMUCHEF_ADB_PATH` in debug builds only;
2. the validated application-managed installation;
3. system `adb` lookup in debug builds only when
   `EMUCHEF_ALLOW_SYSTEM_ADB=1` is deliberately set.

Packaged production behavior never depends on `PATH`.

## 5. Replacement preservation, removal, and cleanup

A replacement is fully staged and validated before activation. If validation
or settings activation fails, the previously active installation and settings
remain unchanged; this is replacement preservation, also reported as failed
replacement recovery. Successful activation removes the superseded managed
directory. Removal deletes backend settings and only the contained managed
installation selected by those settings.

Temporary staging directories are guarded and removed on success or failure.
Stale staging and inactive managed directories are cleaned during startup.
Containment checks prevent cleanup, replacement, or removal from targeting a
path outside the application-managed root.

