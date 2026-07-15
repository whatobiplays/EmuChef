# Phase 4B Secure Update Discovery and Manual DMG Delivery

## 1. Status and Scope

Phase 4B implements user-triggered, signed update discovery for the Apple
Silicon EmuChef desktop app. A validated newer stable release can be opened in
the user's default browser as an HTTPS DMG download. EmuChef does not download,
mount, inspect, install, copy, replace, delete, or restart the application.

Production trust is intentionally unconfigured. The source-owned production
trust document contains exactly `schemaVersion` and `configured: false`.
Consequently, production builds return a local `unconfigured` status without
constructing an HTTP client, performing DNS or proxy discovery, sending a
request, migrating network-dependent state, or opening a browser.

This phase supports only product `com.emuchef.desktop`, channel `stable`,
platform `darwin`, architecture `aarch64`. It does not authorize updating
Android SDK Platform-Tools, catalogs, BIOS/ROM content, caches, configurations,
recovery records, diagnostics, or any third-party application.

## 2. Ownership Boundary

Rust/Tauri alone owns:

1. The production trust document and fixed manifest endpoint.
2. The allowed DMG origin and path prefix.
3. The metadata-signing public key and key identifier.
4. HTTP client construction, request policy, response inspection, and bounded
   body streaming.
5. Exact fixed-JSON parsing, Ed25519 verification, target and version policy,
   expiry, and retained candidate state.
6. Immediate revalidation and the operating-system browser call.

React receives only current/latest versions, timestamps, plain-text release
notes, declared DMG size and SHA-256, optional minimum-macOS information, safe
status text, and action availability. No URL, endpoint, origin, key, key ID,
signature, raw manifest, response, filesystem path, or generic opener argument
crosses IPC. The default Tauri capability remains `core:default`; no guest
opener or updater permission is registered.

The existing Platform-Tools action remains a separate Rust-owned fixed Google
URL. It does not accept React URL input and is outside update authority. The
only update path that can open a URL is `open_update_download`, using the URL
from its retained validated candidate.

## 3. Trust Roles

The extended manifest is signed with one dedicated Ed25519 metadata key. The
key signs release identity and discovery metadata only. EmuChef does not define
a second artifact-signature format and does not use `tauri-plugin-updater`.

After browser handoff, Apple Developer ID signing, hardened runtime,
notarization, ticket stapling, and Gatekeeper are the executable trust controls.
The displayed SHA-256 and size identify the release described by the signed
metadata. They do not prove that the browser-downloaded local DMG matches,
because EmuChef never reads that file.

Fixture trust documents and keys are stored only under test fixtures and are
clearly named `test-*`. Production release policy rejects fixture key IDs and
fixture public keys even if release metadata supplies them manually. No
production private key is stored in the repository.

## 4. Fixed-JSON Manifest Contract

The signed object uses this exact field order:

1. `schemaVersion`
2. `product`
3. `channel`
4. `platform`
5. `architecture`
6. `version`
7. `publishedAt`
8. `expiresAt`
9. `notes`
10. `dmgUrl`
11. `dmgSizeBytes`
12. `dmgSha256`
13. `minimumMacosVersion`, when present
14. `metadataKeyId`

`metadataSignature` is excluded from the signed bytes and is appended last in
the delivered full manifest. The signature is 64 Ed25519 bytes represented as
128 lowercase hexadecimal characters. The public key is 32 raw Ed25519 bytes
represented as 64 lowercase hexadecimal characters.

Canonical bytes are UTF-8 without a BOM, compact JSON without insignificant
whitespace, and have no trailing newline. Rust `serde_json` serialization of
typed, declaration-ordered fields and Node `JSON.stringify` of the equivalent
explicitly ordered object are the single escaping contracts. Neither side
normalizes Unicode. A shared hexadecimal golden proves byte parity for quotes,
backslashes, control characters, non-ASCII text, an empty notes field, an empty
optional-value rejection, and omitted `minimumMacosVersion`.

