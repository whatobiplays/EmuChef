# Qualification Session Device Reassociation Design

## Problem

Qualification sessions persist an opaque device handle so the captured run can
be audited. Production device handles are valid only within one application
process. After restart, device discovery assigns a new handle even when the same
physical target reconnects.

The restored session currently creates an intent lock before the normal workflow
has selected a device. The Connect row and its selection handler treat that
intent lock as a device-selection lock. The qualification refresh effect cannot
resolve the state because its process-local device reference is initialized only
when a session starts in the current process. The backend also compares a refresh
request against the historical persisted handle before observing current device
facts.

## Required Behavior

A restored session keeps its device plan and required recipes locked. It does
not lock normal device selection until the session has a trusted current-process
device association.

After the operator selects and probes a device through the ordinary workflow,
the existing qualification refresh command observes the device through the
production probe, match, device-qualification, and root-check authorities. The
observed profile, manufacturer, model, Android version, Android API level,
ABI/SoC class, firmware build, and root state are compared with the immutable
target facts stored in the session.

If every material fact matches, the current process associates the qualification
session handle with the selected live device handle. The historical device
handle in the persisted session remains unchanged. If any fact differs or the
trusted observation fails, the existing monotonic invalidation path marks the
session invalid and prevents valid evidence promotion.

## Runtime Authority

`SessionHandles` owns the in-memory mapping from qualification session handle
to live device handle. It is the existing process-local authority for opaque
device identities, so no second discovery or identity system is introduced.

A newly created session records its association after the session has been
created successfully. A restored session has no association because a new
`SessionHandles` instance starts empty. Runtime invalidation clears session
associations together with other live handle authority. Device disappearance,
identity-continuity loss, and explicit identity invalidation also clear every
association for the affected opaque handle. Candidate discard, finalization,
and recording remove the session entry so completed lifecycles do not retain
transient authority.

Review and execution binding require a current association. Their device-handle
checks compare production review or execution state with the associated live
handle, not the historical handle stored in the session file. Binding fails
closed if refresh has not established an association.

## Frontend Coordination

The qualification controller exposes device-selection locking separately from
the existing intent lock:

- `intentLock` remains active for every live or restored session and continues
  to lock the device plan and recipe set.
- Device selection is locked only when the session has a current-process device
  association.

When a restored session observes a selected and probed workflow device, the hook
uses that live handle for `refreshQualificationSession`. It does not begin a new
session. A successful backend response retains the session and its checkpoints.
An invalid response remains visible as the canonical invalid session state.

The Connect UI continues to enforce availability, saved-configuration, multiple
device, and busy-state rules. A connected row is not disabled solely by a
restored qualification intent lock. When an active same-process session does
lock selection, the row exposes an explicit explanation.

## Tests

Deterministic tests cover:

1. A restored session selecting and probing a new handle whose material target
   facts match. Refresh preserves validity, checkpoints, plan, recipes, and the
   original session.
2. A restored session selecting and probing a device with different target
   facts. Refresh uses canonical invalidation and does not establish an
   association.
3. Normal selection with no qualification session.
4. An active same-process session retaining its existing lock and association.
5. A connected Connect row remaining selectable when only a restored session
   exists.
6. Review and execution binding failing without a validated runtime association
   and accepting only the associated live handle.

## Scope

The implementation does not create or resume physical execution, create a new
qualification session, alter authored recipes or profiles, modify the target
registry, record evidence, update the qualification matrix, delete runtime
state, commit, or push.
