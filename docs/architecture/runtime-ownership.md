# Runtime Ownership

| Surface | Owner | Status |
| --- | --- | --- |
| Product CLI | Rust | Active |
| Planner | Rust | Active |
| Validation | Rust | Active |
| Executor | Rust | Active |
| Real-ADB apply | Rust | Active; manual device validation required |
| Tauri editor backend | Rust JSONL sidecar | Active |
| Phase 0 end-user runtime protocol | Rust JSONL sidecar | Active backend contract |
| End-user desktop app | React UI with trusted Rust/Tauri bridge | Phase 3E Apple Silicon packaging qualification active; guarded real execution remains default-disabled |
| Platform-Tools import and verification | Rust/Tauri | User-supplied, app-managed, macOS-only |
| Device storage discovery and selection | Rust sidecar and Tauri | Passive ADB volume observation; selection is intent only |
| Catalog source resolution | Rust | Bundled/local snapshots active; cached remote reserved |
| Execution reports and incremental events | Rust JSONL sidecar | Active in-memory session contract |
| Python runtime | None | Retired; no implementation, package, dependency, or entrypoint is retained |
| Compatibility fixtures | Frozen v1 evidence | No Python regeneration |
| Network artifact download | Rust | Active; manual device evidence required |
| End-user macOS package qualification | Rust/Tauri and local Node tooling | Apple Silicon ad-hoc content qualification active; credentialed verification is explicit and separate |
| End-user update discovery | Rust/Tauri | User-triggered signed metadata validation and retained fixed-origin DMG browser handoff; production trust unconfigured |
| End-user release signing/notarization/manual replacement | External Apple release operation | Credentialed artifact verification and manifest tooling implemented; hosting, production trust, and clean-Mac evidence pending |

There is no supported alternate runtime, planner backend selector, fallback,
or secondary product executable. Python is not a repository prerequisite and
does not participate in product verification.

The additive `phase0_end_user_runtime` protocol extension is negotiated
explicitly. It exposes product catalog inventory, reviewed-plan digests,
target-bound real and simulated execution, retained snapshots, ordered events,
and cooperative cancellation without adding product behavior to the editor
frontend. Filesystem roots are sidecar startup policy, not `startExecution`
payload fields. Execution retry creates a new attempt; rollback and device-state
undo are not runtime capabilities.

The end-user app launches and negotiates the sidecar independently of ADB.
Platform-Tools availability gates device discovery only. Sidecar-internal DTOs
may carry exact serials and resolved paths; React DTOs use opaque device/review
handles and never contain exact serials, managed executable paths, catalog
roots, or full reviewed plans. The trusted bridge retains target-bound review
snapshots for Phase 2A simulation and default-disabled Phase 2B real execution.
Tauri revalidates the retained target, catalog, canonical digest,
Platform-Tools identity, and unambiguous user-supplied file/directory inputs,
then selects `dry_run` or `real` without accepting mode from React. Its
kind-aware execution-handle store is bounded to one shared active mapping and
the latest shared terminal mapping. Complete snapshots are the UI authority;
incremental events are presentation data only. React receives no sidecar
execution identifier, full plan, real-flow serial representation, output path,
or raw sidecar response.

[Phase 2B guarded real-device execution](../product/phase-2b-guarded-real-execution.md)
defines the implemented trust boundary. The default-off `real-execution` Cargo
feature is the authoritative gate and its availability query exposes only that
policy boolean. Platform-specific packaged evidence and approval remain
external release prerequisites; ordinary builds do not enable real execution.
## Phase 2C authority boundary

React owns presentation and user intent only. Tauri owns native report writing,
retained review/execution associations, remediation projection, and opaque
one-shot launch actions. The Rust sidecar owns authoritative execution reports,
launch-candidate rederivation, target preflight, and the typed ADB launch
operation. One-shot consumption is intentionally not duplicated in the
sidecar; it belongs solely to the Tauri action-handle store.

## Phase 3A saved-configuration boundary

The sidecar owns schema-v1 configuration documents and persistence. Tauri owns
native file dialogs, configuration paths, sidecar document IDs, opaque document
handles, and the private recent-file index. React receives only safe document
names, portable intent, dirty state, validation status, and sanitized
diagnostics.