Before typed deserialization, both implementations reject invalid UTF-8,
unpaired surrogates, duplicate keys, unknown fields, escaped field names, and
non-canonical number tokens. After deserialization, the complete delivered
bytes must equal the deterministic full serialization. This rejects alternate
field order, whitespace, trailing newlines, alternate escaping, and every
semantically equivalent but byte-different form. JSON is never assembled by
unescaped string concatenation.

All integer fields are non-negative and no larger than JavaScript's safe
integer limit, `9007199254740991`. Numeric strings, floats, exponent notation,
negative zero, leading zeros, and overflow are invalid. `schemaVersion` must be
`1`; `dmgSizeBytes` must be from 1 through 512 MiB inclusive.

## 5. Manifest Policy

A candidate is rejected unless all of the following hold:

1. Product, channel, platform, architecture, schema, and metadata key ID match
   the source trust policy exactly.
2. Version is stable SemVer without prerelease or build metadata. Equal and
   lower versions are validly checked but never offered as updates.
3. Timestamps use exact UTC-second form `YYYY-MM-DDTHH:MM:SSZ`.
4. Publication is no more than ten minutes in the future, expiry is later than
   publication and the current time, and validity is at most 30 days.
5. Notes are plain text, contain no NUL, and are at most 16 KiB of UTF-8.
6. SHA-256 is exactly 64 lowercase hexadecimal characters.
7. The configured DMG path prefix is an absolute normalized path ending in
   `/`, with no query, fragment, repeated slash, backslash, dot segment,
   encoded separator, or encoded dot. The DMG URL is HTTPS, contains no user
   information, query, or fragment, matches the pinned origin, begins with that
   exact segment-bounded prefix, ends in `.dmg`, and contains no traversal or
   encoded separator/dot segment. For example, `/emuchef/` permits nested paths
   below `/emuchef/` but never `/emuchef`, `/emuchef-evil/`, or `/emuchef2/`.
8. The Ed25519 signature verifies the exact canonical unsigned bytes.

`minimumMacosVersion` is signed and format-validated but informational. The UI
warns without blocking browser handoff. EmuChef does not spawn `sw_vers`, a
shell, or another process to discover the local macOS version.

## 6. Network Policy

Checks occur only after the user activates **Check for Updates**. Startup and
ordinary local use never wait for update networking.

The dedicated pinned `reqwest = "=0.13.4"` client uses default features
disabled and enables only reviewed Rustls support. Gzip, Brotli, deflate, zstd,
system-proxy, cookies, and automatic content decoding are not enabled. The
client uses `no_proxy()`, rejects all redirects, has a five-second connect
timeout, a directly supported and tested five-second read timeout, and a
15-second total request timeout. It sends only fixed `Accept: application/json`
and `Accept-Encoding: identity` headers. No credentials, cookies, application
state, preferences, React input, IPC values, CLI arguments, or environment
values influence trust or request headers.

The single manifest response must have status 200, content type
`application/json` with optional UTF-8 charset, absent `Content-Encoding` or
exact normalized `identity`, and exactly one visible valid `Content-Length`
from 1 through 64 KiB. Unrelated operational response headers are ignored.
Every visible content-length value is considered; duplicates or conflicts are
rejected. Malformed HTTP framing may instead be rejected by reqwest/hyper before
application code receives a response, which is also fail-closed behavior.

The body is streamed with an independent 64 KiB count. Empty bodies, excess
bytes, early EOF, a final count different from the accepted length, compressed
responses, cancellation errors, and timeouts are rejected with sanitized
non-fatal status. Tests use only an in-process fixture server and prove direct
connection while common proxy environment variables are set.

## 7. Candidate Lifecycle and Browser Handoff

`get_update_status` is local. `check_for_updates` performs the one manifest
request and retains the validated candidate privately in memory.
`open_update_download` accepts no URL or path. Immediately before handoff it:

1. Reserves one external-navigation activity lease.
2. Rechecks execution, cleanup, native-dialog, and synchronized frontend
   interaction blockers.
3. Revalidates the retained manifest signature policy, target, version,
   expiry, and exact URL policy against current time and source trust.
