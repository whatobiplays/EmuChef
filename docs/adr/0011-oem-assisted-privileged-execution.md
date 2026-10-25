# ADR 0011: OEM-assisted privileged execution without ADB root

**Status:** Accepted owner direction Q47–Q83 and UX-1 A; specific engineering/physical evidence contracts pending. Not implemented, qualified or released.

## Decision and authority

EmuChef supports manufacturer-provided **OEM Root Script Runners** as an **alternative execution strategy for the same typed Recipes** when direct ADB root is unavailable at planning time. Device Profiles own the distinct runner capability, instructions, applicable internal staging location and constraints; shared manufacturer defaults may be reused. The capability is not an ADB `root_shell`/`app_data_write` grant and is not firmware-gated. Initial availability covers AYN, Retroid and AYANEO eligible Profiles with user-confirmed OEM menu. Devices without matching Profiles must be registered first; no per-model rollout mandate, but representative runner qualification and evidence for claimed supported setup results remain required.

EmuChef proper planning reuses current valid device-bound root qualification, or **automatically performs a bounded root check** when there is none. A confirmed denial/unavailable root allows an eligible OEM strategy; inconclusive/transport failures fail closed. Config Editor's explicit-only authored-generation root-probe policy remains unchanged. Verified direct ADB root is preferred and the strategy is frozen in the reviewed plan. **Loss of direct root after review or during execution fails closed**; never switch to OEM without a fresh plan/review.

The script compiler supports **every validated, typed root-capable EmuChef operation**, including protected file read/copy and predicates, permissions/app-ops and applicable file metadata, under the same App Definition and Recipe authority. Unrepresentable required steps cannot be silently dropped. No arbitrary Recipe shell fragments. Privileged read-back is bounded and purpose-specific.

## Stages, integrity and results

Prepare a uniquely owned **Privileged Setup Bundle** in the runner's Profile-approved internal shared storage. Stage only inputs needed for that privileged stage, including applicable shared-storage payloads; ordinary unrelated transfers use ADB. Bind the exact script/manifest/payload digests and device/target/run identity to the reviewed plan, verifying inputs/preconditions as feasible. Necessary inputs do not require additional prompts beyond normal review. Self-checks do not authenticate a compromised OEM runner.

The trusted planner may produce serial alternating stages: OEM script, ZIP reconciliation, normal ADB operations, another OEM script, and so forth. **Q78 B is a strict barrier:** while an OEM script is awaiting user action or is being reconciled, **no other Recipe execution may run**, even independent ADB steps. Bounded identity/status checks, ZIP transfer and owned-artifact cleanup needed to reconcile are permitted; serial Recipe actions resume only after reconciliation. Follow per-operation dependencies, truthful partial-state reporting and the accepted review-invalidation/repair rules. Retain non-authoritative pending stage pointers after app restart; never resume expired authority or rerun a script automatically.

The user starts each invocation via the OEM menu. **Q82 A requires a distinct immutable ZIP results archive for each invocation**, with a unique attempt identity linked to the reviewed plan/stage/run/device. A later attempt never overwrites an earlier ZIP. Each archive is stored outside staging and includes a manifest (plan and attempt identity, operation outcomes and artifact hashes) and bounded sanitized failures/logs. EmuChef imports and reconciles **all observed attempts** before determining final stage state. EmuChef pulls, checks archive structure/limits/paths/hashes, and retains a validated host-side copy for analysis. ZIP claims do not substitute for independent accessible target checks or app usability. Missing/malformed outcomes are unresolved and block further stages pending explicit reconciliation/fresh repair. Root-only inspection can use a separately reviewed script stage.

**Q67 B/Q75 A:** Allow reruns with idempotent typed operations or safe state checks. Already-satisfied operations can skip; an unknown/non-repeatable operation must refuse and require new reviewed intent. No blanket one-shot lock; staging deletion is not a duplicate-execution guard.

## Mandatory device cleanup

**Q71 B/Q76 A:** The OEM script **does not self-delete staging**. After user acknowledgement, validated ZIP import and determination the staging is no longer needed, EmuChef automatically removes the run-owned staging bundle. Actual emulator usability may be assessed later from safely retained evidence. The ZIP/results directory is distinct, preserving diagnostics during staging cleanup.

