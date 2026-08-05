# Phase 6D.1 Execution Safety Audit

## 1. Scope and authority

**Owner:** EmuChef proper / Shared Runtime  
**Date:** 2026-07-31  
**Boundary:** Audit plus evidence-backed, low-risk correctness fixes

This audit covers cancellation, device and runtime interruption, stale authority,
partial results, retry and repair, sanitization, and user-facing recovery. Rust
and trusted Tauri retain execution, device, process, filesystem, review, and
recovery authority. React renders sanitized projections only.

Phase 6D.1 does not add checkpointing, resume, rollback, automatic replay,
persistent active execution, a new execution status, or a new public API. Real
execution remains compile-time gated and disabled in ordinary production builds.

The classifications used below are `Fully implemented`, `Partially implemented`,
`Missing`, and `Intentionally unsupported`. `Fully implemented` means the current
contract has deterministic automated evidence; it does not claim physical-device,
packaged-GUI, signing, notarization, or release qualification.

## 2. Safety matrix

| Area | Classification | Current behavior and user-visible result | Source and test evidence | Gap and disposition |
|---|---|---|---|---|
| Cancellation at safe boundaries | Fully implemented | Cancellation is cooperative. The current atomic operation may finish, completed work remains recorded, no rollback is claimed, and no later step is scheduled after cancellation is observed. Never-started steps retain the existing `pending` serialized status; once the run is terminal, Tauri/React interpret that state as **Not attempted** rather than inventing a new protocol status. The ignored Phase 6D.6 harness additionally observes an in-flight production lifecycle callback for its active-cancellation qualification without changing the public API. | `ExecutorRunner::run_with_progress_and_cancel` in `crates/emuchef-rust-backend/src/executor.rs`; `ExecutionSessionManager::cancel` and `session_cancellation_leaves_never_started_work_pending_for_terminal_projection` in `execution_session.rs`; `ExecutionStep.dom.test.tsx`; the private observed-runner test and Phase 6D.6 evidence contract. | Physical cancellation timing remains a Phase 6D qualification item. No checkpoint, interrupt-in-flight production feature, or rollback work is approved. |
| Completed, cancelled, pending, and never-started work | Fully implemented | While active, `pending` means work may still execute. In a terminal snapshot, remaining `pending` work is never-started work and is labeled/counts as **Not attempted** in the UI. The overall run retains `cancelled`; completed, skipped, blocked, failed, and explicitly cancelled step states remain distinct. | `refresh_recipe_statuses` and `recipe_status_keeps_partially_processed_work_active_or_cancelled` in `execution_session.rs`; `completion_summary` and `terminal_pending_work_remains_derivable_without_a_new_completion_field` in Tauri `execution.rs`; `ExecutionStep.tsx` and its DOM regression. | Export schema 1 retains the existing `pending` field. Consumers must interpret terminal `pending` as never started. A new serialized status is unnecessary and therefore not introduced. |
| ADB disconnect during execution | Fully implemented (automated) | Every real command remains bound to the reviewed serial. Stable completed ADB transport responses are classified at the private adapter boundary, including unchecked predicates and root probing. A typed transport failure fails the active step, preserves prior evidence and active-operation outputs, stops all later work, leaves later steps pending/Not attempted, reports possible partial changes, and releases the active slot. Tauri replaces raw executor messages with allowlisted guidance. | `AdbCommandRunner`, `probe_root_typed`, `DeviceOperationKind`, `StepFailureKind::requires_device_fail_stop`, `finish_attempt`, `issue_code`, `project_real_issue`, and deterministic executor/session/Tauri tests; `docs/product/phase-6d3-adb-transport-failures.md`. | Physical disconnect timing, identity evidence qualification, and representative-device qualification remain open. |
| Offline or unauthorized device during execution | Fully implemented (automated) | Startup inventory and qualification distinguish offline and unauthorized devices and block execution before start. Stable mid-run ADB responses become typed `device_offline` or `device_unauthorized` failures, use the same fail-stop path, and project reconnect/authorization guidance that requires fresh qualification, plan, and review. Pre-operation identity verification remains earlier than the intended mutation and may fail closed as `device_identity_unverified` while the device is independently observed as unauthorized. | Device inventory/qualification code; `IdentityGuard`; `AdbCommandRunner` classifier; executor/session propagation; Tauri and frontend projection tests. | Physical unauthorized qualification remains mandatory Phase 6D.6 evidence and is exercised at a safe boundary after a proven same-serial absence and unauthorized reconnect. Either `device_unauthorized` or identity-first `device_identity_unverified` may be the terminal branch, but the latter qualifies only with exact independent authorization chronology. Earlier blocked attempts are retained as non-passing audit evidence. Physical offline evidence is conditional because ADB exposes no general stable operator-controlled offline transition; deterministic automated coverage remains closure evidence for that classification. |
| Device replacement during execution | Fully implemented (automated) | Real ADB commands use the exact retained serial, and each safe-boundary operation can privately compare complete, stable same-serial identity evidence against the reviewed target. A pre-operation mismatch blocks mutation; a completed operation is followed by a bounded post-operation check that may classify changed or unverified identity without hiding an original timeout, spawn, transport, or process failure. | `IdentityGuard`, `AdbCommandRunner`, `ExecutorRunner`, `DeviceOperationKind`, and deterministic executor/session tests; `start_real_execution_inner_with_runtime`, `validate_target`, and Tauri authority-invalidation tests. | Physical identity qualification, attestation, root revocation, and recovery remain open Phase 6D work. The private fingerprint is not persistent identity and does not claim hardware attestation. |
| Operation timeouts | Fully implemented (automated) | Every backend one-shot ADB/process operation selects a fixed internal deadline: 30 seconds for probes, predicates, root preflight, launch, and force-stop; 120 seconds for shell mutations; and 300 seconds for install, push, device copy, and generic fallback. Each stream is retained up to 4 MiB with overflow distinct from timeout. Timeout and typed transport failures use the private fail-stop path, preserving evidence, leaving later work pending/Not attempted, and keeping possible partial changes true. The EmuChef proper sidecar has a 300-second request deadline and bounded 16 MiB JSONL framing. | `owned_process.rs`, `ProcessOperation`, `DeviceOperationError`, `StepFailureKind::requires_device_fail_stop`, `ExecutorRunner::run_with_progress_and_cancel`, `issue_code`, sidecar framing tests, and Phase 6D.2/6D.3 documentation. | Physical interruption, host sleep, low-storage, and representative device qualification remain Phase 6D follow-ups. |
| Sidecar crash or protocol loss | Fully implemented (automated) | Broken request I/O, timeout, EOF, partial/oversized frames, malformed JSON, response-ID mismatch, and structurally invalid success or error envelopes mark the runtime generation failed and stop the unusable child. Every later request in that generation returns the stable `runtime_session_lost` code. Tauri translates that proof of global loss to `execution_unavailable` and invalidates every execution/launch mapping plus runtime-derived review, device-fact, and root-qualification authority. An `unknown_execution` backend error remains mapping-local. The UI states that a lost real-device outcome may be partial and cannot be resumed or rolled back. | `SidecarClient::request`, bounded `read_frame`, protocol-loss and framing tests in `sidecar.rs`; `execution_session_loss`, `invalidate_lost_runtime_authority`, and `lost_runtime_session_resets_all_execution_mappings` in Tauri `execution.rs`; `docs/product/phase-6d2-operation-deadlines.md`. | Physical disconnect and packaged-GUI interruption qualification remain open. |
| Sidecar restart | Fully implemented | Explicit restart is blocked while an execution is starting or active. A proven-lost session first releases stale execution authority; restart increments runtime generation and resets reviews, execution mappings, qualification evidence, and presentation authority. No execution survives or resumes across restart. | `restart_runtime` and `reset_app_session` in `commands.rs`; runtime-generation workflow tests; `ExecutionHandleStore::reset`. | Persistent execution and resume are intentionally unsupported. |
| Root authority loss | Fully implemented (automated) | Each real atomic command that actually inserts `su` receives one fresh bounded `adb -s <serial> shell su -c id` probe immediately before the intended command. Denied, unavailable, and unexpected completed evidence becomes a dedicated fail-stop root classification; timeout, spawn, process/output, and transport failures retain their existing precedence. Current permission actions remain nonprivileged and receive zero root probes while still counting as mutating intended commands for prior-mutation accounting. Root-only terminal invalidation removes affected root evidence and root-dependent reviews while preserving the live device and non-root authority. | `RealAdbDevice::run_shell_with_privilege_unchecked`, private `RootAuthorityGuard`, `StepFailureKind`, `execution_session::issue_code`, Tauri root-only invalidation, and deterministic backend/Tauri/frontend tests; `docs/product/phase-6d5-root-authority-revalidation.md`. | Physical root-revocation timing and representative-device qualification remain open Phase 6D work. |
| Worker panic | Fully implemented | A panic is caught, the execution becomes terminal failed, a sanitized `execution_worker_panicked` issue/event is retained, pending work remains derivable as not attempted, and the active slot is released. | `catch_unwind`, `finish_panicked`, and `worker_panic_leaves_terminal_report_and_releases_active_slot` in `execution_session.rs`; Tauri issue/event allowlists. | A process-level crash is handled as sidecar session loss rather than as an in-process worker panic. |
| Partial-result reporting | Fully implemented | Terminal summaries retain completed, skipped, blocked, failed, cancelled, and never-started work. Failed or cancelled real runs warn about possible partial device changes when completed work or a failed atomic operation exists; the one fixed post-operation identity marker also enables that warning when identity is the only retained evidence. Neither the UI nor report implies partial success, restoration, or rollback. | `completion_summary_with_identity_state`, `failed_completion_keeps_failure_primary_and_reports_partial_changes`, `only_the_exact_post_identity_marker_allows_real_partial_warning_without_prior_evidence`, and `ExecutionStep.tsx`. | When the sidecar session itself is lost, exact step results may be unavailable; the UI correctly presents the whole outcome as indeterminate. |
| Retry and repair eligibility | Fully implemented | Failed, cancelled, unavailable, stale, and repair flows preserve only reusable portable intent. Another execution requires current device facts, qualification, inputs, a newly generated plan/digest, fresh review, and a new random execution handle. There is no in-place retry or replay. | `prepare-repair` and `return-to-review` transitions in `workflow.ts`; `start_real_execution_inner_with_runtime`; review lifecycle tests; `ExecutionStep.dom.test.tsx`. | Automatic replay, checkpointing, and resume are intentionally unsupported. |
| Stale review, qualification, device, runtime, and execution authority | Fully implemented | Real start revalidates all retained authority immediately before the single start request. A terminal identity issue invalidates only the affected live device facts, qualification context, matching reviews, session epoch/generation, and matching root authority while preserving the existing serial-to-opaque-handle reconciliation. Generation/handle guards reject stale frontend responses. A globally lost runtime session still clears every execution/launch mapping and invalidates its runtime-derived reviews, device facts, and root qualification; a backend `unknown_execution` error remains mapping-local. | Integrated real-preflight tests, `SessionHandles::invalidate_identity_authority`, `RootQualificationStore::invalidate_for_device`, `ExecutionHandleStore`, `invalidate_identity_terminal_authority`, `invalidate_lost_runtime_authority`, `useExecution.ts`, and workflow stale-response tests. | Repeated root-authority revalidation, physical identity qualification, and recovery remain deferred. |
| Event and snapshot sanitization | Fully implemented | React receives opaque handles, authored feature/action text, allowlisted statuses and issue guidance, localized timestamps, and serial/path-redacted content. Raw sidecar IDs, reviewed plans, target bindings, outputs, ADB output, and arbitrary backend messages are excluded. | `project_real_snapshot`, `project_real_event_batch`, `project_real_issue`, `sanitize_real_projection`, and projection/security tests in Tauri `execution.rs`. | Continue extending allowlists when an independently approved executor classification is added. |
| Exported report sanitization | Fully implemented | Schema-1 reports contain sanitized runtime/catalog identity, plan identity, terminal state, completion summary, projected recipes/issues, and bounded target presentation. Private authority and raw executor data are excluded. | `execution_report_document`, `report_document_is_deterministic_and_excludes_private_authority`, and real-projection sanitization tests. | Terminal `pending` remains the existing serialized representation of not-attempted work; no schema/API expansion was approved for 6D.1. |
| Frontend state and copy accuracy | Fully implemented | Active work says Waiting; the same remaining state says Not attempted only after terminal status. Not-attempted work is counted separately and does not inflate completed progress. Proven completed work and uncertain failed atomic work use different partial-change wording. Cancellation explains safe-boundary delay and no rollback. Unavailable real execution is explicitly indeterminate and requires a fresh workflow. | `ExecutionStep.tsx`, `useExecution.ts`, `workflow.ts`, `ExecutionStep.dom.test.tsx`, `useExecution.dom.test.tsx`, and `workflow.test.ts`. | Broad visual redesign is out of scope. |
| Low storage and host sleep | Partially implemented (automated policy; physical evidence pending) | Completed ADB/device operations classify bounded, line-anchored ENOSPC evidence as `device_storage_exhausted` without speculative free-space preflight. Storage failure uses the existing typed fail-stop path, preserves evidence and conservative partial-change reporting, projects authored storage guidance, and requires a fresh workflow. Host sleep keeps the existing locally owned child/future tree and fixed timer semantics; a surviving process may complete, time out, or report transport loss, while sidecar generation loss remains `runtime_session_lost` and cannot resume. The Phase 6D.6 contract records sleep request/entry/wake, wall and executor elapsed time, deadline, timer implementation/toolchain, and an included/excluded/indeterminate/contradictory classification; transport loss cannot qualify the timer branch. | `AdbCommandRunner::classify_completed_storage`, `DeviceStorageExhausted`, executor/session/Tauri tests, owned-process elapsed-jump and bounded-delay tests, sidecar runtime-loss test, the gated Phase 6D.6 harness, `docs/testing/phase-6d6/evidence-schema.json`, and `docs/manual/phase-6d6-physical-interruption-qualification.md`. | No physical low-storage or host-sleep attempt is claimed in this run. The safety reserve, operator checkpoint, measured timer behavior, cleanup, UI-smoke pair, and two clean repetitions remain mandatory before closure; exact clippy gates are also currently blocked by baseline findings. |

