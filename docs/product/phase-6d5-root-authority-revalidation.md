# Phase 6D.5 — Root Authority Revalidation During Execution

## 1. Status and scope

**Owner:** EmuChef proper / Shared Runtime  
**Status:** Completed as an automated implementation slice on 2026-08-03  
**Phase status:** Phase 6D remains **In progress**

This slice closes the execution-time root-authority gap identified by the
Phase 6D.1 audit. It revalidates root immediately before every real-device
command whose adapter-owned shell construction will invoke `su`, preserves the
existing timeout, process, transport, and identity precedence, and fails-stop
the run when continued root authority cannot be proven. It does not add retry,
root reacquisition, prompting, replay, rollback, checkpointing, resume, a
serialized status, a protocol field, or a frontend DTO field.

The evidence is host-automated. It does not claim physical root-revocation,
identity, packaged-GUI, signing, release, or representative-device
qualification.

## 2. Authority boundaries

The private `RootAuthorityGuard` lives beside the Phase 6D.4 identity guard in
`crates/emuchef-rust-backend/src/executor/root_authority.rs`. It retains only:

1. whether the reviewed execution was authorized for root-dependent work; and
2. whether an earlier intended mutating device command reached a trustworthy
   completed result.

It does not retain credentials, UID output, raw command output, serialized
evidence, or a continuing root-authority cache. The runner configures it once
from an authoritative determination that the reviewed plan actually contains
root-dependent work and that the reviewed runtime capabilities satisfy that
work. Runtime capability availability alone does not authorize privileged
execution.

`crates/emuchef-rust-backend/src/executor/root_requirements.rs` is the one
dependency-free source of reviewed-work root semantics. Backend execution
normalizes typed plans into its classifier facts; Tauri compiles that same
source for retained JSON reviews, rather than reinterpreting parameters or
adding a public review field. It centrally defines root-related step
constraints, the application-private path boundary, and the operation rules:
`copy_files` private sources or destinations, device-side `extract_archive`
private destinations that reach `mkdir_p`, and private `path_exists` or
`file_exists` conditions require root. Equivalent shared-storage destinations
do not. Current permission commands remain nonprivileged unless a private
predicate itself requires root. Actual command-time privilege remains
adapter-owned; the executor runner does not maintain a second list of
privileged command shapes.

`ExecutorDevice::configure_root_authority` and
`ExecutorDevice::owns_per_command_root_authority` are lifecycle/configuration
hooks. The executor-visible operation surface remains the established thirteen
device operations. `RealAdbDevice` owns per-command root checks; dry-run devices
retain the existing cached fake preflight behavior and issue zero new root
probes.

## 3. Per-command ordering and classification

The sole execution-time root boundary is
`RealAdbDevice::run_shell_with_privilege_unchecked`. When its `privileged`
argument is true, the adapter performs exactly this ordering:

```text
pre-operation identity check
fresh root probe
intended privileged command
post-operation identity check, only when the result permits it
```

The raw probe is the fixed-serial command:

```text
adb -s <reviewed-serial> shell su -c id
```

It uses `ProcessOperation::RootPreflight`, the existing operation deadline, and
the existing bounded process output seam directly. It bypasses public identity
and root guards and cannot recurse.

Completed probe results classify privately as follows:

| Probe evidence | Private result | Stable issue code |
|---|---|---|
| `uid=0` | proceed | — |
| denied or unavailable/`su` missing | `RootAuthorityRevoked` | `root_authority_revoked` |
| completed unexpected response | `RootAuthorityUnverified` | `root_authority_unverified` |

Timeout, executable resolution or spawn failure, process/output ownership
failure, offline, unauthorized, disconnected, ADB-server failure, transport
reset, and transport failure retain their existing typed classifications and
precedence. They do not trigger a root-failure identity recheck.

For a completed denied, unavailable, or unexpected root response, the adapter
performs exactly one `IdentityCheckPhase::PreOperation` recheck. If that check
fails, its identity result wins; because the intended command never ran, no
post-operation identity marker is emitted. If the recheck succeeds, the root
classification is returned and the intended privileged command count remains
zero.

