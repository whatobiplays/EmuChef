# Packaged GUI E2E Checklist

This checklist records manual evidence for a real packaged Config Editor. The
simulated bundled-sidecar smoke is useful automation but does not complete this
checklist.

## Build

```bash
cd apps/config-editor
npm ci
npm run check:rust-runtime
npm run check:sidecar:bundle-input
npm run tauri build
```

## Exercise

1. Install or open the produced package on the target host.
2. Confirm the application starts without a terminal or development server.
3. Confirm sidecar status and protocol compatibility are healthy.
4. Open an authored recipe, edit every supported section, undo, redo, validate,
   refresh YAML, save, and save as.
5. Restart the sidecar and confirm stale-session recovery behaves safely.
6. Close with clean, dirty, and in-flight document states.
7. Confirm logs contain no Python launch, missing-sidecar, or protocol errors.

## Evidence

| Field | Value |
| --- | --- |
| Date | |
| Commit SHA | |
| Host OS and architecture | |
| Package path | |
| Install/launch result | |
| Editor workflow result | |
| Sidecar restart result | |
| Log path | |
| Notes | |

An empty checklist is not release evidence.
