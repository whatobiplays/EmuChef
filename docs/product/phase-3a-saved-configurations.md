# Phase 3A Saved Configurations

## 1. Product boundary

Phase 3A adds named portable configurations to the end-user application. A
configuration can be created from current wizard intent, saved, saved under a
new identity, opened, edited, selected from Recents, relinked when moved, and
reused with a currently connected device.

The Rust sidecar remains authoritative for the schema-v1 document, structural
and catalog-aware validation, canonical emission, document dirty state,
persistence, configuration description, and plan generation. Tauri owns native
file dialogs, absolute configuration paths, sidecar document identifiers,
recent-file metadata, and opaque React-facing configuration handles.

## 2. Portable state and excluded authority

A saved file contains only portable configuration identity and intent:

1. Schema version, kind, generated configuration ID, and user-visible name.
2. One selected device-plan reference.
3. Selected recipe IDs.
4. User input bindings and structurally safe schema extensions.

The selected device-plan reference is an authored catalog reference. It is not
a generated execution plan. Saved files never contain a generated plan,
reviewed-plan digest, review handle, execution handle, real-execution
confirmation, launch action, sidecar request or execution ID, target serial,
probed device facts, catalog root, ADB path, current catalog identity, runtime
session state, or raw runtime error.

Opening a saved configuration preserves its device-plan reference, recipe IDs,
bindings, and stale references for diagnosis. It discards every prior generated
plan and every form of review, execution, confirmation, launch, target, and
probe authority.

## 3. Open, validation, and reuse

Saved configurations can be opened before device selection. Reuse requires the
following fresh sequence:

1. Select and probe a currently connected device through its opaque handle.
2. Match the fresh device facts against the current catalog.
3. Validate the saved device-plan reference, recipes, and bindings against the
   current catalog and current-device capabilities.
4. Generate a fresh configuration description.
5. Resolve every blocking diagnostic and generate a fresh execution plan.
6. Review that plan and receive a new opaque review handle.
7. Complete the normal simulation or default-disabled real-execution start
   checks.

No stale device plan or recipe is silently substituted. A saved configuration
may be used with a different physical device only when current matching and
compatibility validation accept the selected device-plan reference. Choosing a
different offered device plan is an explicit edit that makes the document
dirty.

Validation is presented as `valid`, `valid_with_warnings`,
`requires_attention`, or `cannot_use`. Stable diagnostic codes remain visible
in technical details. Removed plans or recipes, changed input contracts,
missing required inputs, moved host files, invalid values, and unsupported
device capabilities remain explicit until corrected.

## 4. Save, Save As, and dirty protection

Create uses a user-visible name and a generated `saved.<uuid>` configuration
ID. Save writes the current authoritative sidecar document. Save As requests a
new name, generates a new configuration ID, writes through a native save
dialog, preserves extensions and portable intent, and makes the new file the
active document. The original file and recent entry remain unchanged.

Save/Discard/Cancel protection is required before an action would lose,
replace, close, reload, or invalidate unsaved portable configuration edits.
This includes New, Open, opening or relinking another recent configuration,
runtime restart, and window close.

Platform-Tools removal invalidates device, review, execution, confirmation, and
launch authority. It does not close the active configuration document, discard
portable intent, or clear dirty edits, so removal alone does not require a
dirty prompt. A future implementation that resets the configuration session as
part of removal must prompt before that reset.

## 5. Recents and session loss

Tauri stores at most ten recent entries in private application data. Each entry
contains an opaque recent handle, embedded configuration ID, safe embedded
name, internal path, and last-opened timestamp. React receives no configuration
path or schema ID. Missing entries provide Remove and Relink actions; relinking
requires the selected file to have the same embedded configuration ID.

Frontend reload and sidecar restart invalidate all opaque document, device,
review, execution, and launch-action handles. Portable files and the recent
index remain valid, but a file must be reopened and the complete fresh reuse
sequence must run again. Active execution state is not restored or inferred.

Fresh repair retains only recipe selections and bindings whose current
contracts still match. Repaired portable intent is dirty and may be saved, but
old review or execution authority is never reused.
