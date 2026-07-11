# Runtime Ownership

| Surface | Owner | Status |
| --- | --- | --- |
| Product CLI | Rust | Active |
| Planner | Rust | Active |
| Validation | Rust | Active |
| Executor | Rust | Active |
| Real-ADB apply | Rust | Active; manual device validation required |
| Tauri editor backend | Rust JSONL sidecar | Active |
| Python runtime | Frozen legacy/reference only | Pending deletion |
| Compatibility fixtures | Frozen v1 evidence | No Python regeneration |
| Network artifact download | Rust | Not implemented; next feature |
| Release signing/notarization/updater | Not implemented | Future work |

There is no supported alternate runtime, planner backend selector, Python
fallback, or secondary product executable. The retained Python packages have no
entrypoint and do not participate in product verification.
