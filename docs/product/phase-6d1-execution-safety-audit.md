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
| Cancellation at safe boundaries | Fully implemented | Cancellation is cooperative. The current atomic operation may finish, completed work remains recorded, no rollback is claimed, and no later step is scheduled after cancellation is observed. Never-started steps retain the existing `pending` serialized status; once the run is terminal, Tauri/React interpret that state as **Not attempted** rather than inventing a new protocol status. | `ExecutorRunner::run_with_progress_and_cancel` in `crates/emuchef-rust-backend/src/executor.rs`; `ExecutionSessionManager::cancel` and `session_cancellation_leaves_never_started_work_pending_for_terminal_projection` in `execution_session.rs`; `ExecutionStep.dom.test.tsx`. | Physical cancellation timing remains a Phase 6D qualification item. No checkpoint, interrupt-in-flight, or rollback work is approved. |
| Completed, cancelled, pending, and never-started work | Fully implemented | While active, `pending` means work may still execute. In a terminal snapshot, remaining `pending` work is never-started work and is labeled/counts as **Not attempted** in the UI. The overall run retains `cancelled`; completed, skipped, blocked, failed, and explicitly cancelled step states remain distinct. | `refresh_recipe_statuses` and `recipe_status_keeps_partially_processed_work_active_or_cancelled` in `execution_session.rs`; `completion_summary` and `terminal_pending_work_remains_derivable_without_a_new_completion_field` in Tauri `execution.rs`; `ExecutionStep.tsx` and its DOM regression. | Export schema 1 retains the existing `pending` field. Consumers must interpret terminal `pending` as never started. A new serialized status is unnecessary and therefore not introduced. |
| ADB disconnect during execution | Partially implemented | Every real command remains bound to the reviewed serial. When ADB returns a command failure, the current step becomes failed and the run reaches a stable failed result after remaining dependency processing. Tauri replaces raw executor messages with allowlisted guidance. | `AdbCommandRunner::run`, `RealAdbDevice`, `finish_attempt`, `issue_code`, and `project_real_issue`; real-projection sanitization tests in Tauri `execution.rs`. | There is no typed transport-loss signal that aborts all remaining unrelated work, and ordinary ADB operations have no deadline. Defer to a bounded transport/error model and physical qualification. |
| Offline or unauthorized device during execution | Partially implemented | Startup inventory and qualification distinguish offline and unauthorized devices and block execution before start. If state changes after start, a returned ADB failure becomes a failed operation and sanitized terminal report. | Device inventory/qualification code in `device_qualification.rs`; `start_real_execution_inner_with_runtime`; integrated real-preflight tests; executor failure projection. | Mid-run state changes are not reclassified as dedicated offline/unauthorized outcomes. Typed transport classification and physical evidence are deferred. |
| Device replacement during execution | Partially implemented | Real ADB commands use the exact retained serial, so a different serial cannot receive the reviewed commands. Start immediately re-reconciles inventory, probes facts, validates the target, plan digest, inputs, qualification, root evidence, runtime generation, and Platform-Tools revision. | `start_real_execution_inner_with_runtime`, `validate_target`, `validate_final_qualification`, and integrated real-preflight tests; `AdbCommandRunner` serial injection. | A different physical device reusing the same serial after start cannot be reliably detected with the current command contract. Mid-run identity revalidation is deferred architecture work. |
| Operation timeouts | Fully implemented (automated) | Every backend one-shot ADB/process operation selects a fixed internal deadline: 30 seconds for probes, predicates, root preflight, launch, and force-stop; 120 seconds for shell mutations; and 300 seconds for install, push, device copy, and generic fallback. Each stream is retained up to 4 MiB with overflow distinct from timeout. Timeout is typed through the executor, stops later scheduling, preserves completed evidence, leaves later work pending/Not attempted, and keeps possible partial changes true. The EmuChef proper sidecar has a 300-second request deadline and bounded 16 MiB JSONL framing. | `owned_process.rs`, `ProcessOperation`, `DeviceOperationError`, `ExecutorRunner::run_with_progress_and_cancel`, `issue_code`, sidecar framing tests, and `docs/product/phase-6d2-operation-deadlines.md`. | Physical interruption, host sleep, low-storage, and representative device qualification remain Phase 6D follow-ups. |
| Sidecar crash or protocol loss | Fully implemented (automated) | Broken request I/O, timeout, EOF, partial/oversized frames, malformed JSON, response-ID mismatch, and structurally invalid success or error envelopes mark the runtime generation failed and stop the unusable child. Every later request in that generation returns the stable `runtime_session_lost` code. Tauri translates that proof of global loss to `execution_unavailable` and invalidates every execution/launch mapping plus runtime-derived review, device-fact, and root-qualification authority. An `unknown_execution` backend error remains mapping-local. The UI states that a lost real-device outcome may be partial and cannot be resumed or rolled back. | `SidecarClient::request`, bounded `read_frame`, protocol-loss and framing tests in `sidecar.rs`; `execution_session_loss`, `invalidate_lost_runtime_authority`, and `lost_runtime_session_resets_all_execution_mappings` in Tauri `execution.rs`; `docs/product/phase-6d2-operation-deadlines.md`. | Physical disconnect and packaged-GUI interruption qualification remain open. |
| Sidecar restart | Fully implemented | Explicit restart is blocked while an execution is starting or active. A proven-lost session first releases stale execution authority; restart increments runtime generation and resets reviews, execution mappings, qualification evidence, and presentation authority. No execution survives or resumes across restart. | `restart_runtime` and `reset_app_session` in `commands.rs`; runtime-generation workflow tests; `ExecutionHandleStore::reset`. | Persistent execution and resume are intentionally unsupported. |
| Root authority loss | Partially implemented | Root is revalidated lazily before the first privileged step and cached for that run. A failed preflight aborts later work before privileged operations. If root is lost after the cached preflight, the next privileged ADB command fails and is sanitized as an execution failure. | `ExecutorRunner::ensure_root_preflight`, `RealAdbDevice::check_root`, root executor tests, and Phase 6C.2 qualification evidence. | Root is not re-probed at every privileged safe boundary, and mid-run revocation lacks a dedicated classification. Deferred pending an explicit revalidation and abort policy. |
| Worker panic | Fully implemented | A panic is caught, the execution becomes terminal failed, a sanitized `execution_worker_panicked` issue/event is retained, pending work remains derivable as not attempted, and the active slot is released. | `catch_unwind`, `finish_panicked`, and `worker_panic_leaves_terminal_report_and_releases_active_slot` in `execution_session.rs`; Tauri issue/event allowlists. | A process-level crash is handled as sidecar session loss rather than as an in-process worker panic. |
| Partial-result reporting | Fully implemented | Terminal summaries retain completed, skipped, blocked, failed, cancelled, and never-started work. Failed or cancelled real runs warn about possible partial device changes when completed work or a failed atomic operation exists. Neither the UI nor report implies partial success, restoration, or rollback. | `completion_summary`, `failed_completion_keeps_failure_primary_and_reports_partial_changes`, and `failed_atomic_work_warns_about_possible_partial_changes` in Tauri `execution.rs`; `ExecutionStep.tsx`. | When the sidecar session itself is lost, exact step results may be unavailable; the UI correctly presents the whole outcome as indeterminate. |
| Retry and repair eligibility | Fully implemented | Failed, cancelled, unavailable, stale, and repair flows preserve only reusable portable intent. Another execution requires current device facts, qualification, inputs, a newly generated plan/digest, fresh review, and a new random execution handle. There is no in-place retry or replay. | `prepare-repair` and `return-to-review` transitions in `workflow.ts`; `start_real_execution_inner_with_runtime`; review lifecycle tests; `ExecutionStep.dom.test.tsx`. | Automatic replay, checkpointing, and resume are intentionally unsupported. |
| Stale review, qualification, device, runtime, and execution authority | Fully implemented | Real start revalidates all retained authority immediately before the single start request. Generation/handle guards reject stale frontend responses. A globally lost runtime session clears every execution/launch mapping and invalidates its runtime-derived reviews, device facts, and root qualification; a backend `unknown_execution` error invalidates only its mapping and originating real review. Execution handles are random, session-scoped, kind-aware, and never reused. | Integrated real-preflight tests, `ReadinessGenerations::matches`, qualification context keys, `ExecutionHandleStore`, `invalidate_lost_runtime_authority`, `useExecution.ts`, and workflow stale-response tests. | Same-serial physical replacement after start remains the device-identity gap described above. |
| Event and snapshot sanitization | Fully implemented | React receives opaque handles, authored feature/action text, allowlisted statuses and issue guidance, localized timestamps, and serial/path-redacted content. Raw sidecar IDs, reviewed plans, target bindings, outputs, ADB output, and arbitrary backend messages are excluded. | `project_real_snapshot`, `project_real_event_batch`, `project_real_issue`, `sanitize_real_projection`, and projection/security tests in Tauri `execution.rs`. | Continue extending allowlists when an independently approved executor classification is added. |
| Exported report sanitization | Fully implemented | Schema-1 reports contain sanitized runtime/catalog identity, plan identity, terminal state, completion summary, projected recipes/issues, and bounded target presentation. Private authority and raw executor data are excluded. | `execution_report_document`, `report_document_is_deterministic_and_excludes_private_authority`, and real-projection sanitization tests. | Terminal `pending` remains the existing serialized representation of not-attempted work; no schema/API expansion was approved for 6D.1. |
| Frontend state and copy accuracy | Fully implemented | Active work says Waiting; the same remaining state says Not attempted only after terminal status. Not-attempted work is counted separately and does not inflate completed progress. Proven completed work and uncertain failed atomic work use different partial-change wording. Cancellation explains safe-boundary delay and no rollback. Unavailable real execution is explicitly indeterminate and requires a fresh workflow. | `ExecutionStep.tsx`, `useExecution.ts`, `workflow.ts`, `ExecutionStep.dom.test.tsx`, `useExecution.dom.test.tsx`, and `workflow.test.ts`. | Broad visual redesign is out of scope. |
| Low storage and host sleep | Missing | No dedicated low-storage preflight, host-sleep transition, or stable classification exists. Symptoms currently collapse into an operation failure or an unbounded wait. | No authoritative implementation or qualification evidence found in the audited executor/Tauri surfaces. | Defer to the timeout/transport design and physical-device matrix. Do not claim support in Phase 6D.1. |

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

No fix adds a new serialized execution status, public command, API field,
checkpoint, resume token, replay path, rollback behavior, or persistent active
execution.

## 4. Deferred architectural and qualification backlog

1. Qualify the bounded timeout and child-process ownership model against
   physical interruption, disconnect, host sleep, low storage, and representative
   supported devices.
2. Introduce a trusted typed transport classification before deciding whether
   disconnect, offline, or unauthorized failures abort unrelated remaining work.
3. Design mid-run safe-boundary revalidation for target identity, same-serial
   replacement, qualification context, and root authority.
4. Define low-storage and host-sleep behavior, including whether the outcome is
   terminal failed or explicitly indeterminate.
5. Run the physical interruption matrix for cancellation timing, disconnect,
   offline, unauthorized, root revocation, host sleep, and timeout behavior on
   representative supported devices.

These items keep Phase 6D open. They must not be implemented as automatic
replay, rollback, checkpointing, or resume.

## 5. Verification evidence

The final run result records the exact default, `real-execution`, formatting,
typecheck, lint, build, and diff commands. Phase 6D.2 uses deterministic host
tests and does not claim new physical-device, packaged-GUI, or release
qualification.
