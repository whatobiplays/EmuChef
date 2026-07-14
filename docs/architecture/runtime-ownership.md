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
| End-user desktop app | React UI with trusted Rust/Tauri bridge | Phase 2C completion/report/repair active; guarded real execution remains default-disabled |
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
