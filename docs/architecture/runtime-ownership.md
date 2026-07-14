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
| End-user desktop app | React UI with trusted Rust/Tauri bridge | Phase 3C accessible and resilient presentation active; guarded real execution remains default-disabled |
| Platform-Tools import and verification | Rust/Tauri | User-supplied, app-managed, macOS-only |
| Catalog source resolution | Rust | Bundled/local snapshots active; cached remote reserved |
| Execution reports and incremental events | Rust JSONL sidecar | Active in-memory session contract |
| Python runtime | Frozen legacy/reference only | Pending deletion |
| Compatibility fixtures | Frozen v1 evidence | No Python regeneration |
| Network artifact download | Rust | Active; manual device evidence required |
| Release signing/notarization/updater | Not implemented | Future work |

There is no supported alternate runtime, planner backend selector, Python
fallback, or secondary product executable. The retained Python packages have no
entrypoint and do not participate in product verification.

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
