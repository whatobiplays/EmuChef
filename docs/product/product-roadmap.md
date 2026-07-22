# EmuChef Product Roadmap

## 1. Purpose and authority

This is the single source of truth for remaining product work across the EmuChef repository.

The repository contains two separate applications:

- **EmuChef proper** (`apps/emuchef-app`): the guided end-user application for detecting an Android handheld, choosing a setup, providing required files and values, reviewing a plan, and applying or simulating that plan.
- **Config Editor** (`apps/config-editor`): the authoring application for creating and maintaining recipes, app definitions, device profiles, and related catalog content.

They share the Rust backend, but they are distinct products with different users, workflows, and release concerns. Every roadmap item must explicitly identify one owner:

- **Owner: EmuChef proper**
- **Owner: Config Editor**
- **Owner: Shared Runtime**

A shared Rust implementation does not make a feature cross-product by itself. Work belongs to the application whose workflow and acceptance criteria it serves unless it intentionally changes a shared contract used by both applications.

Detailed evidence remains in the relevant product and release documents. In particular:

- `docs/product/phase-5a-end-to-end-ux-audit.md` is the detailed UX evidence authority for EmuChef proper.
- `docs/product/config-editor-authored-generation.md` is the current-state design and implementation record for Config Editor authored generation.
- `docs/product/phase-5b-apk-verification-and-permission-automation.md` is a Config Editor generation milestone, not EmuChef proper Phase 5B.

## 2. Current status

| Product or track | Current state | Next priority |
|---|---|---|
| EmuChef proper | Phase 5A through 5F completed | Phase 5G support, diagnostics, and recovery polish |
| Config Editor | Authored generation implemented through GitHub release-pattern testing | Later refinements remain unsequenced unless explicitly promoted |
| Shared Runtime | Rust is the sole runtime and retains device, filesystem, planning, execution, validation, and protocol authority | Add shared capabilities only when required by a bounded product slice |
| Release engineering | Deliberately deferred from normal Phase 5 product work | Resume only when the owner declares the relevant application release-comfortable |

Allowed roadmap status values are `Planned`, `Next`, `In progress`, `Blocked`, `Completed`, and `Deferred`.

## 3. Product-wide working rules

1. State the owning application or shared track in every task, plan, result, and roadmap update.
2. Do not use “the app” when the distinction matters; write **EmuChef proper** or **Config Editor**.
3. Preserve Rust and Tauri authority boundaries. React remains presentation-only for device, filesystem, execution, update, external-navigation, validation, and trusted-write authority.
4. Do not move a requirement from one application to the other merely because they share backend types or protocol operations.
5. Keep implementation slices bounded, tested, documented, and reviewable.
6. Do not combine broad visual redesign with major workflow-state changes in one task.
7. Prefer evidence from the running application and existing contracts over speculative redesign.
8. Keep release engineering and explicitly post-MVP work out of ordinary product slices unless reprioritized here.
9. Update this roadmap when a phase completes, ownership changes, or scope is materially deferred.

# Part I — EmuChef Proper Roadmap

## 4. Scope

**Owner: EmuChef proper**

This section applies only to `apps/emuchef-app` and the backend/Tauri capabilities required by its end-user workflow. It does not define Config Editor authoring features.

EmuChef proper Phase 5 focuses on end-user feature completeness, usability, workflow clarity, resilience, accessibility, and visual polish. Formal public-release qualification remains deferred.

## 5. EmuChef Proper Phase Status

| Phase | Name | Status | Primary outcome |
|---|---|---|---|
| 5A | End-to-end UX and feature-gap audit | Completed | Evidence-based prioritized end-user backlog |
| 5B | Workflow navigation and state polish | Completed | Predictable movement, recovery, and state transitions |
| 5C | Recipe and setup selection experience | Completed | Nontechnical setup discovery and selection |
| 5D | Input collection and file-management polish | Completed | Early, understandable, recoverable input validation |
| 5E | Plan review and execution experience | Next | Confidence before execution and useful failure recovery |
| 5F | Saved configurations and reusable setups | Planned | Reliable reuse and maintenance of configurations |
| 5G | Support, diagnostics, and recovery polish | Planned | Troubleshooting without a terminal |
| 5H | Visual consistency and final product polish | Planned | Cohesive, release-comfortable end-user experience |

## 6. Phase 5A — End-to-End UX and Feature-Gap Audit

**Owner: EmuChef proper**  
**Status: Completed**

### Objective

