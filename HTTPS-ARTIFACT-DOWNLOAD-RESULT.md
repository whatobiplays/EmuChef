# HTTP(S) Artifact Download Result

Date: 2026-07-10

HTTP and HTTPS artifact downloading is implemented in the Rust product runtime.
The executor remains single-threaded, existing execution-plan and protocol
surfaces are unchanged, and real-device validation remains manual.

## 1. Commits

| SHA | Purpose |
| --- | --- |
| `e21697e` | Extract artifact resolution, compatible filename derivation, local-file copying, and transport seams from the executor. |
| `877ec94` | Add typed artifact failures, stable codes, and one redacted executor-message boundary. |
| `03d970d` | Add the blocking Reqwest/Rustls HTTP transport, manual redirects, fixed timeouts, and streamed responses. |
| `8156d64` | Add same-directory partial files, flush/sync, no-clobber publication, cleanup, and nonpersistent `cache: none` behavior. |
| `fbfe65e` | Add deterministic local HTTP and TLS coverage plus executor failure and cache tests. |
| `d95abd2` | Add process-level `emuchef` validate, plan, and dry-run apply coverage. |
| `74a4a58` | Document current network artifact behavior and the remaining manual validation boundary. |

This report is committed after those seven commits. Its own SHA is intentionally
not embedded because doing so would require rewriting the report commit.

## 2. Module Structure

- `src/artifact_resolver.rs` owns compatible filenames, scheme routing,
  sandboxed destinations, cache policy, partial-file lifecycle, publication,
  resolved results, typed failures, and redacted messages.
- `src/artifact_transport.rs` owns local byte copying, blocking HTTP requests,
  Rustls client setup, redirects, timeouts, response status handling, and
  fixed-size response streaming.
- `src/executor.rs` retains step orchestration, artifact runtime-state updates,
  runtime references, step failure, dependency blocking, and unrelated-step
  continuation.

All new implementation types are crate-private. No public Rust API, authored
schema, CLI option, environment variable, JSONL field, or frontend behavior was
added.

## 3. Dependencies

| Dependency | Resolved version | Use |
| --- | --- | --- |
| `reqwest` | `0.13.4` | Blocking HTTP client with `default-features = false` and only `blocking`, `rustls`, and `system-proxy`. |
| `rustls` | `0.23.41` | Typed invalid-certificate source inspection and local TLS test-server setup. |
| `url` | `2.5.8` | HTTP URL validation and relative redirect resolution. |
| `tempfile` | `3.27.0` | Secure create-new same-directory partial files and no-clobber persistence. |
| `rcgen` | `0.14.8` | Test-only local certificates. |

Transparent gzip, Brotli, zstd, and deflate decoding are not enabled. Cookies,
JSON helpers, multipart support, resume support, and an application async
runtime were not added.

## 4. Supported Sources and Compatibility

The resolver supports absolute `file://`, `http://`, and `https://` sources.
Unsupported schemes and malformed URLs fail before a connection or destination
publication. A valid pre-existing default-cache regular file remains
authoritative and returns before URL parsing or HTTP client construction.

The cache-key algorithm is unchanged:

- `cache: default` hashes the original URL string.
- `cache: none` hashes the artifact id followed by the original URL string.

Queries, fragments, spelling, and encoding remain part of the hash input.
Fragments are not sent in HTTP requests. Filename derivation still removes
query/fragment text, percent-decodes the final path component, and uses the
existing fallback. Protected filename outputs remain unchanged.

`cache: none` intentionally no longer reuses an existing runtime file. Every
invocation transfers again. The compatible deterministic base name is used when
free; a unique suffix is used without overwriting when it already exists.

## 5. TLS, HTTP, Redirect, and Proxy Policy

Production HTTPS uses Reqwest's Rustls verifier with normal certificate-chain
and hostname validation. There is no insecure verifier, invalid-certificate
acceptance, trust-all mode, or product custom-CA configuration. Test-only
clients may add the certificate generated for their isolated local server.

The client sends `User-Agent: EmuChef/0.1` and
`Accept-Encoding: identity`. Reqwest automatic redirects are disabled. Manual
redirect handling permits HTTP-to-HTTPS and same-security redirects, rejects an
HTTPS-to-HTTP target before issuing it, resolves relative locations, detects
loops, rejects unsupported target schemes, and allows at most five redirects.

