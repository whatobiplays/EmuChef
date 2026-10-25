# ADR 0010: Per-file configuration recovery without whole-setup rollback

**Status:** Accepted product direction; retention and interrupted-write details open.

Configuration backups are enabled by default. Before changing an existing app configuration, EmuChef normally preserves its original bytes as a protected, bounded recovery artifact and supports explicit per-file restoration. If backup is enabled but fails for an affected file, that file's update requires a distinct, informed waiver for that execution, stating that restoration may not be available.

A user may instead disable automatic configuration backups persistently in Advanced Settings. Enabling that opt-out requires explicit confirmation of its consequences; during review of affected changes, EmuChef clearly states that configuration backups are disabled. While the preference is active, EmuChef does not attempt automatic backups or repeatedly demand per-file waivers. Re-enabling backups restores the default behavior for subsequent operations. Neither an isolated waiver nor a persistent opt-out bypasses safe parsing, preservation of unrelated settings, operation review, or any other execution safety condition.

A configuration backup waiver is not a promise of recovery; no whole-device or whole-setup rollback is implied. Reports preserve each operation's actual completed, failed, skipped, or partial state. Exact backup retention limits, restoration target validation, interruption recovery, and settings persistence details remain implementation design questions.

## Open engineering contract (not implemented)

[EmuChef product realignment proposal](../product/emuchef-product-realignment-proposal.md) assigns T10 backup validity, protected/limited retention, restart discovery, target-aware restoration, backup failure consent and global preference persistence; T09/T13b specify revalidation, remote write/publication evidence and interrupted/possibly-changed states. Restoration requires a fresh inspected target and explicit reviewed action. The current executor still has no backup/restoration contract; per-file restore must not be described as available until implemented and qualified.

**Effective-mode review binding:** T10 must capture the effective backup preference (including the global opt-out) within the immutable, digest-bound reviewed execution. A preference change before an unstarted attempt requires fresh review; an already running attempt retains its reviewed backup mode, and later attempts use the current persistent setting. Global opt-out continues to mean no automatic backup attempts or repeated per-file waivers for an execution prepared with that setting. Restoration must freshly validate the supported app-state/quiescence precondition as well as target state; required stop/preparation actions are separately reviewed, and a stop that changes the configuration invalidates the earlier file snapshot. This is an unimplemented contract, not an available recovery capability.

## Execution admission, frozen backup mode and recovery-original retention (T10/T19)

**Accepted T01 clarification; not implemented:** A reviewed execution/stage and
its OEM Privileged Setup Bundle have one immutable, digest-bound effective
backup mode, content and target state. Each distinct manual **Privileged
Execution Attempt** of that bundle inherits its reviewed backup mode and
bytes. Changing a persistent preference while staging is awaiting a user's
invocation invalidates the *unstarted* review; only a **new** reviewed bundle
can use the changed preference. If an invocation may have started but its
start evidence is delayed, treat execution admission as **uncertain** and
reconcile it before assigning mode or applying different contents. Once
started, the effective mode remains fixed despite settings changes.

When automatic backup is **enabled**, an existing-file write must capture
the exact original in a qualified recovery store before overwrite. If that
backup fails, the privileged script **stops before the overwrite** and records
an explicit per-file failure. A user-accepted per-file/per-execution waiver
authorizes only a **newly prepared and reviewed continuation**, not an
implicit branch in the running script. Declining blocks the modification.
Under a *reviewed global opt-out*, no automatic backup attempt or repeated
per-file waiver occurs. A successful ZIP report never proves original bytes
are recoverable.

A **Recovery Original** and metadata needed for restore are retained
separately from temporary staging/results. Do not delete the last device copy
of a required original until integrity, identity and restoration metadata are
confirmed in the qualified recovery store. Cleanup eligibility follows
recovery retention; a failure to clean eligible temporary device artifacts
becomes the separate **Q81 C warning-only cleanup obligation**, not a setup
failure. Actual transport/identity/session-fatal failures still fail closed.