Exercise the primary end-user workflows and produce an evidence-based backlog of defects, missing MVP features, and optional enhancements.

### Evidence authority

See `docs/product/phase-5a-end-to-end-ux-audit.md` for the detailed findings, reproduction steps, severity, frequency, and screenshots or exact UI-state descriptions.

### Exit criteria

- Every primary end-user workflow was exercised.
- High-friction points were documented reproducibly.
- Later Phase 5 work was selected from evidence rather than assumption.

## 7. Phase 5B — Workflow Navigation and State Polish

**Owner: EmuChef proper**  
**Status: Completed**

### Objective

Make workflow location, progression, regression, blocking, cancellation, restart, and device-transition behavior predictable.

### Completion evidence

Completed on 2026-07-19. EmuChef proper now:

- leaves startup focus neutral while retaining deliberate transition focus;
- labels the combined fourth stage `Customize`;
- separates Platform-Tools ZIP selection from validation and installation;
- reconciles Platform-Tools replacement redetection and confirms removal in user-facing terms;
- reports bounded single-flight device refresh progress;
- presents absent, unauthorized, offline, unsupported, and supported states distinctly;
- gates unsupported-device use behind explicit acknowledgement and backend-authored safe generic plans;
- retains a bounded Tauri-only serial-to-handle identity map for same-device reconnection;
- preserves portable setup, recipe, and backend-classified nonsensitive binding intent across disconnect and runtime restart while invalidating runtime authority; and
- reports recovery omissions with friendly labels or counts, never sensitive values, paths, or binding identifiers.

Manual real-device timing and packaged-GUI focus checks remain qualification follow-ups, not unfinished Phase 5B product behavior.

## 8. Phase 5C — Recipe and Setup Selection Experience

**Owner: EmuChef proper**  
**Status: Completed**

### Objective

Allow a nontechnical user to choose an appropriate setup without understanding authored recipe internals.

### Completion evidence

Completed on 2026-07-20. EmuChef proper now:

- keeps the exact backend match recommended while displaying other backend-approved applicable plans;
- provides backend-authored blank `Start from scratch` choices with an approved device profile and no initially selected recipes;
- supports search plus selected, available, and unavailable filters with visible counts;
- summarizes current selections and provides one-action recommended setup selection;
- exposes friendly capability requirements and unavailable reasons without raw capability identifiers;
- explains recipe dependencies by name and identifies dependencies automatically added by backend expansion;
- exposes backend-projected APK, BIOS, ROM/content, and network-download requirements before recipe selection; and
- revalidates selection changes through the Rust runtime while React remains presentation-only.

### Deferred authored-schema additions

The following remain deferred until authoritative authored fields exist:

- user-facing categories;
- expected download sizes; and
- experimental labels.

They must not be inferred heuristically from recipe IDs, names, URLs, or step types.

## 9. Phase 5D — Input Collection and File-Management Polish

**Owner: EmuChef proper**  
**Status: Completed**

### Objective

Catch input problems early, explain requirements in user language, and repair moved or invalid inputs without rebuilding the setup.

### Candidate scope

- Specific picker labels and expected-file descriptions.
- Accepted extensions and formats.
- Immediate backend-authoritative validation where safe.
- Preservation of nonsensitive values across navigation.
- Clear treatment of sensitive values that are not saved or recovered.
- Missing and moved-file detection.
- Guided relink workflow.
- Improved multi-file presentation.
- Duplicate, conflicting, and unsupported-file feedback.
- Drag-and-drop only if Tauri retains filesystem authority.
- Earlier visibility of BYO APK, BIOS, ROM, and content requirements.

### Exit criteria

Users understand what each input requires, errors are found before execution, and moved files can be repaired without rebuilding the setup.

### Completion evidence

Completed on 2026-07-21. EmuChef proper now:

- projects authored labels, descriptions, required/optional state, multiplicity, accepted extensions, and existing Phase 5C APK, BIOS, ROM/content, and network requirements through the authoritative `describeConfiguration` contract;
- retains accepted input contracts in the current runtime generation so Tauri, rather than React, selects picker kind, multiplicity, extension filters, and validation rules;
- validates missing, inaccessible, wrong-kind, unsupported-extension, and canonically duplicate entries before review, with per-entry sanitized diagnostics and stale-response rejection;
- reports reuse of one canonical file across active inputs as a non-blocking warning naming the user-facing fields;
- supports single and multi-file choose, add, replace, relink, remove, and clear actions without rebuilding setup or recipe selection;
- keeps device destinations as device-path text values and sensitive text in concealed, non-persisted controls;
- projects only active, explicitly nonsensitive saved bindings, retains omitted values only in backend-owned state until explicit save, and filters them from Save and Save As; and
- leaves the source configuration unchanged when sanitation is pending and the document is closed without saving.