**Q72 A/Q77 A/Q80 A:** After protected host import, remove run-owned on-device ZIP and other no-longer-needed temporary artifacts. Failed, cancelled or abandoned runs retain needed diagnostic evidence until safely preserved or the user explicitly abandons them; then clean up. **Q81 C:** A device-side cleanup failure is reported as a **warning**, not a blocker to otherwise completed Recipe operations, later stages or normal setup completion. After validated result import/reconciliation, the stage barrier can release even if owned cleanup remains pending. Preserve a separate owned-artifact cleanup obligation, visibly track failed deletion, and retry on the next authorized connection. Do not claim deletion succeeded when it did not. If no reconnection occurs, provide precise manual cleanup guidance. Never erase unowned user files, or claim offline cleanup happened. **Cleanup of every unneeded EmuChef-created device temporary artifact is mandatory**.

## Initial acceptance target and rationale

**Q79 A:** Validate the first OEM-assisted protected write by installing RetroArch's main `retroarch.cfg` into its App Definition `external_app_data/files/retroarch.cfg` target, with proper app quiescence, stale-state and backup rules where applicable. Require protected-target content evidence beyond the ZIP and separately prove actual emulator usability. T06a addresses feasible config/resource routes and a complete required operation graph; T06b addresses real app usage; T13b/T15 address complete consumer setup. Copying one file is not proof of all privileged action parity. **UX-1 A** keeps installation, first-run initialization, OEM handoff and additional configuration reviews in one guided setup journey.

An OEM runner has an independent privilege and timing boundary from rooted ADB. Treating the menu as ADB root would defeat current guards and hide the Pocket Air Mini's unresolved app-storage feasibility. Keeping shared typed Recipes and independently verified execution-stage evidence avoids separate installation semantics.


## Final accepted OEM owner decisions (Q81–Q83)

**Q81 C — warning-only cleanup failure:** Required cleanup should be automated, audited, retried, and safe for user data. An individual failed removal is reported as a warning and does **not** prevent later Recipe steps or normal setup completion after the OEM result has been reconciled. Persist the cleanup-pending obligation; provide reconnection retries and safe manual guidance when offline.

**Q82 A — immutable per-invocation evidence:** Two manual launches of one reviewed script represent **two Privileged Execution Attempts** with distinct attempt identifiers and distinct immutable ZIP archives. Preserve both, including the first attempt's possibly partial mutation, and reconcile their claims with current device state. No successful rerun erases failure history.

**Q83 A — two scoped proposal tickets:** T18 owns runner capability, Device Profile support, compiler, staging and integrity. T19 owns ordered stage execution, user handoff, ZIP ingestion, recovery and cleanup. The original 21 proposed tickets are preserved wherever possible and extended to 23, with a small direct-edge correction making T13a a transitive predecessor of T13b through T19. **No GitHub ticket is created by this ADR.**

## Remaining design/engineering contracts

Cleanup failure handling and repeat-invocation archive identity are resolved by **Q81 C/Q82 A**. The proposed issue architecture adds **T18** (OEM capability, typed script compiler and staging integrity; blocked by T03/T05) and **T19** (serial stage coordination, user handoff, immutable ZIP reconciliation and mandatory cleanup; blocked by T18/T13a). T13b depends directly on T10/T19/T06a; T14 and T06b inherit T06a through T13b, while T15 consumes the implemented T18/T19 strategy and the T06a feasibility evidence transitively and qualifies the first protected RetroArch config copy. Specific OEM command environment, archive validator bounds, supported adapter guarantees, evidence/repair thresholds, run-by-run cleanup ownership and representative physical qualification are future engineering outputs—not unresolved product choices. **UX-1 A is accepted.** See [EmuChef product realignment proposal](../product/emuchef-product-realignment-proposal.md) §5.1. This ADR is not authorization for implementation, device tests, issue publication or release.

## Reviewed stage versus invocation attempt; admission and barrier release (T18/T19)

**Accepted T01 clarification; future contract:** A **Reviewed Execution** may
contain multiple ordered stages. A **Reviewed OEM Stage** compiles an immutable
**Privileged Setup Bundle** with a frozen plan digest, payloads/target and
**effective backup mode**. Each *individual user launch* creates a
**Privileged Execution Attempt** that inherits that same reviewed content and
backup mode, even if Settings changed between attempts. Only a newly prepared
and reviewed bundle can adopt a new preference or newly observed file bytes.

**Execution start** is the actual invocation of the OEM script by the runner,
not mere staging or the user's acknowledgement click. T18/T19 must provide
run-bound **started**, completed and interrupted/uncertain evidence, plus
ordering evidence independent of ZIP arrival. A run-scoped exclusive active
guard prevents *concurrent privileged mutation* by repeated invocations,
without imposing a permanent one-shot lock. Distinct attempts write distinct
immutable result ZIPs. Establish invocation ordering through a validated
monotonic journal/sequence or an equivalently demonstrated trusted ordering
mechanism; file arrival time is insufficient.

