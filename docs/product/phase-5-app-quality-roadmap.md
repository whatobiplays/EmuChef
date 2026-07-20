# Phase 5 App Quality and MVP Readiness Roadmap

## 1. Purpose

Phase 5 focuses on the end-user application itself: feature completeness, usability, workflow clarity, resilience, and visual polish. Release engineering is intentionally deferred until the application is solid enough that the owner is comfortable releasing it.

This roadmap is the durable cross-agent tracking document for the remaining app-focused MVP work. Future agent chats should read this file before proposing the next Phase 5 task and update status, findings, and scope as work is completed.

## 2. Current Baseline

The following foundations are already implemented:

- Rust is the sole product runtime.
- Guided device, setup, input, review, simulation, completion, retry, report, saved-configuration, support, cache, accessibility, recovery, packaging, and manual update-discovery foundations exist.
- Guarded real-device execution exists but remains default-disabled.
- Production update trust remains fail-closed and unconfigured.
- Release signing, notarization, hosting, clean-Mac evidence, and formal public-release qualification are not part of Phase 5.

## 3. Working Rules

1. Prioritize user-visible product quality over release mechanics.
2. Do not broaden Rust/Tauri trust boundaries to simplify frontend work.
3. React remains presentation-only for device, filesystem, execution, update, and external-navigation authority.
4. Preserve accessibility, keyboard completeness, reduced-motion behavior, recovery, saved configurations, and existing app-data durability.
5. Prefer evidence from running the app over speculative redesign.
6. Each implementation slice must be bounded, tested, documented, and reviewable.
7. Do not combine broad visual redesign with major workflow-state changes in one task.
8. Update this roadmap after each completed phase or when scope materially changes.

## 4. Phase Status

| Phase | Name | Status | Primary outcome |
|---|---|---|---|
| 5A | End-to-end UX and feature-gap audit | Completed | Evidence-based prioritized app backlog |
| 5B | Workflow navigation and state polish | Completed | Predictable movement, recovery, and state transitions |
| 5C | Recipe and setup selection experience | Completed | Nontechnical setup discovery and selection |
| 5D | Input collection and file-management polish | Next | Early, understandable, recoverable input validation |
| 5E | Plan review and execution experience | Planned | Confidence before execution and useful failure recovery |
| 5F | Saved configurations and reusable setups | Planned | Reliable reuse and maintenance of configurations |
| 5G | Support, diagnostics, and recovery polish | Planned | Troubleshooting without a terminal |
| 5H | Visual consistency and final product polish | Planned | Cohesive, release-comfortable application experience |

Allowed status values: `Planned`, `Next`, `In progress`, `Blocked`, `Completed`, `Deferred`.

## 5. Phase 5A — End-to-End UX and Feature-Gap Audit

### Objective

Run the application as a first-time user and produce an evidence-based, prioritized list of usability defects, missing features, confusing states, and polish opportunities before broad implementation begins.

### Audit scenarios

- First launch with no Platform-Tools.
- Platform-Tools import and replacement.
- No device connected.
- Unauthorized device.
- Unsupported device.
- Supported device detection and confirmation.
- Setup and recipe selection.
- Input collection, including missing, invalid, sensitive, and moved files.
- Plan generation and review.
- Simulated execution.
- Cancellation, partial failure, retry, and completion reporting.
- Save, Save As, reopen, relink, rename, duplicate, and recent-file behavior where supported.
- Recovery after closing or terminating with dirty intent.
- Support diagnostics and cache cleanup.
- Update panel in unconfigured state.
- Narrow window, zoom, keyboard-only, screen reader, reduced motion, and forced colors.

### Required deliverables

- A repo-local findings document with reproducible observations.
- Severity and frequency classification.
- Screenshots or exact UI-state descriptions where useful.
- A prioritized backlog grouped by Phase 5B–5H.
- Explicit separation of defects, missing MVP features, and optional enhancements.
- No broad implementation changes during the audit unless required to make the app runnable.

### Exit criteria

- Every primary user workflow has been exercised.
- High-friction points are documented with reproducible steps.
- The next implementation phase is selected from evidence rather than assumption.

## 6. Phase 5B — Workflow Navigation and State Polish

### Objective

Make workflow location, progression, regression, blocking, cancellation, and restart behavior predictable.

### Candidate scope

- Safe backward navigation without unnecessary data loss.
- Clear completed, active, blocked, invalid, and stale stages.
- A predictable Start Over action.
- Confirmation only when meaningful data would be lost.
- Preservation of still-valid inputs after setup or recipe changes.
- Clear disabled reasons and next actions.
- Better refresh and redetection behavior.
- Correct handling of device disconnects, identity changes, and stale reviews.
- Prevention of duplicate or conflicting actions during async work.
- Consistent Back, Cancel, Retry, Close, and Start Over semantics.