Drag-and-drop remains deferred because the completed picker and repair workflow satisfies the phase exit criteria without expanding filesystem authority.

## 10. Phase 5E — Plan Review and Execution Experience

**Owner: EmuChef proper**  
**Status: Completed**

### Objective

Make users confident about what EmuChef proper will do and provide useful recovery after partial failure.

### Candidate scope

- Human-readable grouping by recipe and action type.
- Separate presentation of downloads, copies, installs, permissions, launches, skips, and device changes.
- Highlight destructive or irreversible actions.
- Explain automatic dependencies and skipped steps.
- Strongly distinguish simulation from real execution.
- Show current recipe, current step, overall progress, and cancellation state.
- Group completion into succeeded, failed, skipped, and needs-attention work.
- Retry only failed or retryable work through a freshly reviewed plan.
- Improve report display and export.
- Refine product-level real execution without silently broadening release qualification.

### Exit criteria

Users can understand impact before execution and determine what happened, what failed, and what action is safe afterward.

### Completion evidence

Rust emits the exact-plan-bound, feature-first review projection with authored
setup/feature/action/input presentation, deterministic populated sections,
authoritative counts and known waits, neutral sensitive-input summaries, and
fail-closed blocker state for work that cannot be projected safely. Tauri only
retains and verifies the plan/digest/generation, attaches the opaque review
handle, and sanitizes execution state; React renders those DTOs without raw
planner/runtime identity or authority data.

Execution presents localized timestamps, current authored feature/action,
authoritative completion counts, sanitized recent activity, truthful
safe-boundary cancellation copy, action-specific classified failure guidance,
fresh-review recovery, backend-authorized launch, and report export state scoped
to one execution identity. Snapshot and event polling reject stale generations
and handles, retain an independent event cursor, and prevent terminal-state
downgrade. The Phase 5E audit findings UX-019, UX-021, UX-022, UX-034, and
UX-039 through UX-042 are resolved with linked regressions; UX-020 remains open
under Phase 5H. Frontend, security, backend workspace, Tauri workspace, format,
build, and diff gates passed. This is automated source/fixture evidence only;
it does not claim real-device, packaged-GUI, signing, packaging, or release
qualification.

## 11. Phase 5F — Saved Configurations and Reusable Setups

**Owner: EmuChef proper**  
**Status: Completed**

### Objective

Make saved end-user configurations dependable without persisting generated plans or runtime authority.

### Candidate scope

- Duplicate configuration.
- Rename within EmuChef proper.
- Improved recent-file management.
- Missing-file and relink indicators.
- Configuration summary before opening.
- Clear import and export behavior.
- Catalog compatibility warnings.
- Detection of removed, renamed, or materially changed recipes.
- Comparison of saved intent with current selections.
- Save As New after modifying an existing setup.
- User-facing templates only where audit evidence supports them.

### Exit criteria

Users can maintain and reuse multiple configurations and understand when saved intent needs repair or no longer matches the current catalog.

### Completion evidence

Schema V2 stores durable portable intent plus authored-contract fingerprints and rejects runtime authority, generated plans, device identity, and sensitive values. V1 inspection is non-mutating and establishes no historical baseline until the first explicit V2 save. Rust/Tauri owns previews, comparison, bounded repair, native dialogs and menus, collision handling, atomic writes, rename/duplicate/import/export identity, and canonical-path Recents. The frontend provides sanitized pre-open summaries, concrete save disclosure, a focused management surface, and intent-based review invalidation while preserving the Inputs stage for ordinary Save and pure Save As.

The canonical findings UX-011, UX-017, UX-043, UX-044, UX-045, and UX-046 are resolved with backend, Tauri, logic, and DOM regressions. Frontend tests, typecheck, lint, production build, security and Python-retirement checks, both Rust workspace suites, both Rust format checks, and the diff whitespace gate passed. This is automated source, fixture, DOM, and build evidence only; it does not claim real-device, packaged-GUI, signing, packaging, notarization, or release qualification.

## 12. Phase 5G — Support, Diagnostics, and Recovery Polish

**Owner: EmuChef proper**  
**Status: Next**

### Objective

Allow common end-user problems to be diagnosed and corrected without a terminal.

### Candidate scope