Reqwest standard system proxy discovery is enabled. EmuChef adds no proxy CLI,
schema, or environment configuration. Proxy credentials and internal proxy
errors are not included in executor messages.

## 6. Timeout, Retry, and Response Policy

- Connect timeout: 15 seconds.
- Total transfer deadline: five minutes across the initial request, redirects,
  response headers, and response body.
- Automatic retries: none.
- Resume: none.
- Authored checksums: none.
- Arbitrary artifact-size cap: none.

Successful bodies stream through a 64 KiB buffer to the partial file. The
transport reads until EOF, checks the shared deadline during body processing,
uses checked byte accounting, and compares the result with `Content-Length`
when present. Premature EOF is not published. Chunked responses complete on a
successful EOF. Non-success response bodies are neither materialized nor
included in errors.

## 7. Cache and Publication Behavior

Default-cache hits require a regular non-symlink file. Direct destination
symlinks, ancestor symlink escapes, directories at file destinations, and paths
outside runtime/cache roots fail closed.

New local and network content is written to a random create-new partial in the
final directory. The resolver flushes and calls `sync_all` before publication.
It then uses `tempfile::persist_noclobber`, which provides the strongest stable
no-overwrite behavior exposed by that crate on the current platform. The
implementation does not claim one universal crash-atomic primitive across all
supported operating systems.

An existing complete default-cache file is never overwritten. If another
process publishes first, the resolver validates and uses that winner with
`cache_hit = true`. `cache: none` also never overwrites and chooses a unique
published path on collision. Cross-process locking and parallel downloading
were not added.

Partial files are explicitly closed and removed on transfer, write, sync, or
publish failure. A cleanup failure preserves the primary error and appends the
stable `artifact_partial_cleanup_failed` indication. Existing complete cache
entries are not deleted by failure handling.

## 8. Typed Errors and Redaction

The internal error model exposes these stable codes:

- `artifact_url_invalid`
- `artifact_scheme_unsupported`
- `artifact_source_not_found`
- `artifact_download_failed`
- `artifact_http_status`
- `artifact_redirect_limit_exceeded`
- `artifact_redirect_downgrade_rejected`
- `artifact_connect_timeout`
- `artifact_request_timeout`
- `artifact_tls_verification_failed`
- `artifact_response_incomplete`
- `artifact_response_too_large`
- `artifact_cache_write_failed`
- `artifact_cache_publish_failed`
- `artifact_partial_cleanup_failed`
- `artifact_sandbox_rejected`

TLS classification uses typed source-chain inspection and only selects the TLS
code for a typed invalid-certificate cause. Other unrecognized connection
failures remain `artifact_download_failed`; display strings are not parsed.

Executor messages may contain the artifact id, scheme, host, path, HTTP status,
or redirect count. They omit URL username/password, query and fragment,
authorization data, headers, cookies, proxy credentials, response bodies, raw
Reqwest/Rustls errors, and host environment details.

## 9. HTTP and HTTPS Test Coverage

Rust-native local tests require no public internet and cover:

- binary, empty, 512 KiB streamed, and chunked bodies;
- query-dependent bodies and URL-encoded filenames;
- exact byte preservation and compatible query/fragment cache keys;
- 301, 302, relative redirects, five allowed redirects, a rejected sixth
  redirect, loops, malformed locations, and unsupported target schemes;
- 404, 500, delayed response headers, delayed bodies, truncation, and mid-body
  connection close behavior;
- strict rejection of an untrusted certificate and a wrong-host certificate;
- successful local HTTPS with a test-only trusted root;
- HTTPS-to-HTTP downgrade rejection before contacting the HTTP target;
- immediate default-cache hits, one-request cold cache, offline warm cache,
  cache byte identity, and `cache: none` re-transfer/collision behavior;
- injected destination-write, sync, publish, and cleanup failures;
- no residual partial file after successful cleanup; and
- failed resolve steps, blocked dependents, and continuing unrelated steps.

Production timeout constants are asserted separately from short deterministic
test deadlines.

## 10. Product CLI Integration

Process-level tests invoke `CARGO_BIN_EXE_emuchef` against a temporary authored
recipe, matching profile, matching device plan, and local server. They cover:

