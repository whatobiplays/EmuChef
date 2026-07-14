# Phase 3C Accessibility and Interaction Resilience

## 1. Purpose

Phase 3C defines frontend presentation and interaction behavior for the EmuChef
end-user application. It makes the guided workflow, saved configurations,
execution results, and Support & Storage usable with keyboard navigation,
screen readers, reduced motion, forced colors, narrow desktop windows, and
200% zoom. It does not change backend authority, IPC contracts, execution
semantics, saved-file contents, diagnostics contents, cache ownership, or
release packaging.

React owns semantic markup, accessible names and relationships, focus,
announcements, and resilient empty/loading/failed states. Trusted Tauri still
owns native dialogs, paths, opaque handles, exact device identity, retained
reviews, and action authority. The Rust sidecar remains authoritative for
catalog, validation, planning, execution, progress snapshots, and cache data.

## 2. Navigation and semantics

The application provides a skip link, one stable main-content target, a page
header, setup-progress navigation, a notification region, workflow headings,
and the Support & Storage dialog. Native controls retain platform keyboard
behavior. Recipe and device-plan choices use fieldsets, while recent files,
devices, diagnostics, execution steps, cleanup outcomes, and cache entries use
explicit list or description-list semantics.

Generated recipe inputs have deterministic IDs, native labels, descriptions,
`aria-invalid` state, and diagnostic associations. A failed validation or
planning action exposes and focuses an error summary. Field-specific summary
links target the affected input; global errors remain summary text. Stable
machine codes appear only inside optional technical details.

Disabled controls reference nearby visible explanations. Success, warning,
failure, selected, dirty, unavailable, in-use, cancellation, and execution
states always include text and are never conveyed by color alone.

## 3. Dialog lifecycle

Promise-backed prompts use an explicit controller. Each request owns one
identity, one resolver, and one safe cancellation result. User action, Escape,
runtime restart, configuration replacement, app reset, component unmount, and
error-boundary activation settle the request at most once. Safe teardown never
implies Save, Discard, cleanup confirmation, or real-execution confirmation.

A second incompatible prompt is rejected with its own safe cancellation result
while the first request remains unchanged. A hidden or unmounted prompt cannot
leave an awaiting workflow suspended.

Support & Storage explicitly owns its nested cleanup request. Closing the
parent cancels the cleanup request before closing. Support cannot close while a
confirmed cleanup or diagnostics export is in a non-cancellable active phase;
Escape announces the reason and leaves the dialog open. Cleanup confirmation
initially focuses Cancel. Real-execution confirmation initially focuses its
phrase field and remains cancellable until execution start is accepted.

All modal surfaces expose an accessible title and description, contain focus,
make application background content inert, support Tab and Shift+Tab wrapping,
and close on Escape only when dismissal is safe.

## 4. Focus restoration

Modal and native-dialog focus restoration records an interaction generation.
A stale generation cannot steal focus after a newer modal or destination
transition has taken ownership.

When a surface closes, focus follows this order:

1. The recorded invoker, but only when it remains connected, visible, enabled,
   outside inert or hidden content, and explicitly focusable.
2. A transition-specific destination such as the new workflow heading or the
   first actionable error summary.
3. The surviving workflow heading or primary workflow action.
4. The main-content heading or main container.
5. The application-header Support & Storage action.

Focus never falls silently to `document.body`. Configuration replacement,
workflow-step changes, validation failures, execution completion, and runtime
restart prefer their destination headings over generic restoration. The
frontend error fallback focuses its safe heading or reload action.

## 5. Announcements and progress

Always-mounted polite and assertive live regions announce device probing,
validation, planning, saved-file outcomes, execution start and completion,
diagnostics export, cache refresh and cleanup, runtime restart, cancellation,
failures, and rejected stale responses. Messages contain only sanitized public
presentation data.

Execution progress uses the authoritative snapshot counts. Determinate progress
uses a native `progress` element. A starting operation with no total uses an
explicit indeterminate status. High-frequency polling is coalesced to phase
changes, ten-percent buckets, cancellation, and terminal results.

## 6. Visual resilience

All interactive controls have visible high-contrast focus indicators and a
minimum practical 44-pixel target. `prefers-reduced-motion` removes nonessential
motion. Forced-colors rules preserve native control and focus visibility.
Layouts collapse to one column at narrow desktop widths, allow controls to
wrap, and avoid page-level horizontal scrolling. Long cache and event
collections may scroll inside labeled focusable regions.

## 7. Failure and privacy behavior

Major surfaces keep explicit loading, empty, cancelled, stale, failed,
runtime-unavailable, and operation-locked states visible. Unexpected React
render failures activate a top-level accessible fallback, safely cancel pending
prompts, and offer a reload action. The fallback does not display or log the
raw exception, stack trace, paths, handles, serials, sidecar payloads, logs, or
configuration contents.

Phase 2 and Phase 3A/3B authority remain unchanged. Real execution stays
compile-time gated and default-disabled. Crash restoration, automatic draft
recovery, localization, telemetry, cloud services, packaging qualification,
signing, notarization, and a broad design-system rewrite are outside Phase 3C.
