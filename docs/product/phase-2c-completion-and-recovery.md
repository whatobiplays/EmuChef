# Phase 2C Completion Reporting and Recovery

## 1. Product boundary

Phase 2C completes the end-user Apply and execution-reporting workflow. It adds
authoritative completion summaries, fixed remediation guidance, sanitized report
export, fresh-plan repair, and an optional final app-launch action. It does not
add rollback, resume, retry-in-place, persisted execution history, saved named
configurations, arbitrary ADB commands, or release qualification. Guarded real
execution remains behind the default-disabled `real-execution` Cargo feature.

All result data comes from retained sidecar execution snapshots. React never
submits a report body, package, activity, serial, ADB path, or command as
authority. Tauri retains the reviewed plan and exact target data and exposes
only sanitized DTOs and opaque session handles.

## 2. Completion states

The authoritative execution status remains the primary result:

1. `succeeded` is complete success.
2. `succeeded_with_warnings` is success with warnings.
3. `failed` remains failure even when earlier steps completed.
4. `cancelled` remains cancellation even when earlier steps completed.
5. A lost in-memory execution is an unknown outcome and must not be inferred
   from the last observed event or snapshot.

Counts for completed, skipped, blocked, failed, cancelled, and pending steps are
secondary context. A failed or cancelled real execution with completed steps
warns that the device may have partial changes; it is never relabeled as partial
success. Outcomes remain grouped by their reviewed feature/recipe.

Execution issue messages and remediation are selected from fixed Tauri-owned
code mappings. Unknown codes receive generic report-and-fresh-plan guidance.
Raw sidecar errors, command output, step outputs, serials, paths, and
credential-bearing URLs are not displayed.

## 3. Sanitized report export

A retained simulated or real terminal execution can be exported through a
native save dialog. Tauri fetches the authoritative snapshot, constructs the
document, and writes the user-selected file. React supplies only the opaque
execution handle and never sees the destination path or provides report data.

The deterministic JSON document uses schema `emuchef.execution-report`, version
`1`, and includes app/runtime metadata, public catalog identity, plan identity,
simulation and verification scope, timestamps, completion counts, grouped
feature outcomes, and sanitized issues. It excludes all internal and public
handles, sidecar execution identifiers, exact serials, filesystem paths,
credentials, reviewed-plan bodies, step outputs, and raw ADB/process output.

## 4. Fresh repair

`Retry failed work` and `Repair configuration` always return to planning. The
old review, execution, and real-execution confirmation provide no authority for
the new attempt. The app refreshes the catalog, device inventory, device facts,
match result, and configuration description before a new review can be made.

Recipe choices are retained only while their recipe identifiers still exist.
Bindings are retained only when the current input has the same key, value type,
multiplicity, and path kind. Removed or changed inputs are left unresolved so
the normal current diagnostics are shown. A real retry requires the complete
high-friction confirmation again after the new plan is reviewed.

## 5. Launch authority ownership

Launch is available only for a retained terminal real execution whose status is
`succeeded` or `succeeded_with_warnings` and whose reviewed plan plus report
establish exactly one distinct successful `launch_app` candidate. Candidate
package and optional activity values must be trusted literals. Simulation,
failure, cancellation, unknown outcomes, zero candidates, dynamic candidates,
and multiple distinct candidates are ineligible.

Tauri exclusively owns one-shot authority:

1. Tauri mints a random opaque launch-action handle for an eligible retained
   execution and sends React only that handle and a safe display label.
2. Invocation atomically removes the handle before Platform-Tools, device,
   target, report, or ADB revalidation. Concurrent use of one handle therefore
   has at most one winner.
3. Tauri revalidates the live review association, catalog, plan digest,
   Platform-Tools identity, connected exact device, stable device facts,
   authoritative terminal report, and launch eligibility.
4. Tauri calls the internal `launchExecutionApp` sidecar operation with only the
   retained sidecar execution identifier.
5. The sidecar independently rederives eligibility and invokes only the
   existing typed `launch_app` ADB adapter. It does not consume or permanently
   mutate execution-level launch authority.

The consumed opaque handle is never reusable, including after failure. If
revalidation or launch fails while the execution remains retained and eligible,
a later authoritative snapshot refresh may mint a new opaque action. A
successful launch suppresses further actions for that execution. Restart,
review invalidation, catalog/device/Platform-Tools change, mapping eviction, or
execution-session loss makes existing actions unusable.

Public launch failures use stable sanitized codes: `launch_unavailable`,
`launch_stale_target`, `device_disconnected`, `platform_tools_unavailable`, and
`launch_failed`.