The saved device-plan value is an authored plan reference, not a generated
execution plan. Opening or reusing a document invalidates prior device facts,
generated plans, digests, reviews, executions, confirmations, and launch
actions. Fresh probe, match, catalog validation, description, planning, and
review are required before execution.

## Phase 3B support and cache boundary

The end-user Tauri app injects its trusted application-data artifact-cache root
when it starts the sidecar. This is app-specific runtime construction; backend
defaults, CLI behavior, and config-editor startup remain unchanged. React never
supplies or receives the root, a cache path, filename, metadata identity,
diagnostics destination, or diagnostics bytes.

The sidecar publishes optional safe metadata beside new default-cache payloads.
Tauri inventories payload plus metadata as one logical opaque entry, includes
both components in size accounting and stale checks, blocks cleanup while
execution use is possible, and owns every filesystem deletion. Unrecognized
files and orphan metadata remain unmanaged and non-removable.

## Phase 3C presentation boundary

React owns semantic HTML, keyboard interaction, focus containment and
restoration, validation summaries, live announcements, progress presentation,
responsive layout, and the sanitized top-level render fallback. These
presentation responsibilities do not authorize React to supply or retain
paths, exact serials, raw sidecar data, reviewed plans, execution modes, cache
roots, diagnostics destinations, or filesystem actions.

Promise-backed frontend prompts have exactly one resolver and a safe cancel
result. Runtime restart, configuration replacement, app reset, unmount, and
error-boundary activation cancel pending presentation requests without
implicitly saving, discarding, deleting, cleaning, or starting execution.
Focus restoration validates the original invoker and otherwise follows the
documented workflow, main-content, and header fallback order. A stale dialog or
native-picker return cannot steal focus from a newer transition.

## Phase 3D recovery boundary

Tauri owns one fixed-path, bounded, versioned recovery record and the
interrupted-session marker. React can submit only portable intent through a
strict DTO; it cannot choose the recovery path or submit device, review,
execution, dialog, cache, diagnostics, or sidecar authority. Atomic writes and
request, draft, and record generations prevent stale replacement or clearing.

Binding persistence is controlled only by trusted authored `sensitive`
metadata from the sidecar description. Sensitive and unclassified values are
omitted and represented by key-only re-entry requirements. The source saved
configuration is referenced by its embedded identity rather than a React
handle or path. Restore creates fresh document authority and leaves every
device, plan, review, execution, confirmation, and launch association invalid.

Deferred recovery disposition is independent of current-session dirty state.
Not now survives clean shutdown and is offered on the next launch; explicit
discard, successful save, or an atomic newer dirty generation supersedes it.

## Phase 3E packaging boundary

Tauri resolves the bundled catalog through its release resource directory and
the sidecar beside the packaged main executable. The release probe uses those
same paths and the normal negotiated protocol; it adds no planner, device,
filesystem, or execution authority. The probe report contains only readiness,
catalog-operation, and default-disabled real-execution booleans.

Local qualification deletes the fixed Apple credential allowlist from the
child build environment and explicitly selects ad-hoc signing. Credential
validation occurs only after an operator selects developer-id mode. The local
manifest records safe provenance, normalized executable/resource/configuration
content, and per-build raw identities without environment dumps or credential
values.

The normalized content digest is the repeatability contract. Code-signature
blobs, signing timestamps, DMG container metadata, mtimes, temporary names, and
caller-specific absolute paths are volatile and excluded. Final signed app and
DMG hashes are identities of one produced build and are not asserted to be
stable across rebuilds.

## Phase 4B update-discovery boundary

Rust owns the fixed manifest endpoint, DMG origin/path policy, metadata public
key, no-proxy identity-only HTTP client, bounded response reader, exact
fixed-JSON parser, Ed25519 verification, target/version/expiry policy, retained
candidate, interaction leases, and browser call. React receives display-only
metadata and action availability; it cannot supply or receive an update URL,
key, signature, path, raw manifest, response, or opener argument.

