# Phase 3D Crash-Safe Draft Restoration

## 1. Product boundary

The end-user app keeps at most one app-owned recovery record for dirty portable
configuration intent. The record permits recovery after a process crash, app
quit, or Rust runtime restart. It is not a saved configuration, execution
history, backup, rollback mechanism, or authority to operate on a device.

Tauri exclusively chooses the fixed application-data path, validates the
record, writes it atomically, and removes it. React sends a strict portable
intent DTO and receives only sanitized status and restore projections. The Rust
sidecar remains authoritative for saved-configuration documents, current
catalog validation, configuration descriptions, plan generation, and
execution.

## 2. Record contents and limits

The schema-v1 recovery record contains only:

1. A monotonically increasing record generation, dirty flag, bounded timestamp,
   and optional safe display name.
2. The selected authored device-plan reference and selected recipe IDs.
3. Schema-permitted user input bindings and input keys whose sensitive values
   were deliberately omitted.
4. The stable embedded identity of a source saved configuration, when one
   exists.

The record is limited to 64 KiB, 256 recipes, 512 bindings, bounded strings,
bounded arrays, and bounded value nesting. It is written through an
owner-readable and owner-writable temporary file, synchronized, and atomically
renamed. Restrictive `0600` permissions are enforced on Unix platforms.
Symlinks, non-files, corrupt JSON, unsupported schema versions, oversized
records, invalid identifiers, unexpected fields, and structurally invalid
values fail closed and are removed with a sanitized startup status.

The record never contains a device serial or handle, probed facts, generated or
reviewed plan, digest, review or execution handle, confirmation, launch action,
cache or diagnostics handle, sidecar document or request ID, ADB path, native
dialog destination, environment variable, log, process output, raw error, or
active operation state.

## 3. Sensitive inputs and portable paths

Only authored input metadata controls recovery persistence. An input binding is
persisted only when the trusted current configuration description explicitly
declares `sensitive: false`. A declaration of `sensitive: true` stores only the
binding key as a missing-value marker. Missing or unknown sensitivity metadata
is handled conservatively in the same way. Key names, substrings, regular
expressions, and value shapes never classify secrecy: a neutral-name sensitive
input is omitted, while a credential-like name explicitly authored as
non-sensitive follows that metadata.

Restore exposes omitted keys only as sanitized re-entry requirements. The prior
value, a hash, a masked representation, and its length are not stored or
returned. Configuration description shows a blocking diagnostic until the user
supplies a current value, and review creation independently rejects unresolved
re-entry requirements.

File and directory input bindings follow the existing Phase 3A portable
saved-configuration rules. Phase 3D does not add another path type or broaden
which user bindings are portable. A path binding is recovery-eligible only when
its authored input is explicitly non-sensitive; current validation must still
report a moved or missing host file after restore.

## 4. Startup choices and retention

A valid record is offered through an accessible Restore, Discard, and Not now
dialog before device selection and normal workflow use:

1. Restore loads only portable intent into fresh frontend and sidecar document
   sessions. A source file is opened by its private embedded identity and an
   unsaved overlay is applied without writing the file. If the source is
   missing, the same intent opens as an unsaved recovered configuration.
2. Discard explicitly and atomically removes the offered generation.
3. Not now marks the offered generation as deferred for the current session,
   leaves it on disk, and continues with a clean temporary workflow. Clean
   shutdown does not remove a deferred generation, so it is offered again on
   the next launch.

Restoration never restores device selection, facts, description, review,
execution, confirmation, or launch authority. The user must connect and probe a
current device, validate stale references and bindings against the current
catalog, generate a fresh plan, and complete a fresh review. Removed plans,
recipes, and missing paths remain visible for repair and are not substituted.

An interrupted-session marker reports that the prior process did not finish
normally. It does not imply that an execution can be inspected or resumed;
execution state is explicitly reported as not resumed.

## 5. Generations, supersession, and clearing

React assigns increasing request and draft generations. Tauri rejects a stage
request that is not newer than the latest accepted request and draft. Each
successful atomic replacement receives a monotonically increasing record
generation, including replacements after an in-session clear, so an old
action cannot target a newer record through generation reuse.

A newer valid dirty portable intent atomically supersedes the authoritative
record, including a record deferred earlier in the same session. Once that
replacement succeeds, the older deferred generation cannot be restored or
recreated by a stale response. Runtime restart stages the current dirty intent
before invalidating transient handles and then restores the authoritative
generation into fresh sessions.

The authoritative record is cleared only by:

1. Explicit Discard of that exact generation.
2. Successful Save or Save As after the current dirty or restored intent has
   been staged.
3. Clean session finish when the record represents stale current-session data
   and the current session is no longer dirty.

Deferred and restored disposition is tracked separately from current-session
dirty state. Therefore Not now followed by a clean close does not trigger the
third rule. A newer dirty stage changes disposition to current-session and
atomically replaces the deferred record.

## 6. Accessibility and release boundary

The recovery dialog uses the Phase 3C exactly-once dialog controller, focus
containment, safe Not now dismissal, and deterministic focus behavior. Startup
remains blocked until the choice settles. Errors and interrupted-session
notices are sanitized and never include record contents or filesystem paths.

This feature does not enable real execution, packaging qualification, signing,
notarization, telemetry, cloud sync, accounts, remote catalogs, or automatic
support upload.
