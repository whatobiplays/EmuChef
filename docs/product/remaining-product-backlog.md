# Remaining Product Backlog

## 1. Purpose

This document is the durable summary of the remaining EmuChef product backlog after completion of the current Config Editor authored-generation milestones through GitHub release-pattern testing.

It separates:

1. the completed Phase 5A audit and Phase 5B workflow-state implementation;
2. subsequent main-application quality phases;
3. later Config Editor refinements;
4. deferred release engineering; and
5. explicitly post-MVP work.

The detailed evidence authority remains `docs/product/phase-5a-end-to-end-ux-audit.md`. The detailed phase definitions and current phase status remain in `docs/product/phase-5-app-quality-roadmap.md`.

## 2. Current state

- Config Editor authored-generation work is complete through:
  - remote source analysis;
  - native APK inspection;
  - permission review and automation;
  - GitHub error classification;
  - target-SDK applicability handling; and
  - GitHub release-pattern testing.
- Main application Phase 5A is complete with 48 recorded findings and an evidence-based Phase 5B-5H backlog.
- Phase 5B is complete. Portable intent, device transitions, runtime restart, Platform-Tools maintenance, refresh feedback, and unsupported-device gating now have explicit tested contracts.
- Phase 5C is selected as the next implementation phase for the dedicated scalable recipe-selection experience.

## 3. Immediate priority: execute Phase 5C

Phase 5B established the state boundaries required for safe workflow restructuring. The next product task is the bounded Phase 5C recipe and setup selection program defined in `docs/product/phase-5-app-quality-roadmap.md` and grounded in the Phase 5A findings.

### 3.1 Completed Phase 5B foundation

Phase 5B resolved the state-preservation and device-transition defects that could erase work or misrepresent device readiness:

- preserve portable setup, recipe, and input intent across runtime restart while invalidating runtime-owned authority;
- preserve still-valid portable intent across a temporary disconnect and same-device reconnect;
- distinguish absent, unauthorized, unsupported, and supported devices;
- make device refresh and Platform-Tools replacement progress explicit;
- add confirmation and plain-language consequences for Platform-Tools removal; and
- correct workflow labels and initial focus behavior.

### 3.2 Phase 5C boundary

Phase 5C may split recipe discovery from input collection, but it must preserve the completed Phase 5B contracts: backend-only generic-plan safety, explicit unsupported-device choice, portable intent across same-device reconnect and runtime restart, conservative sensitive-value omission, and generation-based rejection of stale responses.

### 3.3 Audit conclusions carried forward

The completed audit identified 48 findings across Phase 5B-5H. The most important cross-phase conclusions are:

- portable intent must be separated more consistently from device and execution authority;
- connection state, authorization state, and supported-device state need distinct user-facing models;
- recipe discovery must become a dedicated scalable stage after workflow state is stabilized;
- validation, plan review, and reports need user-facing projections instead of raw backend identifiers;
- the canonical simulation path must support a successful result;
- saved configurations need reliable in-place saving, compatibility repair, rename, and duplicate workflows; and
- maintenance, diagnostics, and visual polish should follow the primary workflow corrections rather than lead them.

## 4. Main application backlog after Phase 5A

## 4.1 Phase 5B — Workflow navigation and state polish

Status: `Completed`

Primary outcomes:

- safe backward navigation without unnecessary data loss;
- clear active, completed, blocked, stale, and invalid workflow stages;
- predictable Start Over behavior;
- confirmation only when meaningful data would be lost;
- preservation of still-valid state across setup or recipe changes;
- clear disabled reasons and next actions;
- better refresh and redetection behavior;
- correct device disconnect and identity-change handling;
- prevention of duplicate or conflicting async actions; and
- consistent Back, Cancel, Retry, Close, and Start Over semantics.

Implementation evidence is recorded in `docs/product/phase-5-app-quality-roadmap.md` and the Phase 5A audit. Step 4 is now `Customize`; the structural recipe/input split remains assigned to Phase 5C. Real-device timing and packaged-GUI focus checks remain qualification follow-ups rather than unfinished Phase 5B behavior.

## 4.2 Phase 5C — Recipe and setup selection experience

This remains the largest structural MVP redesign, but it now follows Phase 5B. The audit showed that device, restart, disconnect, and portable-intent state transitions must be stabilized before the workflow is split into dedicated recipe-selection and input stages.

Primary outcomes:

- separate recipe discovery and selection from recipe-specific input collection;
- add search and filtering;
- add categories or meaningful grouping;
- explain recipe purpose and device impact;
- show requirements before selection;
- explain dependencies and conflicts;
- support recommended recipes and a Select Recommended Setup action;
- show known download sizes where available;
- show APK, BIOS, ROM/content, network, root, and experimental requirements;
- explain when a selection becomes invalid; and
- provide scalable selected-recipe summaries.
- keep exact matches recommended without hiding other applicable backend-approved setup plans;
- provide a backend-authoritative blank `Start from scratch` setup;

## 4.3 Phase 5D — Input collection and file-management polish

Primary outcomes:

- specific picker labels and expected-file descriptions;
- accepted extension and format guidance;
- immediate validation where safe;
- preservation of nonsensitive values across navigation;
- clear handling of sensitive values that are not saved or restored;
- missing and moved-file detection;
- guided relinking;
- improved multi-file presentation;
- duplicate, conflicting, and unsupported-file feedback;
- drag-and-drop only if Tauri retains filesystem authority; and
- earlier visibility of BYO APK, BIOS, ROM, and content requirements.