## 3. Bounded fixes completed

1. Cancellation no longer emits terminal step-progress events that misclassify
   unscheduled work as cancelled. Existing report `pending` state is preserved
   and interpreted as Not attempted only when the execution is terminal.
2. Recipe aggregation no longer reports a partially processed active recipe as
   succeeded; it remains running and becomes cancelled when the run is cancelled.
3. Failed real atomic work now conservatively enables the possible-partial-change
   warning even when no earlier step completed successfully.
4. Fatal sidecar transport/protocol loss now persists for the failed runtime
   generation and clears all execution, launch, review, device-fact, and root
   qualification authority derived from that process. Mapping-local
   `unknown_execution` handling remains separate.
5. The result UI now shows cancelled and waiting/not-attempted counts, labels
   terminal pending steps as Not attempted, and keeps never-started work out of
   completed progress accounting.
6. Phase 6D.2 adds locally owned process and sidecar deadlines, typed timeout
   propagation, fail-stop scheduling after a real timeout, bounded output/frame
   capture, and sanitization regressions without changing public schemas.
7. Phase 6D.3 adds bounded, line-anchored ADB transport classification across
   checked, unchecked, and root-probe paths. Typed offline, unauthorized,
   disconnected, ADB-server, and transport-loss failures fail-stop real runs,
   preserve evidence, keep terminal pending work as **Not attempted**, warn
   about possible partial changes, release the active slot, and project only
   sanitized guidance. Root revocation and physical qualification remain
   deferred; same-serial replacement is covered by item 8.