4. Releases ordinary state locks and calls the Rust-side opener while the
   navigation lease remains active.

The shared activity gate is the lock-order root. Execution start, cleanup,
native-dialog acquisition, and navigation reserve there before acquiring their
ordinary state locks. The gate mutex is not held across execution work,
filesystem cleanup, native dialogs, or the operating-system opener. Lease flags
prevent a racing operation from passing the reservation while avoiding a
deadlock if OS browser IPC blocks.

The frontend interaction signal uses a process-local UUID session and a bounded
generation from 0 through 1,000,000. A new mount rotates the session into a
blocked state. Duplicate identical updates are idempotent; stale, conflicting,
wrong-session, or overflowing updates are rejected. Teardown ends the session.
Command failure rotates to a new blocked session before resynchronizing. Rust
execution, cleanup, native-dialog, and navigation states remain authoritative
regardless of frontend state.

## 8. User Experience

The accessible Updates dialog shows current/latest version, signed plain-text
notes, declared DMG size and SHA-256, and the manual procedure:

1. Open the validated DMG address in the default browser.
2. Open the browser-downloaded DMG.
3. Drag `EmuChef.app` to Applications and replace the existing copy.
4. Relaunch manually.

The dialog explicitly states that EmuChef verified signed metadata, the browser
performs the download, and EmuChef does not verify the local DMG. It provides
keyboard containment, focus restoration, live status announcements,
reduced-motion behavior, forced-colors support, and disabled-reason text.
Offline, timeout, and malformed-service failures do not block startup or local
runtime use.

Replacing only the application bundle preserves saved configurations, recents,
recovery drafts, diagnostics, artifact cache, and managed Platform-Tools under
application data.

## 9. Release Manifest Tooling

`npm --prefix apps/emuchef-app run release:macos:update-manifest -- prepare`
accepts explicit app, DMG, URL, timestamps, notes file, optional minimum macOS
version, and output paths. It reuses the existing Phase 3E bundle and
credentialed verification functions. Release-only verification rejects ad-hoc
signatures, missing Developer ID/hardened-runtime policy, failed Gatekeeper,
notarization, or stapling checks, wrong identity/version/architecture, and a DMG
that does not contain the exact verified app tree. The DMG is mounted read-only
only by credentialed release tooling and is always detached; product code never
mounts it.

Prepare derives version, exact byte size, and SHA-256 from the supplied
artifacts, validates publication and expiry against the actual current clock,
rejects local path leakage, and atomically emits canonical unsigned JSON.
Production CLI arguments, environment, and trust cannot override the clock;
only pure-policy tests may inject a deterministic `now`. Signing occurs
externally. Finalize accepts the unsigned file and an external lowercase-hex
signature file, revalidates canonical bytes and trust, verifies Ed25519,
rejects fixture production authority, and atomically emits the full canonical
manifest. No private-key argument or repository-held production private key
exists.

Ordinary Phase 3E ad-hoc qualification does not run update-manifest tooling and
requires no update credential. Hosting, production trust pinning, credentialed
release metadata, and clean-Mac manual replacement evidence remain release
prerequisites.

## 10. Dependency and Lockfile Contract

The only Phase 4B direct Rust dependencies are exactly:

1. `reqwest = "=0.13.4"` with default features disabled and Rustls only.
2. `ring = "=0.17.14"`.
3. `time = "=0.3.53"` with parsing and formatting.

Only `apps/emuchef-app/src-tauri/Cargo.lock` may change, and only as these direct
dependencies require. No updater plugin is present.

## 11. Historical In-Place Updater Decision

The original Phase 4B in-place updater was rejected. The stock pinned Tauri
updater API could configure its client but could not expose the exact manifest
response status, headers, and bounded body before plugin deserialization while
also retaining the installer object created from that response. Parsed
`raw_json` was too late, and a separate fetch would violate single-response
identity. That blocker remains valid for secure in-place installation.

This manual-DMG design supersedes the blocker only for signed discovery and
browser handoff. It does not claim that secure in-place updating, artifact
download verification, installation, replacement, or restart was implemented.
