# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

The products are delivered as native macOS/Tauri desktop applications with
React presentation layers. The current supported packaging focus is Apple
Silicon macOS.

## Users

### EmuChef proper

Nontechnical Android handheld owners who need to configure one device safely
and reproducibly. They connect a device, confirm detected facts, choose an
appropriate setup, provide required files and values, review the resulting
plan, and simulate or—when explicitly enabled and qualified—apply it.

### Config Editor

Technical authors who create and maintain recipes, application definitions,
device profiles, and related catalog content used by EmuChef proper.

## Product Purpose

EmuChef proper guides a user through planning and applying a reproducible
configuration to a supported Android handheld while making the planned impact
and any required recovery action understandable. Ordinary builds are
simulation-first so users can inspect behavior without changing a real device.

Config Editor provides the authoring workflow that maintains the catalog and
configuration data consumed by EmuChef proper.

The shared Rust runtime owns validation, planning, execution, filesystem and
device authority, and the sidecar protocol. The two applications remain
separate products with distinct users and workflows.

## Positioning

The product combines authored, validated configuration content with a
target-bound, reviewable execution plan. It preserves a strict trust boundary:
the trusted Rust/Tauri runtime retains device, filesystem, planning, execution,
update, and external-navigation authority while React presents sanitized data.
This makes reproducible handheld setup approachable to nontechnical users
without turning the frontend into a source of runtime authority.

## Operating Context

EmuChef proper is used on macOS with one connected Android handheld at a time.
The workflow includes device discovery, device confirmation, setup and recipe
selection, required input collection and repair, plan review, simulated
execution, reporting, saved portable configurations, support diagnostics, and
recovery of unsaved intent. Android SDK Platform-Tools is a user-supplied
prerequisite and may be imported through the native application flow.

Config Editor is used to author and validate the catalog and to generate or
maintain the content that defines supported device setups. Both applications
launch the shared Rust `emuchef --sidecar` runtime and do not share frontend
modules or runtime state.

## Capabilities and Constraints

- Rust is the sole product runtime; Python is not a product, development,
  testing, packaging, or release prerequisite.
- EmuChef proper supports a guided end-user workflow for one device, saved
  portable configurations, simulation, bounded diagnostics, local recovery,
  and user-triggered manual update discovery.
- Real-device execution is feature-gated, development-only by default, and
  requires separate qualification and release approval. Execution has no
  rollback or device-state undo.
- EmuChef does not bundle, download, proxy, or automate license acceptance for
  Android SDK Platform-Tools.
- React receives sanitized projections and opaque handles. Exact serials,
  executable paths, catalog roots, full plans, credentials, and runtime
  authority must remain outside frontend state, markup, logs, and storage.
- Saved configurations retain portable intent, not generated plans, device
  facts, serials, review authority, execution authority, or sensitive values.
- Product work distinguishes EmuChef proper, Config Editor, and Shared Runtime;
  a shared backend implementation does not merge product ownership.
- Confirmed open product decisions include public release qualification,
  production trust configuration for manual updates, and the eventual scope
  of supported root-only real-device execution.

## Brand Commitments

The product name is EmuChef. User-facing language should be understandable to
nontechnical end users and should avoid exposing internal catalog identifiers,
protocol details, filesystem roots, exact device serials, or implementation
terminology where a human-facing explanation is available.

## Evidence on Hand

Product and workflow evidence is maintained in the repository, including:

- `README.md`
- `docs/product/product-roadmap.md`
- `docs/product/phase-1-read-only-app.md`
- `apps/emuchef-app/README.md`
- `apps/config-editor/README.md`
- the phase contracts and release evidence under `docs/product/` and
  `docs/release/evidence/`

The repository contains implementation and automated verification evidence for
the documented workflows. It does not by itself establish public-release
qualification, Developer ID/notarization readiness for every product surface,
or broad real-device coverage; future work must not fabricate those claims.

## Product Principles

1. Make reproducible device configuration understandable and reviewable before
   any execution.
2. Keep trusted runtime authority in Rust/Tauri and expose only the minimum
   sanitized information needed by the frontend.
3. Prefer safe, bounded, recoverable workflows over hidden mutation or implied
   authority.
4. Keep EmuChef proper and Config Editor distinct while sharing only deliberate
   runtime contracts.
5. Treat accessibility, truthful status, and actionable recovery as core
   product behavior.

## Accessibility & Inclusion

EmuChef proper is intended to be keyboard-complete and screen-reader
operable, with semantic landmarks and collections, accessible validation and
progress, bounded live announcements, focus-contained dialogs, deterministic
focus fallback, reduced-motion support, forced-colors support, and resilience
at narrow window sizes and increased zoom. These presentation requirements do
not move authority out of Rust or trusted Tauri.