## 4.4 Phase 5E — Plan review and execution experience

Primary outcomes:

- human-readable grouping by recipe and action type;
- distinct presentation of downloads, copies, installs, permissions, launches, skips, and device changes;
- clear highlighting of destructive or irreversible actions;
- explanation of automatic dependencies and skipped steps;
- strong distinction between simulation and real execution;
- visible current recipe, current step, overall progress, and cancellation state;
- better completion grouping for succeeded, failed, skipped, and needs-attention work;
- safe recovery of failed or retryable work through a fresh reviewed plan;
- clearer report display and export; and
- product-level real-execution refinements without broadening release qualification.

## 4.5 Phase 5F — Saved configurations and reusable setups

Primary outcomes:

- duplicate configuration;
- rename from within the application;
- improved recent-file management;
- missing-file and relink indicators;
- configuration summary before opening;
- clear import and export behavior;
- catalog compatibility warnings;
- detection of removed, renamed, or materially changed recipes;
- comparison of saved intent with current selections;
- Save As New after modifying an existing setup; and
- user-facing templates only where audit evidence justifies them.

## 4.6 Phase 5G — Support, diagnostics, and recovery polish

Primary outcomes:

- actionable status summaries;
- copyable sanitized error codes;
- clear diagnostics-export disclosure;
- better cache-category descriptions and deletion consequences;
- improved recovery-draft explanations;
- a unified troubleshooting view for runtime, Platform-Tools, device, catalog, cache, and update status;
- safe corrective actions next to each issue; and
- granular Reset Local App State behavior instead of an all-or-nothing reset.

## 4.7 Phase 5H — Visual consistency and final product polish

Primary outcomes:

- fix the unsaved-configuration panel layout;
- remove internal catalog IDs and implementation terminology from normal UI;
- establish consistent typography, spacing, and alignment;
- establish consistent primary, secondary, destructive, and disabled button hierarchy;
- align panels, dialogs, empty states, and status presentation;
- improve long-text wrapping and narrow-window behavior;
- review high zoom, forced colors, and reduced motion;
- improve loading and transition consistency;
- improve focus visibility and disabled-state explanations;
- clean up terminology;
- complete app icon, window title, About surface, and version display; and
- review light and dark appearance if both remain supported.

## 5. Config Editor later refinements

Core Config Editor authored-generation work is substantially complete. Remaining documented refinements are:

### 5.1 OS-keychain GitHub credentials

- avoid unauthenticated GitHub API rate-limit friction;
- define a secure storage and lifecycle model;
- keep credentials out of authored documents and logs; and
- treat private-repository support as a separate security decision rather than an automatic consequence.

### 5.2 Dedicated app-definition editor

- open and edit existing app definitions directly;
- validate identifiers and authored fields through the Rust backend; and
- preserve dirty-state and collision behavior.

### 5.3 Dedicated profile editor

- provide a first-class profile-authoring surface;
- explain profile scope and inheritance; and
- preserve backend ownership of validation and persistence.

### 5.4 Obtainium import

- import compatible source/update definitions;
- normalize imported data into EmuChef-owned contracts; and
- clearly report unsupported or ambiguous fields.

### 5.5 Source-update checks

- detect whether authored remote sources have newer releases or changed assets;
- avoid background polling unless separately designed; and
- preserve fail-closed behavior for ambiguous source changes.

### 5.6 Alias management

- support creation and maintenance of package or application aliases;
- surface collisions and stale aliases; and
- avoid silently changing authored identity.

### 5.7 Device-plan assistance

- help authors associate apps and recipes with supported device plans;
- explain plan compatibility and required capabilities; and
- avoid moving runtime or planner authority into the frontend.

Release-pattern testing is complete and is no longer part of this later-refinement list.

## 6. Deferred release-engineering track

The following remain intentionally deferred until the application is considered release-comfortable:

- Developer ID signing;
- hardened-runtime release runs;
- notarization and stapling;
- clean-Mac qualification evidence;
- production update endpoint, origin, and metadata-key pinning;
- release hosting and signed metadata publication;
- formal real-device release approval;
- cross-platform packaging and release automation; and
- public launch checklist and release evidence archive.

These items must not be silently pulled into normal Phase 5 implementation work.

## 7. Explicitly post-MVP unless reprioritized

- Windows and Linux packaging;
- Intel or universal macOS builds;
- in-place automatic updating;
- execution resume after application restart;
- device-state rollback or undo;
- persistent execution history;
- remote catalogs or remote recipe delivery;
- accounts, telemetry, cloud sync, or analytics;
- automatic Platform-Tools updates;
- broad multi-device support beyond deliberately supported plans; and
- public distribution of the Config Editor.

## 8. Sequencing rules

1. Preserve the completed Phase 5B state and authority contracts throughout the Phase 5C structural recipe-selection redesign.
2. Keep Phase 5C implementation slices bounded by finding clusters.
3. Preserve the audit document as the detailed evidence authority and update finding status as fixes land.
4. Keep Config Editor refinements separate from main-application UX work unless a task has a clear cross-surface dependency.
5. Keep release engineering deferred until the owner explicitly changes priority.
6. Keep post-MVP work out of MVP tasks unless explicitly reprioritized.

## 9. Immediate next action

Start Phase 5C with the dedicated recipe-selection stage and scalable discovery model. Keep recipe selection separate from recipe-specific inputs, retain explicit backend-authored recommendations and availability, and do not weaken the Phase 5B reconnect, restart, unsupported-device, or stale-response guarantees.