### Exit criteria

A user can always determine where they are, what to do next, why an action is unavailable, and what data will be lost by navigating or restarting.

### Completion evidence

Phase 5B completed on 2026-07-19. The main application now:

- leaves startup focus neutral while retaining deliberate transition focus;
- labels the combined fourth stage `Customize`, with the Phase 5C stage split still deferred;
- separates Platform-Tools ZIP selection from validation and installation, reconciles replacement redetection, and confirms removal in user-facing terms;
- reports bounded single-flight device refresh progress and presents unauthorized, offline, unsupported, and supported states distinctly;
- exposes unmatched devices only through a deliberate unsupported-device acknowledgment followed by explicit selection from the backend-authored `safeGenericPlans` list;
- retains a bounded Tauri-only serial-to-handle identity map for same-device reconnection without serializing exact serials to React;
- preserves setup, recipe, and backend-classified nonsensitive binding intent across disconnect and restart while invalidating device facts, reviews, plans, executions, and prior-runtime responses; and
- uses backend recovery omissions to request restart confirmation with friendly field labels or a count, never sensitive values, paths, or binding identifiers.

Implementation is concentrated in `apps/emuchef-app/src/App.tsx`, `src/workflow.ts`, the Tauri command/handle/recovery modules, and app-local tests and tooling. Verification passed through the app-local Vitest/jsdom and Node suites, correctness-focused ESLint, TypeScript checking, production build, the full Tauri Rust suite, the full Rust backend suite, security gates, and repository diff validation. The remaining risk is manual real-device and packaged-GUI confirmation of hardware timing and native focus behavior; no such evidence is claimed by this code-level phase.

## 7. Phase 5C — Recipe and Setup Selection Experience

### Objective

Allow a nontechnical user to choose an appropriate setup without understanding authored recipe internals.

### Candidate scope

- Search and filtering.
- User-facing categories.
- Clear recipe purpose and device impact.
- Requirements visible before selection.
- Dependency and conflict explanations.
- Recommended recipes per device plan.
- Select Recommended Setup action.
- Known download-size display where available.
- Indicators for APK, BIOS, ROM/content, network, root, and experimental requirements.
- Clear behavior when a selection becomes invalid.
- User-facing setup presets where justified by audit evidence.
- Keep the exact device match recommended while also showing other backend-approved applicable setup plans.
- Provide a backend-authoritative blank `Start from scratch` setup for users who want to select recipes manually.

### Exit criteria

Users can understand what a setup does, what they must provide, and why recipes are included, blocked, or incompatible.

### Completion evidence

Phase 5C completed on 2026-07-20. The setup and recipe-selection experience now:

- keeps the exact backend match recommended while displaying other backend-approved applicable plans;
- provides backend-authored blank `Start from scratch` choices that retain an approved device profile while beginning with no selected recipes;
- supports recipe search and selected, available, and unavailable filters with visible result counts;
- summarizes the current selection and provides a one-action recommended setup selection that excludes unavailable recipes;
- exposes friendly device-capability requirements and unavailable reasons without leaking raw capability identifiers;
- explains recipe dependencies by name and identifies dependencies added automatically by backend expansion;
- exposes backend-projected APK, BIOS, ROM/content, and network-download requirements before recipe selection; and
- revalidates selection changes through the Rust runtime while preserving React as presentation-only.

Implementation is concentrated in the Rust device-matching and runtime-configuration projections, `apps/emuchef-app/src/workflow.ts`, `App.tsx`, DTOs, styles, and focused contract/DOM tests. Verification passed through the app-local tests, lint, TypeScript checking, production build, and the full Rust backend suite.

User-facing authored categories, expected download sizes, and experimental labels remain deferred because the current recipe/artifact schema does not provide authoritative fields for them. They should be introduced as an explicit authored-schema extension rather than inferred from recipe IDs, names, URLs, or step types.

## 8. Phase 5D — Input Collection and File-Management Polish

### Objective

Catch input problems early and explain them in user language.

### Candidate scope

- Specific picker labels and expected-file descriptions.
- Accepted extensions and formats.
- Immediate validation where safe.
- Preservation of nonsensitive values across navigation.
- Clear treatment of sensitive values that are not saved or recovered.
- Missing and moved-file detection.
- Guided relink workflow.
- Improved multi-file presentation.
- Duplicate, conflicting, and unsupported-file feedback.
- Drag-and-drop only if it preserves Tauri-owned filesystem authority.
- Earlier visibility of BYO APK, BIOS, ROM, and content requirements.

### Exit criteria

Users understand what each input requires, errors are found before execution, and moved files can be repaired without rebuilding the setup.

## 9. Phase 5E — Plan Review and Execution Experience

### Objective

Make users confident about what EmuChef will do and give them useful recovery options after partial failure.

### Candidate scope

