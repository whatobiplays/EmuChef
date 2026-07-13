# Planner and Executor

The planner loads an authored device plan and profile, selects recipes, applies
input bindings, expands dependencies, normalizes steps, and emits a typed
execution plan. Optional live ADB probing supplies detected facts before
explicit CLI context overrides are applied to the emitted context.

Runtime input declarations, saved user configurations, binding precedence,
provenance, discovery, and side-effect-free plan creation are specified in
[Runtime Recipe Configuration](runtime-recipe-configuration.md). The planner is
the only layer that merges those sources. The execution plan contains normalized
effective inputs; the executor does not load saved configuration or apply
precedence.

The executor processes steps in dependency order on one thread. Failure blocks
dependent work, skip conditions produce skipped results, and verification can
fail a completed action. Progress events report execution and per-step state.

Artifact resolution supports absolute `file://`, HTTP, and HTTPS URLs inside the
runtime/cache sandbox. The resolver owns compatible URL-based filenames,
destination selection, partial-file cleanup, and no-clobber publication. The
transport owns serial blocking HTTP requests and fixed-size response streaming.

HTTPS uses strict Rustls certificate and hostname validation. The client has a
15-second connect timeout, one five-minute deadline across headers, redirects,
and body, a five-redirect limit, and no retries. HTTPS-to-HTTP redirects fail
before the downgraded request. Transparent decompression, resume, freshness
checking, and authored checksums are absent. Standard system proxy discovery is
enabled without an EmuChef-specific configuration surface.

Complete default-cache regular files are authoritative and bypass URL parsing
and client construction. New files use unique partials in the destination
directory, `sync_all`, and `persist_noclobber`; the exact underlying
no-overwrite primitive is platform-dependent. `cache: none` always transfers
and uses a unique runtime filename when the compatible base path already exists.