Production trust is a source document containing only schema version 1 and
`configured: false`. This returns a local unconfigured state without network or
browser activity. The browser downloads a DMG only after a user action; EmuChef
does not read or verify that local file. Apple Developer ID, notarization,
stapling, and Gatekeeper remain executable trust. Platform-Tools and all
application-data state remain outside update authority.

The prior in-place updater remains rejected because its pinned public API could
not preserve the required single-response pre-deserialization validation model.
Phase 4B implements manual discovery only, not in-place installation.

## Device storage boundary

Storage discovery is passive. React requests a storage projection for one
opaque device handle. Tauri validates the retained device session, confirms the
ADB inventory contains exactly that serial, and forwards only the ADB path and
serial to the sidecar listDeviceVolumes operation. The sidecar runs adb devices
-l, dumpsys mount, and sm list-volumes all as argument arrays. It parses the
current AOSP Disks:, Volumes:, Records:, and Primary storage UUID: sections,
retains legacy mDisks/mVolumes/mPrimaryStorageUuid compatibility, and accepts
both two-field legacy and three-field AOSP shell volume lines. Repeated
VolumeInfo or DiskInfo records are accepted while the fields used for
classification agree; a repeat that changes only fields EmuChef does not
classify by stays accepted. Contradictory classification evidence and
conflicting primary-storage declarations fail closed. Raw filesystem UUIDs,
mount paths, disk identifiers, and serials remain sidecar-owned. An adopted
primary volume resolves to its stacked emulated volume
through the private volume's disk identity, without relying on the private
volume's mount user. A sole emulated volume with no positive user evidence uses
the assumed-user compatibility fallback. A volume positively associated with a
non-system user, including an explicit emulated;<non-zero> primary declaration,
is ambiguous and is not offered as internal storage. A public primary volume is
promoted to internal storage only when its public-volume classification is
available, which requires supported non-USB media, visible mount evidence, and
agreement from sm list-volumes. Each volume also carries a sidecar continuity
digest over its disk, mount and backing paths, mount flags, mount state, and
filesystem UUID.

Tauri runs the two storage commands off the main thread. It re-reads the device
session after the device command finishes, discards superseded probe responses,
and caches one observation for ten seconds keyed by device handle, session
epoch, runtime generation, and Platform-Tools revision. Cache lookup uses the
request-start time. A successful probe records its fetched_at at probe
completion, and projection and admission apply the ten-second limit at their
current time. After the sidecar request returns, Tauri re-reads the runtime
generation and Platform-Tools revision. A changed value discards the observation,
clears the selection, and returns a fixed stale-context error. An expired
observation is discarded before a busy probe returns. The bridge mints stable
opaque volume handles per handle, epoch, and identity token, plus a
presentation-only key for duplicate non-selectable list entries. The continuity
digest stays in trusted state.

Selection is user intent stored in Tauri only. Admission re-locks state,
revalidates the captured session epoch, identity, the expected observation token,
and the latest per-device selection request sequence. A token that no longer
matches the current trusted observation returns stale_observation and cannot
rebind an older choice to a replacement card. Admission rejects stale, missing,
unsupported, or ambiguous choices. Explicit clearing carries no volume handle or
token and only removes retained intent. Any change of session epoch, inventory
continuity, runtime generation, Platform-Tools revision, or continuity evidence
invalidates both the observation and the selection. A failed probe or a context
failure before the probe also clears the retained selection.

React renders the projection in the setup step as labelled storage choices
beside a Not offered as a destination list. It receives product labels, fixed
detail text, opaque handles, presentation keys, and cached-versus-live evidence
only. Radio accessible names contain only the option label, and detail text is
exposed through aria-describedby. Focus restoration applies only to a still
present option and only while focus remains in the storage surface. A failed
read leaves error announcement to the panel alert. Invalidation notices use one
panel live region. When a read fails, the projection becomes an unavailable state
with no volumes and no selected handle, so a stale checkmark and live-evidence
label cannot remain visible. A stale observation error keeps the current trusted
projection and any newer selection. A mounted volume does not imply writability,
application access, or execution eligibility. A read-only mount stays visible in
the non-destination list with its mount detail. The selection never participates
in planning, review, execution, or real-device authority.