- Actionable status summaries.
- Copyable sanitized error codes.
- Clear diagnostics-export disclosure.
- Better cache-category descriptions and deletion consequences.
- Improved recovery-draft explanations.
- Unified troubleshooting for runtime, Platform-Tools, device, catalog, cache, and update status.
- Safe corrective actions beside each issue.
- Granular Reset Local App State instead of an all-or-nothing reset.

### Exit criteria

A user can identify the failing subsystem, understand the consequence, and take a safe corrective action within EmuChef proper.

## 13. Phase 5H — Visual Consistency and Final Product Polish

**Owner: EmuChef proper**  
**Status: Planned**

### Objective

Create a cohesive, comfortable end-user experience after workflow behavior stabilizes.

### Candidate scope

- Fix the unsaved-configuration panel layout.
- Remove internal catalog IDs and implementation terminology from normal UI.
- Establish consistent typography, spacing, alignment, controls, panels, dialogs, empty states, and statuses.
- Improve long-text wrapping and narrow-window behavior.
- Review high zoom, forced colors, reduced motion, focus visibility, and disabled-state explanations.
- Normalize loading and transition behavior.
- Clean up terminology.
- Complete app icon, window title, About surface, and version display.
- Review light and dark appearance if both remain supported.

### Exit criteria

The full EmuChef proper application feels visually and linguistically coherent, and no major end-user surface appears unfinished or developer-oriented.

# Part II — Config Editor Roadmap

## 14. Scope and current state

**Owner: Config Editor**

This section applies only to `apps/config-editor` and the backend/Tauri capabilities required for authored catalog workflows. It does not change EmuChef proper Phase 5 status.

Implemented authored-generation work includes:

- typed authored app-definition and device-profile foundations;
- standard read-only connected-device profile generation;
- local APK generation;
- GitHub, GitLab, Forgejo/Codeberg, and direct HTTPS APK source generation;
- native bounded APK manifest inspection;
- reviewed package-name and optional trusted-checksum enforcement;
- permission review and optional generated permission automation;
- collision detection and trusted atomic saves; and
- GitHub latest-compatible release-pattern testing.

The authoritative current-state design is `docs/product/config-editor-authored-generation.md`.

## 15. Remaining Config Editor refinements

These items are not part of EmuChef proper Phase 5. They remain unsequenced until explicitly promoted to `Next`.

### 15.1 OS-keychain GitHub credentials

**Owner: Config Editor**  
**Status: Planned**

- Reduce unauthenticated GitHub API rate-limit friction.
- Define secure storage, access, revocation, and lifecycle behavior.
- Keep credentials out of authored documents, React state, logs, and diagnostics.
- Treat private-repository support as a separate security decision.

### 15.2 Dedicated app-definition editor

**Owner: Config Editor**  
**Status: Planned**

- Open and edit existing app definitions directly.
- Validate identifiers and authored fields through Rust.
- Preserve dirty-state, collision, canonical-YAML, and trusted-save behavior.

### 15.3 Dedicated device-profile editor

**Owner: Config Editor**  
**Status: Planned**

- Provide a first-class profile-authoring surface.
- Explain match scope, inheritance, and capability consequences.
- Preserve backend ownership of validation and persistence.

### 15.4 Obtainium import

**Owner: Config Editor**  
**Status: Planned**

- Import compatible source and update definitions.
- Normalize imported data into EmuChef-authored contracts.
- Clearly report unsupported, lossy, or ambiguous fields.

### 15.5 Source-update checks

**Owner: Config Editor**  
**Status: Planned**

- Detect whether authored remote sources have newer releases or changed assets.
- Avoid background polling unless separately designed.
- Preserve fail-closed behavior for ambiguous source changes.

### 15.6 Alias management

**Owner: Config Editor**  
**Status: Planned**

- Create and maintain package or application aliases.
- Surface collisions and stale aliases.
- Never silently change authored identity.

### 15.7 Device-plan assistance

**Owner: Config Editor**  
**Status: Planned**

- Help authors associate apps and recipes with supported device plans.
- Explain plan compatibility and required capabilities.
- Keep planner and runtime authority out of React.

### 15.8 Extended device capability checks

**Owner: Config Editor**  
**Status: Deferred**

A future explicit authoring action may test bounded shared-storage access, package-manager availability, activity-manager availability, and optional root-shell access. Root probing must never run automatically, standard capture remains read-only, and any temporary device material must be disclosed and cleaned up.

# Part III — Shared Runtime and Cross-Product Work