An OEM stage barrier releases only after reconciling **every known or possible
started invocation**. Missing finish evidence, a late/incomplete ZIP or a
completed first attempt followed by a second interrupted attempt leaves that
stage unresolved until reviewed reconciliation/repair. A necessary root-only
inspection is a separately reviewed, bounded reconciliation substage **under
the current barrier**, with its own invocation start/result evidence. It does
not authorize normal successor Recipe execution or prematurely end the barrier.
Test reversed archive arrival, competing invocations, and interrupted-after-
success evidence.

**No-clobber, backup and app-state preconditions:** The chosen strategy
must implement every mandatory typed-operation precondition, including
original-file hashing, app quiescence/preparation, permitted backup/waiver,
stale-state validation and race-safe create-only publication where required.
"Where feasible" never permits an overwrite after failed backup or
unreviewed file drift. If an enabled backup fails inside a manual script,
stop before the write and require a *new* reviewed continuation with specific
file/execution consent. A ZIP report alone does not establish recoverability.

**Cleanup and recovery original:** The Recovery Original (bytes and target
metadata retained under T10) is **not** temporary staging. An original is
never cleanup-eligible merely because a ZIP was successfully imported; first
prove qualified retention. An OEM temporary artifact becomes cleanup-eligible
only after needed invocation evidence, staged inputs and recovery originals
have been preserved. Attempt cleanup automatically and retain a separately
tracked `cleanup_pending` warning when deletion fails (**Q81 C**); after
the barrier is otherwise reconciled, cleanup failure alone does not block
later work or setup completion. Transport/identity/authority failure remains
subject to the existing fail-stop contract.

## Post-publication correction: device-side OEM invocation admission closure

**Accepted product direction, not current implementation.** T18 owns an enforceable **device-side, bundle-bound admission lifecycle**. An `open` Reviewed OEM Bundle may admit safe, state-checked **serial reruns** only while the OEM stage barrier is held. A manual script must check open admission and atomically acquire exclusive mutation authority while durably recording its own start/attempt ID. Closing admission must be **atomic relative to this actual start**: a racing invocation was either admitted and must later be reconciled or was rejected before mutation. The `closing` state refuses new starts while admitted work drains; `closed/revoked` permanently refuses this bundle's future mutations, **including when cleanup fails and the script remains on-device**. A host UI flag, attempted deletion, acknowledgement, ZIP import or previously released barrier is not device-side closure proof. No blanket one-shot lock is introduced; safe serial reruns are allowed while admission remains open.

**T19 owns closure, drain and barrier release.** Prove device-side closure to new admissions, then reconcile all admitted/possibly started invocation identities and results (ordered independently of ZIP delivery); establish none is active, incomplete or start-uncertain. Necessary separately reviewed root-only inspection is **bounded reconciliation within the held barrier**, with its own invocation start/outcome evidence. Release the barrier only after these conditions hold. If device closure or a racing start cannot be established, preserve `admission_closure_unverified` and existing fail-stop behavior; do not reclassify the issue as cleanup pending. A completed first attempt followed by an interrupted second cannot count as proven stage completion.

**Authority invalidation:** Preference changes, cancel/replan, superseded/replacement bundles and app restart/reconnect must use the **same atomic close/revoke and drain protocol**. Each admitted invocation retains its reviewed content, destination, mode and strategy, and the old bundle never becomes admissible merely because a new host session is created. Replacement requires its own fresh review. No automatic mid-run direct-root/OEM strategy switch.

**Cleanup is a distinct outcome (Q81 C):** Once admission closure and all attempt reconciliation are proven, qualified Recovery Originals and needed result evidence are retained, and temporary artifacts are eligible, failed deletion is a **nonblocking warning with `cleanup_pending`** and retry/manual guidance. A retained closed script remains inert; valid later Recipe steps and reconciled setup completion are permitted. Inability to prove closure remains a separate authority blocker.

**T13b evidence:** an invocation racing close; success → proven closure → failed cleanup → barrier release → manual retained-script launch produces no mutation; preference invalidation racing actual start; restart or replacement without old-bundle reactivation; and warning-only failed cleanup after proven authority closure. Preserve Q82 A immutable per-attempt ZIPs, accepted backup waiver behavior and safe serial reruns. Engineering selects the mechanism, but must demonstrate these outcomes.

