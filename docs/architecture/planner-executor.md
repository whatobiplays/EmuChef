# Planner and Executor

The planner loads an authored device plan and profile, selects recipes, applies
input bindings, expands dependencies, normalizes steps, and emits a typed
execution plan. Optional live ADB probing supplies detected facts before
explicit CLI context overrides are applied to the emitted context.

The executor processes steps in dependency order on one thread. Failure blocks
dependent work, skip conditions produce skipped results, and verification can
fail a completed action. Progress events report execution and per-step state.

Artifact resolution supports local paths and `file://` URLs with runtime/cache
sandboxing. HTTP(S) downloads are intentionally unsupported. The next feature
must use strict TLS, redirects, timeouts, deterministic cache keys, temporary
files, atomic rename, partial cleanup, typed errors, local HTTP-server tests,
and full clean-cache RetroArch validation.
