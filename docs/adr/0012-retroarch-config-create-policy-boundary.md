# ADR 0012: RetroArch configuration-write policy boundary

**Status:** Accepted

**Supersedes:** The 2026-11-16 probe-based evidence gate for RetroArch `config` writes.

## Context

RetroArch configuration Create and Replace operate on the trusted, native-resolved `config` App Target. The target is an app-owned file destination with a frozen source, operation, and original-file precondition. Generic protected writes continue to follow their existing capability policy.

Root qualification, the verified root route, current device-session identity, Device Profile selection, and execution admission remain required for real execution. Native consent adds authority for one reviewed configuration operation; it does not replace those checks.

## Decision

`retroarch_config_write_policy` resolves the authored Recipe and source, frozen artifact, `config` App Target, package, catalog identity, destination, operation, source SHA-256, original-file precondition, and backup mode. Native authorization applies only to a matching Create or Replace operation for that target. The policy does not broaden `appDataWrite` or authorize another step that happens to use the same path.

Tauri presents native confirmation and sends a one-use claim through the existing `start_real_execution` admission path. The claim binds the immutable review digest, exact target and destination, Create or Replace operation, source hash, original-file precondition, backup mode, device identity, and session. Renderer data and ordinary sidecar requests cannot construct this claim.

Create requires a freshly reviewed confirmed-absent precondition and exclusive creation. Replace requires the reviewed original-file identity and byte precondition, then writes the complete frozen file directly in place. The executor stops the resolved RetroArch package once for Android user 0, checks that the process has stopped and has not relaunched, and revalidates the original precondition before writing.

Replace uses a durable private Mac backup by default. A backup failure blocks before the RetroArch config is opened. A waiver requires a fresh review derived from the retained failed report and a separate native confirmation. It binds the failed operation, report digest, exact target, source, original-file precondition, backup mode, and current device/session identity. Tauri consumes it once at dispatch. The waiver skips backup creation for that replacement only.

Missing or stale authorization, an unsupported root route, changed session or profile, a running or relaunched app, and failed file preconditions block execution. Selecting source `none` skips the configuration-specific operation.

## Historical rationale

The earlier temporary-marker and quarantine design was abandoned because changing UID, GID, or groups does not establish RetroArch's SELinux domain. A denied read does not prove that writes, directory traversal, or unlink are denied. A check followed by a path-based rename can move a raced replacement. Host access mocks and cross-compilation cannot establish Android SELinux or FUSE behavior. The helper therefore returned `ENOTSUP` before creating a probe marker because isolation had not been established.

The probe-based evidence gate also depended on a test-only evidence insertion path with no production writer. A universal write probe would change admission for established rooted workflows. Those findings remain relevant to the abandoned mechanism; the current policy authorizes one exact reviewed operation and retains all existing root and session checks.

## Consequences

RetroArch `config` writes have an operation-specific native authorization path. General `appDataWrite` behavior and permissions for other App Targets remain unchanged. Backups remain available in the private host folder without automatic pruning. V1 adds no Backup History, deletion UI, automatic rollback, or in-app restoration.

This decision does not establish physical-device qualification or production enablement. The KONKR target remains pending physical qualification.

## Historical implementation status — 2026-11-16

The KONKR Pocket Advance had a product Device Profile and linked base Device Plan with conservative capability defaults. Ambiguous eligible KONKR and AYANEO matches required a choice bound to the current device session. Host regressions covered unavailable required APK-install and app-launch steps and test-scoped evidence for the create-only `config` App Target. The production protected-app-data probe and trusted evidence bridge were absent, so real configuration creation remained blocked.

## Current implementation status — 2026-11-21

The Issue #56 implementation adds native one-use authorization for Create and Replace, default private backups, a report-bound backup-failure waiver, and host-side replacement checks. Host and fake-device checks do not establish Android behavior. No physical-device operation or production enablement has been performed.
