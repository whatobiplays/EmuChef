# Phase 6D.6 Active Host-Push Stimulus Design

## Status

Approved for implementation planning.

## Problem

The first implementation of exact active-process qualification used a calibrated device-side `cp` operation and required a predicted operator window of 15–240 seconds. Physical diagnostics on the AYANEO Pocket S2 Pro showed that this design cannot satisfy its own bound:

- A fully allocated 256 MiB device file copied to another fully allocated device file in 0.27 seconds.
- The committed 8 GiB maximum therefore predicts only about 8.6 seconds.
- Reaching the 30-second target would require roughly 28 GiB for the source and another 28 GiB for the destination.

The failure is not sparse-file behavior. `stat` and `du` showed equivalent allocation for the source and destination. The harness correctly blocked before qualification and completed fixture cleanup.

A second diagnostic showed that a 256 MiB non-compressible host file pushed through ADB took 6.88 seconds. This supports a practical active payload of about 1.1 GiB for the existing 30-second target while consuming only one payload on the device.

## Decision

Use a calibrated, non-compressible, fixture-owned host file and the production ADB push path for the four supported active scenarios:

- `cancellation_active`
- `usb_disconnect_active`
- `device_offline`
- `device_unauthorized`

The exact observed process operation is `ProcessOperation::Push`. Evidence serializes this as the semantic operation class `host_push`.

Do not weaken the 15–240 second predicted-duration contract, the five-second action freshness window, fixed process deadlines, cancellation semantics, process ownership, cleanup requirements, or evidence chronology.

## Stimulus preparation

### Host fixture

The harness creates a unique run-owned temporary host workspace outside the repository and authored corpus. Pass that workspace explicitly into the executor sandbox read allowlist for the reviewed run; do not broaden any persistent or global sandbox root. Its file bytes are deterministic, non-secret, and non-compressible for transport purposes. Generate them as a fast streaming SplitMix64-style sequence seeded from the run-scope digest rather than using an all-zero sparse or compressible payload. This is fixture generation, not cryptography.

Requirements:

- Calibration size remains 256 MiB.
- Active size remains bounded between 512 MiB and 8 GiB.
- Creation must stream in bounded memory using fixed-size chunks.
- The temporary workspace and partial host files are removed on every error path and normal completion.
- The reviewed executor receives only this run-owned workspace as the additional host read root.
- The host file size is verified before use.
- Host paths, random-looking fixture bytes, and device paths never enter public evidence.

### Calibration

Calibration uses the same production ADB push adapter as the reviewed active step, without the qualification observer attached. It pushes the 256 MiB host calibration file into the unique fixture-owned device scope, measures elapsed time, verifies the destination size, and removes the device calibration destination before deriving the active payload.

Derivation keeps the existing formula and bounds:

- target duration: 30 seconds;
- accepted predicted range: 15–240 seconds;
- payload range: 512 MiB–8 GiB.

A zero or unrepresentable duration, a derived duration outside the accepted range, a size mismatch, or cleanup failure blocks the attempt.

### Capacity checks

After calibration cleanup, require:

- device free space for the active destination plus 1 GiB cleanup headroom;
- host capacity sufficient to create the active source file;
- no pre-existing run-scoped host or device fixture path.

The previous requirement for both a device-side source and destination is removed for these four active scenarios because only the push destination is stored on the device.

## Reviewed execution and observation

The first reviewed `copy_files` step uses a host `file_path` source and a unique device destination. This routes through the ordinary production `device.push` implementation and creates one owned `ProcessOperation::Push` child.

The qualification observer:

1. waits for the exact `Push` mutation event;
2. samples that owner-held child alive;
3. creates `active-ready`;
4. requires `operator-action` within five seconds;
5. records terminal evidence for the same operation identity;
6. preserves the existing active-cancellation or transport-transition semantics.

For `cancellation_active`, the marker requests cancellation. The active atomic push settles truthfully, later work remains unattempted, and the production execution slot is released. The scenario does not use `terminal-ready` or `cleanup-ready`.

For disconnect, offline, and unauthorized scenarios, the existing `terminal-ready` and `cleanup-ready` recovery protocol remains unchanged.

## Evidence contract evolution

Do not relabel a `Push` child as `device_copy`.

Add `host_push` as an allowed operation-class value in the shared evidence schema. Change only the four supported active scenario contracts to require `host_push`. Keep these contracts as `device_copy`:

- existing boundary, identity, and root scenarios;
- `operation_timeout`;
- both host-sleep scenarios.

The writer derives `scenarioFacts.operationClass` and `activeProcess.operationClass` from the selected reviewed operation. Rust and Node validators compare the observed value with the selected scenario contract rather than imposing one global operation class.

The six accepted physical records remain unchanged and valid as `device_copy` evidence.

## Cleanup and failure behavior

Cleanup removes only run-owned resources:

1. active device destination;
2. calibration device destination if still present;
3. active host file;
4. calibration host file;
5. the run-owned temporary host workspace;
6. run-scoped device fixture directories and sentinel markers.

Operation failure, cancellation, transport loss, calibration failure, host-file generation failure, and cleanup failure remain separately reportable. No retry, resume, automatic reconnect, or ownership transfer is added.

## Automated tests

Add or update tests that prove:

- deterministic SplitMix64-style fixture generation is non-zero, varies across blocks, streams in bounded memory, reproduces for the same seed, differs across run scopes, and produces the exact requested length;
- partial host fixtures are removed after generation failure;
- 256 MiB at 6.88 seconds derives an active payload near 1.1 GiB and a prediction inside the existing window;
- unusably fast, slow, zero, overflow, and capacity-insufficient calibrations fail closed;
- the calibration and reviewed step use `ProcessOperation::Push`;
- exact `Push` lifecycle evidence serializes as `host_push`;
- relabeling a `DeviceCopy` event as `host_push`, or a `Push` event as `device_copy`, is rejected;
- only the four supported active contracts change to `host_push`;
- all six accepted physical records continue to validate without modification;
- host and device fixture cleanup remains exact and run-scoped;
- the complete Rust backend real-execution suite and Phase 6D.6 Node validators remain green.

## Out of scope

This correction does not:

- qualify `operation_timeout` or host sleep;
- change production deadlines or cancellation behavior;
- add a private delay or artificial shell sleep;
- increase the device-copy ceiling;
- create a many-small-files workload;
- modify accepted physical evidence;
- close Phase 6D.6.