## 16. Shared Runtime

**Owner: Shared Runtime**

The Rust backend and trusted Tauri layers serve both applications, but shared work should be created only when a concrete product slice requires it.

Potential shared work includes:

- additive protocol contracts needed by both applications;
- authored-schema extensions with authoritative validation and canonical serialization;
- planner or executor capabilities consumed by both workflows;
- artifact resolver hardening;
- shared ADB capability modeling; and
- reusable redacted diagnostics and stable error classifications.

Shared-runtime tasks must identify:

1. which applications consume the change;
2. which application drives the acceptance criteria;
3. whether the protocol or authored schema changes;
4. compatibility and migration expectations; and
5. separate frontend verification for each affected application.

No roadmap item should be placed here merely because its implementation file lives under `crates/emuchef-rust-backend`.

## 17. Authored-schema candidates carried forward

**Owner: Shared Runtime**, with **EmuChef proper** as the initial consumer unless reprioritized.

Potential authoritative authored fields include:

- user-facing recipe categories;
- expected download sizes; and
- experimental status.

These should be designed as explicit schema extensions with Rust validation and projections. EmuChef proper must not infer them from IDs, names, URLs, or step types, and Config Editor should expose them only after the schema contract exists.

# Part IV — Release Engineering

## 18. Application-specific release boundaries

Release readiness must always name the application being qualified.

### 18.1 EmuChef proper release engineering

**Owner: EmuChef proper**  
**Status: Deferred**

- Developer ID signing and hardened-runtime release runs.
- Notarization and stapling.
- Clean-Mac qualification evidence.
- Production update endpoint, origin, and metadata-key pinning.
- Release hosting and signed metadata publication.
- Formal real-device release approval.
- Public launch checklist and release evidence archive.

### 18.2 Config Editor release engineering

**Owner: Config Editor**

Config Editor already has recorded macOS signing and notarization evidence for its qualified artifacts. Any future release work must continue to identify Config Editor explicitly and must not be used as evidence that EmuChef proper is signed, notarized, or release-qualified.

### 18.3 Cross-platform release automation

**Owner: Shared Runtime / release infrastructure**  
**Status: Deferred**

Cross-platform packaging and release automation must define separate deliverables and evidence for EmuChef proper, Config Editor, CLI/runtime artifacts, and any platform-specific prerequisites.

# Part V — Explicitly Post-MVP Unless Reprioritized

## 19. EmuChef proper post-MVP

**Owner: EmuChef proper**

- Windows and Linux packaging.
- Intel or universal macOS builds.
- In-place automatic updating.
- Execution resume after application restart.
- Device-state rollback or undo.
- Persistent execution history.
- Accounts, telemetry, cloud sync, or analytics.
- Automatic Platform-Tools updates.
- Broad multi-device support beyond deliberately supported plans.

## 20. Config Editor post-MVP

**Owner: Config Editor**

- Public distribution of Config Editor unless explicitly promoted.
- Private-repository support unless its credential and trust model is separately approved.
- Broad arbitrary-site scraping or generic crawler behavior.

## 21. Shared-runtime post-MVP

**Owner: Shared Runtime**

- Remote catalogs or remote recipe delivery.
- Runtime behavior that silently expands trust or execution authority beyond reviewed plans.

# Part VI — Tracking Procedure

## 22. Starting a product session

1. Read this roadmap.
2. State the target owner: EmuChef proper, Config Editor, or Shared Runtime.
3. Inspect git status and the most recent evidence or result for that owner.
4. Confirm the active status, objective, and exit criteria.
5. Create a bounded task that names the owning application and allowed files.
6. Do not begin a later phase or unrelated product track while unresolved blockers remain unless this roadmap is explicitly updated.

## 23. Completing a product session

1. Record implementation or audit evidence in the owner-specific document or result.
2. Update status and completion evidence here.
3. Move deferred findings to the correct application or shared track.
4. Verify that requirements were not accidentally transferred between EmuChef proper and Config Editor.
5. Keep release engineering deferred unless explicitly reprioritized.

## 24. Immediate next action

### Phase 5G — Support, Diagnostics, and Recovery Polish

**Owner: EmuChef proper**  
**Status: Next**

Begin from the objective, candidate scope, and exit criteria in Section 12 and
the open Phase 5G findings in the Phase 5A end-to-end UX audit. Preserve the
completed saved-setup, review, execution, recovery, and Rust/Tauri authority
contracts while keeping troubleshooting guidance sanitized and actionable.
