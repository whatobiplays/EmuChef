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
| End-user desktop app | React UI with trusted Rust/Tauri bridge | Active through reviewed-plan simulation; guarded real execution is specified but default-disabled and not implemented |
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
snapshots for trusted Phase 2A simulated execution. Tauri revalidates the
retained target, catalog, and canonical digest, then forces `dry_run` through
the existing Phase 0 operations. Its execution-handle store is bounded to one
active mapping and the latest terminal mapping. Complete snapshots are the UI
authority; incremental events are presentation data only. React receives no
sidecar execution identifier, full plan, exact serial, output path, or raw
sidecar response.

[Phase 2B guarded real-device execution](../product/phase-2b-guarded-real-execution.md)
defines the planned trust boundary for a later real workflow. The specification
keeps trusted start data and real mode selection in Tauri, reuses the existing
Phase 0 sidecar operations, and requires explicit platform-specific rollout
approval. It does not change current runtime ownership or enable real execution.