- Human-readable grouping by recipe and action type.
- Separate downloads, file copies, installs, permissions, launches, skips, and device changes.
- Highlight destructive or irreversible actions.
- Explain automatic dependencies and skipped steps.
- Strong distinction between simulation and real execution.
- Current recipe, current step, overall progress, and cancellation state.
- Better completion grouping: succeeded, failed, skipped, and needs attention.
- Safe retry of failed or retryable work only.
- Clearer report display and export.
- Product-level real-execution refinements without performing release qualification.

### Exit criteria

Users can understand impact before execution and can determine what happened, what failed, and what action is safe afterward.

## 10. Phase 5F — Saved Configurations and Reusable Setups

### Objective

Turn saved configurations into a dependable daily-use feature without persisting generated plans or runtime authority.

### Candidate scope

- Duplicate configuration.
- Rename from within the app.
- Improved recent-file management.
- Missing-file and relink indicators.
- Configuration summary before opening.
- Clear import and export behavior.
- Catalog compatibility warnings.
- Detection of removed, renamed, or materially changed recipes.
- Compare saved intent with current selections.
- Save As New after modifying an existing setup.
- User-facing templates only where audit evidence supports them.

### Exit criteria

Users can maintain and reuse multiple configurations and understand when saved intent needs repair or no longer matches the current catalog.

## 11. Phase 5G — Support, Diagnostics, and Recovery Polish

### Objective

Allow common problems to be diagnosed and corrected without opening a terminal.

### Candidate scope

- More actionable status summaries.
- Copyable sanitized error codes.
- Clear diagnostics-export disclosure.
- Better cache-category descriptions and deletion consequences.
- Improved recovery-draft explanations.
- A unified troubleshooting view for runtime, Platform-Tools, device, catalog, cache, and update status.
- Safe corrective actions next to each issue.
- Granular Reset Local App State workflow rather than a destructive all-or-nothing reset.

### Exit criteria

A user can identify the failing subsystem, understand the consequence, and take a safe corrective action from within the app.

## 12. Phase 5H — Visual Consistency and Final Product Polish

### Objective

Create a cohesive, comfortable application experience after workflow behavior has stabilized.

### Candidate scope

- Typography hierarchy.
- Spacing and alignment.
- Primary, secondary, destructive, and disabled button hierarchy.
- Panel, dialog, empty-state, and status consistency.
- Long-text wrapping and narrow-window behavior.
- High zoom, forced colors, and reduced motion.
- Loading and transition consistency.
- Focus visibility and disabled-state explanations.
- Terminology cleanup.
- Removal of raw IDs, schema language, sidecar language, digests, and internal implementation terms from normal UI.
- App icon, window title, About surface, version display, and other finishing details.
- Light and dark appearance review if both are supported.

### Exit criteria

The full application feels visually and linguistically coherent and no major user-facing surface appears unfinished or developer-oriented.

## 13. Deferred Release-Engineering Track

The following remain intentionally outside Phase 5 until the owner decides the application is release-comfortable:

- Developer ID signing and hardened-runtime release runs.
- Notarization and stapling.
- Clean-Mac qualification evidence.
- Production update endpoint, origin, and metadata-key pinning.
- Release hosting and signed metadata publication.
- Formal real-device release approval.
- Cross-platform packaging and release automation.
- Public launch checklist and release evidence archive.

These items must not be silently pulled into a Phase 5 implementation task.

## 14. Explicitly Post-MVP Unless Reprioritized

- Windows and Linux packaging.
- Intel or universal macOS builds.
- In-place automatic updating.
- Execution resume after application restart.
- Device-state rollback or undo.
- Persistent execution history.
- Remote catalogs or remote recipe delivery.
- Accounts, telemetry, cloud sync, or analytics.
- Automatic Platform-Tools updates.
- Broad multi-device support beyond deliberately supported plans.
- Public distribution of the Config Editor.

## 15. Tracking Procedure for Future Agent Chats

At the start of a new app-quality session:

1. Read this roadmap.
2. Inspect current git status and the most recent relevant Phase 5 result.
3. Confirm the active phase and its exit criteria.
4. Create a bounded Codex task or audit task tied to that phase.
5. Do not begin a later phase while unresolved blockers remain in the active phase unless this document is explicitly updated.

At completion:

1. Record findings or implementation evidence in a phase-specific product document or Codex result.
2. Update the Phase Status table.
3. Note deferred findings under the appropriate later phase.
4. Keep release-engineering work deferred unless the owner explicitly changes priority.

## 16. Immediate Next Action

Begin Phase 5D with focused input-presentation and validation improvements. Start with specific picker labels, accepted extension/format guidance, and immediate backend-authoritative validation while preserving Tauri-owned filesystem authority, nonsensitive intent recovery, and the rule that sensitive values are neither persisted nor recovered.