- `emuchef validate`;
- `emuchef plan` and the unchanged execution-plan shape;
- cold `emuchef apply --dry-run` network resolution;
- warm-cache apply while the server is available;
- warm-cache apply after the server is stopped;
- HTTP 500 with nonzero exit status;
- stable redacted stderr;
- dependent blocking and unrelated-step execution; and
- absence of partial files.

No process-level test invokes real ADB.

## 11. Verification

The complete required matrix passed after commit `74a4a58`:

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all -- --check` | Passed. |
| `cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml` | Passed. |
| `cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml` | 351 passed, 7 ignored. |
| `cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets -- -D warnings` | Passed with no issues. |
| `npm run check:rust-runtime` from `apps/config-editor` | Passed, including packaging guards, Rust-only guards, typecheck, and 94 logic tests. |
| `npm run typecheck` from `apps/config-editor` | Passed. |
| `npm run test:logic` from `apps/config-editor` | 94 passed. |
| `npm run build` from `apps/config-editor` | Passed; Vite production build completed. |
| `cargo fmt --manifest-path apps/config-editor/src-tauri/Cargo.toml --all -- --check` | Passed. |
| `cargo check --manifest-path apps/config-editor/src-tauri/Cargo.toml` | Passed after preparing the ignored debug sidecar input. |
| `cargo test --manifest-path apps/config-editor/src-tauri/Cargo.toml` | 29 passed. |
| `cargo clippy --manifest-path apps/config-editor/src-tauri/Cargo.toml --all-targets -- -D warnings` | Passed with no issues. |
| `git diff --check` | Passed. |

The initial Tauri check reported that generated resource
`binaries/emuchef-aarch64-apple-darwin` did not exist. Running the maintained
`npm run sidecar:dev` prerequisite generated that ignored input; the entire
Tauri matrix then passed. The generated sidecar was removed afterward.

Repository searches found no active unsupported-network wording and no
`danger_accept_invalid`, `accept_invalid`, `insecure`, `trust_all`, or
`trust-all` configuration. Python/shadow terms remain only in the frozen
current-state statement, cumulative cleanup report, and negative guard tests;
no Python runtime or shadow planner was reintroduced.

## 12. Compatibility Fixtures

Before commit 1, SHA-256 hashes were recorded for all 61 files under
`crates/emuchef-rust-backend/tests/fixtures/compatibility_goldens_v1`. The
post-implementation manifest matched byte-for-byte with `cmp`. No fixture was
rewritten, regenerated, reformatted, or renamed.

## 13. Generated Outputs Removed

After verification, path-specific cleanup removed:

- `.emuchef_cache/` and `.emuchef_runtime/` when present;
- `crates/emuchef-rust-backend/target/`;
- `apps/config-editor/src-tauri/target/`;
- generated Tauri sidecar binaries while preserving `binaries/.gitignore`;
- `apps/config-editor/src-tauri/gen/` and `apps/config-editor/dist/`;
- repository `__pycache__/` directories and `.DS_Store` files.

`.codegraph`, app-local `node_modules`, app-local lockfiles, authored YAML,
source fixtures, compatibility fixtures, and Rust lockfiles were preserved.

## 14. Tests Not Run

- Real-device ADB apply was not run. It is destructive and requires explicit
  operator authorization and a safe device.
- The clean-cache full RetroArch remote download and apply was not run.
- The real-device warm-cache and network-unavailable warm-cache reruns were not
  run.
- Real packaged GUI validation on release targets was not run.
- No public-internet endpoint was used by automated tests.

No manual validation is claimed as passed.

## 15. Manual Runbook and Remaining Blockers

The operator runbook is
`docs/manual/real-device-retroarch-validation.md`. Release evidence must record
the exact tested commit, clean cache, full remote RetroArch resolution,
successful real-device apply and runtime checks, warm-cache rerun,
network-unavailable warm-cache rerun, identical cache bytes, and no partial
files.

Public release readiness remains blocked on:

- recorded real-device RetroArch evidence, including the network cases;
- recorded packaged GUI evidence on supported targets;
- signing and macOS notarization decisions and automation;
- updater support;
- CSP hardening; and
- cross-platform release automation and artifact inspection.
