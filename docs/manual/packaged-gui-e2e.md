# Real Packaged GUI E2E Checklist

## Purpose

This checklist manually validates the installed or packaged EmuChef Config
Editor GUI artifact. It proves more than sidecar bundle-input inspection or
simulated sidecar resolution because the tester launches the packaged app
artifact and drives the UI through a real recipe editing workflow.

Do not mark real packaged GUI E2E complete unless the packaged app was built,
launched, manually driven through this checklist, and the result record below
was completed with evidence.

## Test boundary

- Host-target sidecar bundle-input checks run from `apps/config-editor` with
  `npm run check:sidecar:bundle-input`. They inspect the prepared host-target
  `externalBin` source artifact before a full packaged GUI run.
- Simulated-packaged sidecar smoke runs from `apps/config-editor` with
  `npm run smoke:sidecar:simulated-packaged`. It copies the real Rust backend to
  a temporary simulated bundle directory and exercises packaged-mode sidecar
  resolution without launching the packaged GUI artifact.
- Real packaged GUI E2E launches the packaged GUI artifact and manually drives
  the Tauri UI through opening, validating, editing, saving, Save As, closing,
  and reopening a recipe.

The simulated-packaged smoke is supporting coverage only. It is not real
packaged GUI E2E.

## Prerequisites

- Frontend npm dependencies are installed.
- The Rust and Tauri toolchain needed for the local host target is available.
- This checklist covers host-target builds only. Cross-compilation is outside
  this checklist.
- Signing, notarization, and updater validation are not required for this
  checklist.
- Testers must use a temporary recipe copy and must never edit canonical files
  under `authored/recipes/` during manual packaged GUI E2E.

## Build command

Run the packaged build from the config editor app:

```bash
cd apps/config-editor
npm run tauri build
```

## Package artifact locations

Known macOS package outputs include:

- App bundle:
  `apps/config-editor/src-tauri/target/release/bundle/macos/EmuChef Config Editor.app`
- DMG directory, if produced:
  `apps/config-editor/src-tauri/target/release/bundle/dmg/`

Non-macOS testers must record the actual artifact path printed or produced by
the local Tauri build output instead of relying on guessed paths.

## Launch instructions

For a direct macOS app bundle launch from the repository root:

```bash
open "apps/config-editor/src-tauri/target/release/bundle/macos/EmuChef Config Editor.app"
```

When testing from a DMG or installer, launch the installed artifact by the normal
OS flow for that artifact.

Record any OS security prompts, quarantine or Gatekeeper behavior, or launch
failures in the result record.

## Packaged sidecar confirmation

The expected macOS sidecar location is under the app bundle's `Contents/MacOS/`
directory. For the current package name, the expected path is:

```text
apps/config-editor/src-tauri/target/release/bundle/macos/EmuChef Config Editor.app/Contents/MacOS/emuchef-rust-backend
```

Record the actual sidecar binary path found in the package. If
`emuchef-rust-backend` or the expected platform sidecar binary is missing, record
the run as a failure and include investigation notes.

Confirm sidecar health using the app's visible status indicator or diagnostics
UI if available. If there is no explicit sidecar status UI, confirm sidecar
health by successfully opening, validating, saving, using Save As, and reopening
a recipe through the packaged app with no packaged-sidecar resolution error.

As supporting evidence only, testers may run the bundled sidecar `hello` command
directly and record the output. Direct sidecar invocation does not complete GUI
E2E by itself.

## Temp recipe setup

Create a temporary recipe copy from the repository root:

```bash
tmp_dir=$(mktemp -d)
cp authored/recipes/feature.copy_bios.yaml "$tmp_dir/feature.copy_bios.yaml"
printf '%s\n' "$tmp_dir/feature.copy_bios.yaml"
```

Open and edit only the temp copy. Never edit canonical
`authored/recipes/*.yaml` files during manual packaged GUI E2E.

## Manual checklist

- [ ] Build packaged app.
- [ ] Record exact command and result.
- [ ] Record package artifact path.
- [ ] Launch packaged app artifact.
- [ ] Open the temp recipe copy.
- [ ] Confirm no packaged-sidecar resolution error is shown.
- [ ] Confirm sidecar status is healthy if the UI exposes status; otherwise
      record that no explicit status UI exists and rely on successful
      open/validate/save/reopen behavior.
- [ ] Inspect diagnostics.
- [ ] Inspect canonical YAML preview or output.
- [ ] Edit one Overview field, preferably recipe description.
- [ ] Edit at least one structured authored-model field, preferring an input
      field if present, an artifact URL/cache field if present, or a step
      display name, params, or dependency field if present.
- [ ] Save.
- [ ] Verify dirty state returns to Saved.
- [ ] Use Save As.
- [ ] Verify the Save As file exists at the chosen path.
- [ ] Close and reopen the Save As file specifically.
- [ ] Verify diagnostics remain visible.
- [ ] Verify canonical YAML reflects both the Overview edit and the structured
      authored-model edit.
- [ ] Record screenshots/logs as appropriate.
- [ ] Mark pass/fail.

Skip sidecar failure/recovery testing unless a tester has a safe manual
procedure. Do not require destructive binary moves or mutation of the app bundle.

## Result recording template

```text
OS/arch:
Commit SHA:
Command used to build:
Build result:
Package artifact path:
Launch method:
Sidecar binary name/path:
Recipe temp-copy path:
Save As output path:
Sidecar status source:
Pass/fail:
Notes:
Screenshots:
Logs:
Direct sidecar hello output, if run:

Unchecked or failed runs are not release readiness evidence.
```
