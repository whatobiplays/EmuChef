# Device-Unauthorized Identity-Precedence Qualification Design

## Decision

Keep the safe-boundary physical `device_unauthorized` procedure and preserve production identity-first execution ordering.

A qualifying authorization transition may terminate with either:

- `device_unauthorized`, when a production ADB command returns a recognized unauthorized transport response; or
- `device_identity_unverified`, when the production pre-operation identity guard cannot collect complete identity evidence from the independently observed unauthorized device.

`device_identity_unverified` is not sufficient by itself. It qualifies only when the exact authorization-transition evidence proves the same selected serial was initially authorized, the first reviewed operation completed, authorization was revoked at the safe boundary, the serial was absent, the same serial reconnected as `unauthorized`, the second operation was released only after that observation, and final authorized cleanup completed.

## Boundaries

- Do not change `RealAdbDevice` identity checks, transport classification, failure precedence, issue codes, retry/resume behavior, public APIs, or mandatory counts.
- Preserve the actual terminal issue in `authorizationTransition.issueCode`.
- Reject disconnect, offline, changed identity, generic unverified identity, mismatched transition/terminal codes, and missing authorization chronology.
- Preserve both existing blocked authorization attempts as non-passing audit evidence.
- Add the pre-change safe-boundary contract as an exact non-passing legacy audit snapshot so the latest blocked record remains valid after the contract update.

## Evidence contract

The current `device_unauthorized` contract allows:

- issue codes `device_unauthorized` and `device_identity_unverified`;
- exactly one completed first step and one failed second step;
- no active-process evidence;
- the existing exact safe-boundary authorization chronology;
- authority invalidation, no automatic resume, production slot release, successful cleanup, and clean residual state.

The independent transition evidence remains the authority for proving the device was unauthorized. The terminal code records which production layer failed first.
