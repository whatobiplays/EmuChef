# Phase 3B Support Diagnostics and Artifact Cache

## 1. Product boundary

Phase 3B adds a Support & Storage panel to the end-user application. The panel
exports a bounded sanitized diagnostics ZIP and manages the artifact cache that
belongs to that application. It does not add telemetry, upload, crash reports,
cloud state, remote catalogs, accessibility redesign, packaging qualification,
or release enablement.

React owns presentation, selection intent, and explicit confirmation. Tauri
owns the native save dialog, diagnostics bytes, app-data cache root, opaque
cache-entry mappings, execution-use checks, and deletion. The Rust sidecar
continues to own artifact resolution and execution.

## 2. End-user cache ownership

The end-user Tauri application resolves its platform application-data directory
and starts its sidecar with an explicit `<app-data>/artifact-cache` root. That
root is fixed by trusted startup construction. React, environment variables,
the working directory, and runtime requests cannot redirect it.

This policy is specific to the end-user application. The backend default
remains `.emuchef_cache/artifacts` beneath its current working directory when no
explicit root is supplied. Product CLI behavior, the config editor, tests, and
other embedders retain their existing cache-root behavior. Phase 3B neither
migrates nor deletes legacy caches.

## 3. Logical cache entries

A managed cache entry is one logical unit containing a payload and optional
schema-v1 metadata sidecar. Inventory exposes one opaque handle for the unit,
never separate payload and metadata handles. Entry size includes both files.

Metadata contains only the payload filename, a validated artifact label, source
kind, SHA-256 source fingerprint, expected payload size, and payload
modification fingerprint captured before promotion. It contains no raw URL or
filesystem path and is not execution authority. `complete` means the regular
payload and metadata agree structurally and match that filesystem fingerprint;
it is not a cryptographic authenticity claim. Missing metadata produces
`unindexed`, interrupted download files produce `incomplete`, and inconsistent
metadata produces `metadata_mismatch`.

Artifact bytes are written to a unique same-directory temporary file. Metadata
is prepared and validated before payload promotion, then published atomically
as a sidecar. Metadata publication failure does not fail execution: stale
metadata is removed when possible, any retained stale sidecar fails payload
fingerprint validation, and the usable payload is left unindexed. Temporary
files are cleaned up best effort.

Inventory accepts at most 4,096 directory entries and 16 KiB per metadata
record. Recognized payloads and partial files are manageable. Unrecognized
files and orphan metadata are counted as unmanaged and remain non-removable.

## 4. Cleanup authority and lifecycle

Selective cleanup, Clear unused, and Clear all removable accept only the current
inventory generation, opaque handles, and an exact confirmed entry count and
aggregate byte size. Tauri revalidates the canonical root, direct-child
confinement, regular-file and non-symlink status, payload and metadata
fingerprints, logical association, and execution state immediately before
deletion.

Cleanup is unavailable while an execution is starting or active. Any uncertain
association fails closed. In-use state applies to the complete logical entry.
Metadata is removed before its payload; if payload removal then fails, the
entry is reported as a sanitized partial failure and becomes unindexed. A
metadata-removal failure leaves the payload untouched and is not reported as
success. Every cleanup returns stable per-entry outcomes and a fresh inventory,
invalidating all prior handles.

Runtime restart invalidates cache mappings and frontend support generations. It
does not modify cached bytes, portable saved-configuration files, or the recent
configuration index.

## 5. Diagnostics bundle

The native save dialog produces `emuchef-support-diagnostics.zip`. React sees
only `saved` or `cancelled`; the selected destination and ZIP bytes remain in
Tauri. The deterministic schema is `emuchef.support-diagnostics` version `1`
with fixed member order and timestamps:

1. `manifest.json`
2. `runtime.json`
3. `catalog.json`
4. `configuration-summary.json`
5. `execution-summaries.json`
6. `cache-summary.json`

The bundle is limited to 2 MiB compressed and uncompressed. It includes app and
runtime versions/status, OS class, compile-time feature gates, public catalog
identity/digest, aggregate saved-configuration availability, aggregate retained
execution state, and aggregate cache counts/sizes.

It excludes configuration names and contents, bindings, handles, paths,
filenames, exact serials, usernames, home directories, environment variables,
raw URLs, credentials, query strings, logs, stdout/stderr, ADB output, plan
bodies, raw errors, file contents, and crash data. A final recursive sanitizer
rejects path, credential, token, URL-suffix, control-character, and newline
patterns even after allowlist construction.