Current permission actions remain nonprivileged and receive zero Phase 6D.5
root probes. They continue to use their existing `adb shell pm grant` and
`adb shell appops set` commands and are treated as mutating intended commands
for prior-mutation accounting. Any future root-wrapped permission operation
must use the adapter-owned privileged-shell seam and will then receive the same
per-command root verification automatically.

## 4. Mutation evidence and partial changes

Every guarded intended operation explicitly supplies the private
`DeviceCommandEffect` classification. Predicates are `ReadOnly`; device
changes, launches, force-stops, and current permission commands are
`Mutating`. Mutation evidence is recorded before the existing post-operation
identity check.

Evidence is trustworthy only after a mutating intended command returns success
or `AdbCommandError::CommandFailed`, which means the process completed with an
ordinary nonzero result. Validation failure before process start, identity
failure, root failure, resolution/spawn failure, timeout, process/output
failure, and transport failure never establish prior-mutation evidence.

The exact private marker is:

```rust
pub(crate) const ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER: &str =
    "Root authority could not be confirmed after earlier device changes may have occurred.";
```

It is used only for `root_authority_revoked` or
`root_authority_unverified` after trustworthy earlier mutation evidence. Tauri
consumes the marker before public projection and enables
`partialChangesPossible` only for that exact code/marker pairing. Identity
markers, arbitrary messages, and unrecognized issue codes cannot activate the
root partial-change signal. Existing failed-atomic-operation reporting remains
independent.

## 5. Typed propagation and fail-stop behavior

The private root variants propagate through:

```text
AdbCommandError
  -> DeviceOperationKind
  -> StepFailureKind
  -> execution_session issue_code
  -> root_authority_revoked / root_authority_unverified
```

Both variants use the existing device fail-stop predicate. The active step
fails before its intended privileged command, prior outputs and evidence are
preserved, later parameter resolution/device work/host mutations/skip
predicates/verification/permission actions do not run, and later initialized
work remains pending for terminal **Not attempted** projection. The existing
terminal path releases the active execution slot. Dry-run scheduling behavior
remains unchanged.

## 6. Tauri authority invalidation

On the first terminal retention of a real execution, `mark_terminal` gates
authority invalidation. If the report contains any identity issue, existing
identity invalidation runs alone. Otherwise, a root issue invokes root-only
invalidation:

- completed root qualification evidence for the affected device is removed;
- matching in-flight root qualification is cancelled and fenced against late
  completion;
- the originating review and other root-dependent reviews for that device are
  tombstoned;
- non-root reviews for that device remain valid;
- the live device record, cached facts, qualification context, session epoch,
  device generation, serial-to-handle mapping, unrelated authority, and the
  terminal execution mapping/report remain unchanged.

Repeated terminal retrieval and export are pure reads and do not invalidate a
second time or change generations. A combined identity-plus-root report takes
the broader identity path only.

## 7. User-visible projection

Tauri projects the two root classifications with distinct authored copy:

- `root_authority_revoked`: **Root access was revoked during execution.**
- `root_authority_unverified`: **EmuChef could not safely confirm continued
  root access.**

Both remediations require fresh root qualification, fresh plan generation,
fresh review, and a new execution, and state that the old execution cannot
resume. Snapshots, events, React, diagnostics, and export documents do not
expose `su`, `uid=0`, serials, raw root output, command arguments, process
output, private markers, or arbitrary backend messages. Existing generic
terminal pending rendering is sufficient; `ExecutionStep.tsx` and public DTO
shapes remain unchanged.

## 8. Evidence

The focused TDD sequence recorded a red result for the missing classifier and
private guard symbols, followed by green focused tests for raw classification,
mutation evidence, reviewed-work authorization, per-command ordering, identity
recheck precedence, exact marker pairing, typed propagation, fail-stop/pending
behavior, root-only invalidation, combined identity precedence, sanitization,
and frontend rendering. The run-specific `RESULT.md` records the exact
commands, command counts, changed files, and final verification matrix.