8. Phase 6D.4 adds private, bounded same-serial identity evidence at the existing
   executor safe boundaries. Complete stable samples cover the reviewed
   manufacturer/model/API target, required build/device properties, ABI list,
   build fingerprint, and normalized Android ID. Identity mismatch or safe
   unavailability uses the existing fail-stop path, with one exact
   post-operation marker enabling the conservative partial-change warning.
   Terminal identity findings invalidate only the affected Tauri authority and
   fence late root-qualification completions while preserving serial mapping.
9. Phase 6D.6 adds the private completed-result `device_storage_exhausted`
   classification, authored recovery projection, deterministic timeout/process
   seams, and one ignored physical qualification harness. The harness selects
   exactly one scenario and repetition, binds one exact serial, reuses the
   Phase 6C fixture roots, requires explicit destructive/root/authorization/
   host-sleep opt-ins, and uses a ten-minute sentinel checkpoint. Its shared
   manifest requires measured host-sleep branches, in-flight cancellation,
   production execution-session slot evidence, exact target-child liveness,
   measured production deadline-clock behavior, bounded same-serial and
   authorization chronology, globally unique canonical evidence identities,
   and two artifact-bound composite development-build UI-smoke records. Exact
   target-child observation is implemented through the production-owned
   host-`Push` lifecycle seam; host-sleep remains fail-closed until the
   production deadline clock can be measured. The dependency-free evidence
   validator accepts an incomplete matrix for CI but
   reports Phase 6D incomplete until every mandatory physical and UI repetition
   has a passing sanitized record. The manifest retains thirteen supported
   scenarios but requires two repetitions for only twelve; `device_offline` is
   conditional diagnostic evidence and USB disconnect supplies mandatory
   transport proof.

No fix adds a new serialized execution status, public command, API field,
checkpoint, resume token, replay path, rollback behavior, or persistent active
execution.

## 4. Deferred architectural and qualification backlog

1. Run the gated Phase 6D.6 mandatory matrix twice from clean state for
   cancellation, disconnect, unauthorized, identity stability/replacement,
   root revocation, timeout, low storage, and host sleep. Run `device_offline`
   only when a reviewed device-specific transition can prove the genuine state;
   retain any blocked or attempted record without treating it as mandatory
   physical proof.
2. Qualify the same-serial identity evidence on representative physical devices
   and decide whether any future attestation or persistent identity work is
   separately approved.
3. Qualify repeated root-authority revalidation and its abort policy on representative physical devices.
4. Measure whether the fixed Rust timer observes active-host or suspended time
   on each supported qualification host and retain that limitation in evidence.
5. Complete the development-build Tauri/React recovery smoke without claiming
   packaged-GUI or release readiness.

These items keep Phase 6D open. They must not be implemented as automatic
replay, rollback, checkpointing, or resume.

## 5. Verification evidence

The final run result records the exact default, `real-execution`, formatting,
typecheck, lint, build, and diff commands. Phase 6D.2, Phase 6D.3, Phase 6D.4,
and Phase 6D.5 use deterministic host tests and do not claim new physical-device,
packaged-GUI, attestation, recovery, or release qualification.
