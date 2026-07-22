# Phase 5A End-to-End UX and Feature-Gap Audit

## 1. Status

- Phase: `5A`
- Status: `Completed`
- Started: `2026-07-14`
- Product surface: `apps/emuchef-app`
- Audit authority: observed application behavior is authoritative; code inspection may explain an observation but does not replace running the workflow.
- Implementation boundary: this phase records evidence and priorities only. Product behavior changes are out of scope unless the app cannot be launched or audited without a narrowly documented repair.

## 2. Objective

Exercise EmuChef as a first-time end user and produce a reproducible, prioritized backlog of:

1. defects;
2. missing MVP features; and
3. optional enhancements.

The resulting backlog must be grouped into Phases 5B through 5H so subsequent implementation work is selected from observed evidence rather than assumption.

## 3. Audit environment

Record the exact environment for each audit run.

| Field | Value |
|---|---|
| Git commit | Pending |
| Git branch | Pending |
| macOS version | Pending |
| Mac architecture | Pending |
| App launch method | Pending |
| Node version | Pending |
| npm version | Pending |
| Rust version | Pending |
| Tauri CLI version | Pending |
| Platform-Tools state at first launch | Pending |
| Connected device state | Pending |
| Display scale and resolution | Pending |
| Assistive technology used | Pending |

## 4. Classification model

### 4.1 Finding type

- `Defect`: implemented behavior is broken, inconsistent, misleading, inaccessible, unsafe, or materially different from the documented product contract.
- `Missing MVP feature`: required end-user capability is absent or incomplete enough to block a credible MVP workflow.
- `Enhancement`: useful improvement that is not required for MVP correctness or core usability.

### 4.2 Severity

- `S0 — Audit blocker`: the application cannot be launched or the audit cannot continue. Use only when no practical workaround exists.
- `S1 — Critical`: data loss, unsafe authority behavior, unrecoverable workflow failure, severe accessibility failure, or a primary workflow cannot be completed.
- `S2 — Major`: substantial confusion, repeated friction, incorrect state, poor recovery, or a primary workflow is difficult but still completable.
- `S3 — Minor`: localized usability, wording, layout, feedback, or consistency problem with a clear workaround.
- `S4 — Polish`: visual or interaction refinement with low task impact.

### 4.3 Frequency

- `Always`: reproduced every attempt under the stated preconditions.
- `Common`: reproduced in most attempts or affects a normal path.
- `Occasional`: reproduced intermittently or under a plausible secondary path.
- `Rare`: requires an unusual but supported state.
- `Unknown`: observed once or not yet repeated.

### 4.4 Evidence level

- `Observed`: reproduced in the running application.
- `Observed + code-supported`: reproduced in the application and traced to relevant implementation or contract details.
- `Code-supported only`: identified through inspection but not yet reproduced. This cannot by itself satisfy Phase 5A exit criteria.

## 5. Audit protocol

For each scenario:

1. Begin from the documented precondition.
2. Record exact user actions, not inferred internal calls.
3. Record the expected user-facing result.
4. Record the actual result, including visible text and control state.
5. Capture a screenshot when layout, visual hierarchy, focus, disabled state, or wording is material.
6. Repeat defects enough to assign frequency honestly.
7. Record whether a workaround exists and whether it loses entered intent.
8. Classify the finding by type, severity, frequency, evidence, and proposed Phase 5B–5H owner.
9. Do not implement the finding during this audit unless it is an `S0` audit blocker.

Sensitive values, exact device serials, private filesystem paths, credentials, and raw diagnostics must not be added to this document or screenshots.

## 6. Scenario matrix

Status values: `Not run`, `Running`, `Passed`, `Findings`, `Blocked`, `Not applicable`.

| ID | Scenario | Required states or variants | Status | Finding IDs | Evidence notes |
|---|---|---|---|---|---|
| A01 | First launch with no Platform-Tools | Clean app data; no managed Platform-Tools; no debug ADB override | Findings | UX-004 | Runtime badge exposes internal catalog identity. |
| A02 | Platform-Tools import | Valid current macOS ZIP; cancellation; invalid ZIP; unsupported/older ZIP where available | Findings | UX-002, UX-003, UX-025, UX-026, UX-027 | Valid import, cancellation, non-ZIP filtering, unrelated ZIP rejection, persistence, and same-version replacement passed. Setup copy is overly technical; picker-open state prematurely says `Validating...`; replacement lacks in-progress wording. |
| A03 | Platform-Tools replacement and removal | Existing valid managed installation; failed replacement preservation; successful replacement | Findings | UX-028, UX-029 | Cancellation, failed-replacement preservation, successful same-version replacement, and persistence passed. Successful replacement showed a false disconnect warning; removal had no confirmation and used internal authority terminology. |
| A04 | No device connected | Runtime/catalog available; device list empty; refresh/redetection | Findings | UX-002, UX-003, UX-030, UX-031 | Empty state is clear, but refresh has no visible progress or completion feedback and the screen does not explain USB debugging or authorization. Relaunch again reproduced the heading focus outline and false unexpected-session warning. |
| A05 | Unauthorized device | One ADB device awaiting authorization | Findings | UX-032 | Unauthorized devices are omitted from the list and presented as if no device is connected; authorizing and refreshing transitions cleanly to available. |
| A06 | Unsupported device | One connected device that matches no supported plan | Findings | UX-033 | Unsupported device is shown as available, selecting it skips the Device stage, and Setup exposes `Match confidence: none` instead of a clear unsupported-device explanation. |
| A07 | Supported device detection and confirmation | Supported target; disconnect/reconnect; identity change where practical | Findings | UX-002 | Initial workflow heading receives unexpected focus on window appearance. |
| A08 | Setup and recipe selection | Recommended path; manual selection; incompatible/dependent/conflicting recipes | Findings | UX-014, UX-015, UX-034 | Recipe selection, deselection, multiple selections, Back/forward preservation, and plan generation generally worked. Selecting a recipe with required inputs immediately shows blocking errors with internal binding IDs; the review exposes raw capability labels, an absolute host path, and a plan digest. |
| A09 | Input collection | Missing, invalid, sensitive, moved, multi-file, and optional values | Findings | UX-014, UX-015, UX-016, UX-035, UX-036, UX-037 | Optional RetroArch config omission, Back/forward persistence, and re-selection value restoration worked. Selected file values cannot be cleared; deleting a selected file produces a generic review failure; device destination editing is blocked and uses an inappropriate host Browse control; deselecting Copy ROM library leaves stale bindings that trigger validation errors. |
| A10 | Plan generation and review | Valid setup; invalidated/stale review; back navigation | Findings | UX-034, UX-038 | Plan generation and regeneration generally worked, but the normal review surface exposes raw bindings, host paths, capability labels, action kinds, technical IDs, and the plan digest. Disconnecting safely returns to Connect, but reconnecting the same device discards setup and input state. |
| A11 | Simulated execution | Successful run; progress; cancellation; completion | Findings | UX-019, UX-022, UX-039 | Progress, queued/running/succeeded states, scrolling, cooperative cancellation, and simulation labeling worked. Repeated runs could not reach a successful terminal result because dry-run verification failed on expected simulated output. |
| A12 | Partial failure and retry | Retryable and non-retryable outcomes where supported | Findings | UX-021, UX-022, UX-040, UX-041 | Failed and blocked work are visually distinct, but raw verifier/dependency codes remain visible and summary messages are generic. `Retry failed work` returns to Inputs for a fresh plan rather than retrying; `Return to Review` exposes the same prior plan without a strong stale-plan warning. Export succeeded and produced a sanitized structured report. |
| A13 | Completion report | Succeeded, failed, skipped, needs-attention, export/display behavior | Findings | UX-019, UX-020, UX-021, UX-022, UX-039, UX-042 | Failed and cancelled report summaries and exports worked, but raw verifier/dependency terms and timestamps remain visible, failure cards are difficult to scan, the successful-report variant is blocked by UX-039, and `Report saved` persists across a later cancelled or re-executed run. |
| A14 | Save and Save As | New unsaved setup; overwrite prompts; naming; cancelled native dialogs | Findings | UX-001 | Unsaved-configuration action panel has a visibly broken and difficult-to-scan layout. |
| A15 | Reopen and recent files | Valid recent file; missing file; malformed file; stale catalog references | Findings | UX-043, UX-044 | Saving from Inputs unexpectedly returns to Connect. Missing recent files are detected and can be relinked or removed. Malformed/incompatible configurations remain selectable and proceed into the workflow before surfacing raw catalog IDs and internal diagnostics. |
| A16 | Rename and duplicate saved configuration | Where supported; verify identity and dirty-state behavior | Findings | UX-045 | No first-class rename or duplicate action exists. `Save As...` can create another file, but identity shown in the UI comes from the YAML name rather than the filename, so externally renamed or duplicated files are not automatically distinguishable. |
| A17 | Relink moved inputs | Saved configuration with moved or missing file/directory bindings | Findings | UX-015, UX-016, UX-036, UX-046 | Both missing inputs were detected and could be resolved independently. Relinking one left the other unresolved as expected. Errors still use internal binding identifiers, the device destination retains an inappropriate Browse control, and successful Save is paired with misleading disabled-state guidance. |
| A18 | Dirty-intent close and crash recovery | Normal close, forced termination, Restore, Discard, Not now, sensitive re-entry | Findings | UX-003 | Normal Cmd+Q is reported as an unexpected prior shutdown on next launch. |
| A19 | Support diagnostics | Runtime available/unavailable; export success/cancel/failure; disclosure clarity | Findings | UX-047 | Diagnostics disclosure was clear and two exported archives were sanitized as promised. Reopening Support & Storage later retained `Success: diagnostics saved.` from a previous export, so the modal presents stale operation state. |
| A20 | Cache inventory and cleanup | Empty cache; removable entry; in-use entry; cleanup cancellation/failure | Findings | UX-006, UX-007 | Cache refresh leaves stale success notices visible; technical details expose internal result codes. |
| A21 | Update panel | Production trust unconfigured; repeated check; external-navigation state | Not applicable | — | This build correctly reports that update discovery is not configured. `Check for Updates` produces no actionable result and the DMG button is disabled because release packaging, signed metadata, and the validated download address are not yet configured. Full update behavior remains deferred to release engineering. |
| A22 | Narrow window and high zoom | Minimum window; 200% zoom; long text; wrapping; no page-level horizontal scroll | Passed | — | At the minimum practical window width, navigation reflowed into two columns, System Status moved below the main task, controls remained reachable, and Support & Storage scrolled internally without horizontal page scrolling. A 200% zoom control was not available in the packaged app, so that subtest could not be exercised. |
| A23 | Keyboard-only workflow | Full primary workflow; dialogs; native-dialog return; visible focus; skip link | Findings | UX-002 | Initial focus appears on the workflow heading without user navigation. |
| A24 | Screen-reader workflow | Headings, landmarks, fieldsets, summaries, live announcements, dialogs | Not applicable | — | Skipped by product decision; dedicated screen-reader testing is outside the current audit scope. |
| A25 | Reduced motion | OS preference enabled; transitions and progress remain understandable | Not applicable | — | Skipped by product decision; dedicated accessibility-preference testing is outside the current audit scope. |
| A26 | Forced colors / increased contrast | Supported browser/WebView and macOS contrast settings | Not applicable | — | Skipped by product decision; dedicated accessibility-preference testing is outside the current audit scope. |
| A27 | Runtime restart and stale async responses | Restart during safe idle state and during pending frontend work where supported | Findings | UX-048 | Restarting the runtime always returns the app to Connect, clears active device/setup/input/review state, and requires reopening a saved portable configuration. Generated plans are invalidated safely, but unsaved portable intent is lost without a confirmation or preservation path. |
| A28 | Start Over and backward navigation | Every workflow stage; dirty and clean intent; expected preservation/loss | Passed | — | New/Start Over, Back navigation, stale-plan invalidation, completed/failed simulation return paths, and unsaved-change protection all behaved as expected across the tested states. |

## 7. Findings

Add one subsection per finding using the template below. Finding IDs are sequential: `UX-001`, `UX-002`, and so on.

### Finding template

#### UX-000 — Concise user-facing problem statement

- Type: `Defect | Missing MVP feature | Enhancement`
- Severity: `S0 | S1 | S2 | S3 | S4`
- Frequency: `Always | Common | Occasional | Rare | Unknown`
- Evidence: `Observed | Observed + code-supported | Code-supported only`
- Scenario(s): `A00`
- Proposed phase: `5B | 5C | 5D | 5E | 5F | 5G | 5H`
- Status: `Open | Needs reproduction | Deferred | Resolved | Resolved as audit blocker`

**Preconditions**

1. ...

**Reproduction**

1. ...

**Expected**

...

**Actual**

...

**User impact**

...

**Workaround**

...

**Evidence**

- Screenshot: `Pending | path or description`
- Exact UI state/text: ...
- Related contract/code, when useful: ...

**Notes for later implementation**

...

### UX-001 — Unsaved-configuration panel has a broken horizontal layout

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Unknown`
- Evidence: `Observed`
- Scenario(s): `A14`
- Proposed phase: `5H`
- Status: `Open`

**Preconditions**

1. Launch EmuChef with the main workflow available.
2. No saved configuration is currently open.

**Reproduction**

1. Open the main application window.
2. Inspect the `Unsaved configuration` panel at the top of the workflow.

**Expected**

The configuration status, New/Open/Save/Save As actions, disabled-state explanations, and runtime-restart action should form a compact, aligned, easily scanned control group. Explanatory text should be visually associated with the control it describes without creating large unused horizontal gaps.

**Actual**

The panel spreads content across the full width in disconnected columns. The `New`, `Open`, `Save`, and `Save As` buttons form one narrow vertical stack, while the explanations for Save and Save As appear far to the right with large empty gaps. `Restart runtime` is visually detached below the button stack. The overall layout looks incomplete or structurally broken.

**User impact**

The panel is difficult to parse and makes it unclear which explanatory text belongs to which action. This creates avoidable friction in a prominent first-use area and makes the application appear unfinished.

**Workaround**

The controls remain usable, but the user must infer the relationship between each disabled action and the distant explanation text.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the full main window and unsaved-configuration panel.
- Exact UI state/text: `Unsaved configuration`, `New`, `Open...`, `Save`, `Save As...`, `Save requires a selected device plan and unsaved portable changes.`, `Save As requires a selected device plan and no other active operation.`, and `Restart runtime`.
- Related contract/code, when useful: Pending later implementation inspection.

**Notes for later implementation**

Treat this as a layout and hierarchy correction, not a workflow-state redesign. Likely remedies include grouping each action with its disabled reason, reducing excessive horizontal distribution, and visually separating configuration actions from runtime recovery actions.

### UX-002 — Workflow heading receives unexpected initial focus when the window appears

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Unknown`
- Evidence: `Observed`
- Scenario(s): `A02`, `A04`, `A07`, `A23`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Launch EmuChef into the Connect stage with a supported device visible.
2. The main window appears after startup or restoration handling.

**Reproduction**

1. Launch the application.
2. Do not press Tab or otherwise move keyboard focus.
3. Observe the `Choose an Android device` heading when the main workflow appears.

**Expected**

Initial focus should either remain on the window in a neutral state or move to a deliberate first actionable control or required recovery surface. A static workflow heading should not appear selected unless focus movement is intentionally required and clearly supports the current interaction.

**Actual**

`Choose an Android device` receives a prominent focus rectangle immediately when the window appears, making the heading look like a selected text box or active editable control.

**User impact**

The initial state is visually confusing and may cause keyboard and assistive-technology users to believe the heading is interactive. It also makes the app appear to have selected an arbitrary element before the user has acted.

**Workaround**

Press Tab or click another control to move focus.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing a focus rectangle around `Choose an Android device` immediately after window appearance.
- Exact UI state/text: `Choose an Android device` is outlined before user keyboard navigation.
- Related contract/code, when useful: Phase 3C requires deterministic focus behavior and says workflow transitions may prefer destination headings, but the observed startup presentation appears indistinguishable from an editable selected field and needs validation against intended startup focus semantics.

**Notes for later implementation**

Confirm whether startup intentionally focuses the workflow heading or whether stale recovery/focus-restoration logic is firing. Preserve accessible destination focus after meaningful transitions, but avoid an unexplained initial focus target on ordinary window appearance.

#### UX-003 — Normal Cmd+Q termination is reported as an unexpected shutdown

- Type: `Defect`
- Severity: `S2`
- Frequency: `Unknown`
- Evidence: `Observed`
- Scenario(s): `A02`, `A04`, `A18`
- Proposed phase: `5G`
- Status: `Open`

**Preconditions**

1. Launch EmuChef normally.
2. Allow startup to complete.
3. Do not force-terminate the process.

**Reproduction**

1. Quit EmuChef using `Cmd+Q`.
2. Launch EmuChef again.

**Expected**

A normal application quit is recorded as a clean session end. The next launch should not display an interrupted-session or unexpected-shutdown warning unless the prior process actually terminated abnormally.

**Actual**

The next launch displays: `Attention: The previous session ended unexpectedly. Execution was not resumed.` even though the prior session was closed using the normal macOS Quit command.

**User impact**

The warning is a false positive and undermines confidence in crash recovery and execution-state reporting. Users may believe work was lost or an execution was interrupted when the application exited normally.

**Workaround**

Dismiss or ignore the warning. No reliable clean-quit workaround has been established.

**Evidence**

- Screenshot: the startup screenshot captured for UX-001 and UX-002 includes the unexpected-session warning.
- Exact UI state/text: `Attention: The previous session ended unexpectedly. Execution was not resumed.`
- Related contract/code, when useful: Phase 3D requires clean-session termination to be distinguished from an interrupted prior process.

**Notes for later implementation**

Inspect the Tauri/macOS application-exit lifecycle and interrupted-session marker cleanup. The fix should distinguish normal Quit, window close followed by app termination, controlled runtime restart, and actual process interruption without weakening crash detection.

#### UX-004 — Runtime status badge exposes an internal catalog identifier

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Unknown`
- Evidence: `Observed`
- Scenario(s): `A01`
- Proposed phase: `5H`
- Status: `Open`

**Preconditions**

1. Launch EmuChef normally.
2. Allow the Rust runtime and bundled catalog to initialize.

**Reproduction**

1. Open the main application window.
2. Inspect the centered status badge in the application header.

**Expected**

The header should communicate only useful end-user status, such as that EmuChef is ready. Internal catalog identities or implementation-version labels should be omitted from the normal interface or confined to an optional technical-details surface.

**Actual**

The badge displays `Runtime ready · phase1-bundled-1`. The `phase1-bundled-1` portion is an internal bundled-catalog identity and is not meaningful to an end user.

**User impact**

The label is confusing, visually noisy, and makes the application appear unfinished or developer-oriented. Users cannot infer whether it represents the app version, runtime version, catalog version, release channel, or an error state.

**Workaround**

Ignore the identifier.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing `Runtime ready · phase1-bundled-1` in the application header.
- Exact UI state/text: `Runtime ready · phase1-bundled-1`.
- Related contract/code, when useful: The end-user app documentation describes catalog identity as trusted runtime metadata, but Phase 5H explicitly calls for removal of raw IDs and internal implementation terms from normal UI.

**Notes for later implementation**

Keep the readiness signal but replace the badge with user-facing text such as `Ready`, or remove it entirely if readiness is already clear from the workflow. Preserve the catalog identity only in Support & Storage, diagnostics, or another explicitly technical details surface when it is useful for troubleshooting.

### UX-005 — Application uses the wrong icon

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A01`
- Proposed phase: `5H`
- Status: `Open`

**Preconditions**

1. Launch the EmuChef macOS application.
2. Compare the displayed application icon with the owner-approved EmuChef icon asset.

**Reproduction**

1. Launch EmuChef.
2. Inspect the application icon shown by macOS and any in-app branding surface that reuses the packaged icon.

**Expected**

The application should use the approved EmuChef icon consistently in the app bundle, Dock, Finder, window/application presentation, and other product surfaces that consume the packaged icon.

**Actual**

The running application uses an icon that does not match the icon previously supplied and approved for EmuChef.

**User impact**

The product presents incorrect branding and appears unfinished or disconnected from the approved visual identity.

**Workaround**

None for end users.

**Evidence**

- Exact observation: owner reports that the app icon is not the supplied EmuChef icon.
- Related configuration: the Tauri bundle currently references generated `icon.icns` and `icon.png` assets; those assets or their generation source need to be compared with the approved icon.

**Notes for later implementation**

Replace the packaged icon source with the approved master asset, regenerate all required macOS/Tauri icon sizes without altering the artwork, and verify Finder, Dock, application bundle, DMG, and window presentation after clearing macOS icon caches where necessary.

#### UX-006 — Refresh leaves stale cache-operation notifications visible

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Unknown`
- Evidence: `Observed`
- Scenario(s): `A20`
- Proposed phase: `5G`
- Status: `Open`

**Preconditions**

1. Open `Support & Storage` and view the artifact cache.
2. Have one or more removable cache entries.

**Reproduction**

1. Remove cached artifacts.
2. Confirm the cache reports `0 entries · 0 B` and shows the empty state.
3. Click `Refresh`.
4. Observe the prior cleanup-success notifications.

**Expected**

Refreshing the cache should reload the current cache state and clear completed operation notifications that no longer provide actionable information. At most one recent result should remain when it is still useful.

**Actual**

Multiple `Success: The cache entry was removed.` notifications remain stacked after the cache is empty and after Refresh is used.

**User impact**

The stale messages dominate the dialog, imply that old operations are still relevant, and make the current empty-cache state harder to scan. Repeated cleanup actions can create an increasingly noisy support surface.

**Workaround**

Close and reopen the Support & Storage dialog or ignore the stale messages.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing an empty cache with four retained success notifications after cleanup.
- Exact UI state/text: `0 entries · 0 B`, `The app-owned artifact cache is empty.`, and repeated `Success: The cache entry was removed.` messages.
- Related contract/code, when useful: Pending later implementation inspection.

**Notes for later implementation**

Treat cache refresh as a state-reconciliation boundary. Clear obsolete operation results on successful refresh, and consider retaining only a single bounded latest result when needed for confirmation or accessibility announcements.

#### UX-007 — Cache notifications expose internal technical result codes

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A20`
- Proposed phase: `5G`
- Status: `Open`

**Preconditions**

1. Open `Support & Storage` and view the artifact cache.
2. Complete a cache-removal action.

**Reproduction**

1. Expand `Technical details` on a cache-success notification.
2. Inspect the displayed value.

**Expected**

Normal success notifications should contain only user-relevant confirmation. Internal result identifiers should remain in sanitized diagnostics or support exports unless a specific copyable error code is needed for troubleshooting.

**Actual**

The notification exposes the internal identifier `cache_entry_removed` under `Technical details`. The identifier adds no useful information for a successful action and exposes implementation terminology directly in the product UI.

**User impact**

The technical section looks developer-oriented and unfinished. Similar identifiers could disclose internal naming conventions without helping users understand or resolve a problem.

**Workaround**

Leave `Technical details` collapsed.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing an expanded success notification with `cache_entry_removed`.
- Exact UI state/text: `Technical details` and `cache_entry_removed`.
- Related contract/code, when useful: Phase 5H calls for removal of raw IDs and internal implementation terms from normal UI; Phase 5G permits sanitized, actionable error codes where troubleshooting requires them.

**Notes for later implementation**

Remove technical-detail disclosure from routine success messages. For actual failures, expose only stable, sanitized, copyable support codes when they enable a concrete troubleshooting workflow; keep raw internal event names out of the normal interface.

#### UX-008 — Bulk cache-clear actions remain enabled when the cache is empty

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A20`
- Proposed phase: `5G`
- Status: `Open`

**Preconditions**

1. Open `Support & Storage`.
2. Clear all removable artifact-cache entries so the cache reports `0 entries · 0 B`.

**Reproduction**

1. Observe the cache controls after the cache becomes empty.

**Expected**

`Clear unused` and `Clear all removable` should be disabled when no matching cache entries exist. Disabled controls should expose a concise visible reason or accessible description where needed.

**Actual**

The cache reports that it is empty, but `Clear unused` and `Clear all removable` remain enabled. Only `Remove selected` is disabled.

**User impact**

The enabled destructive-looking actions imply that work remains available, contradict the empty-state message, and allow pointless operations. This weakens trust in the cache state and control semantics.

**Workaround**

Ignore the enabled actions when the cache count is zero.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing `0 entries · 0 B`, the empty-cache message, disabled `Remove selected`, and enabled `Clear unused` and `Clear all removable` buttons.
- Exact UI state/text: `The app-owned artifact cache is empty.`
- Related contract/code, when useful: Phase 3C requires disabled controls and their reasons to reflect current operation availability.

**Notes for later implementation**

Derive each bulk action's enabled state from the refreshed cache inventory and operation lock. `Clear unused` requires at least one unused removable entry; `Clear all removable` requires at least one removable entry. Preserve the existing in-progress lock behavior.

#### UX-009 — System-status panel exposes the implementation language

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A01`
- Proposed phase: `5H`
- Status: `Open`

**Preconditions**

1. Launch EmuChef and allow the backend runtime to initialize successfully.
2. Open or inspect the main workflow's `System status` panel.

**Reproduction**

1. View the runtime row in the `System status` panel.

**Expected**

The panel should describe user-relevant subsystem state without naming the implementation technology. A label such as `Runtime`, `Application services`, or `Backend` with value `Ready` is sufficient.

**Actual**

The panel labels the subsystem as `Rust runtime` and reports `Ready`.

**User impact**

The programming language is irrelevant to the user's task and makes the interface read like an engineering or diagnostics surface rather than a finished consumer application. It also adds unnecessary terminology that users may not understand.

**Workaround**

None required; the status remains understandable, but the label exposes implementation detail.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the `System status` panel with `Rust runtime` and `Ready`.
- Exact UI state/text: `Rust runtime` / `Ready`.
- Related contract/code, when useful: Phase 5H explicitly calls for removal of sidecar, schema, digest, and other internal implementation terminology from normal UI.

**Notes for later implementation**

Rename the user-facing label without changing runtime authority or diagnostics. Detailed implementation information may remain available in exported diagnostics or developer-only surfaces.

#### UX-010 — Platform-Tools maintenance actions are exposed in the primary workflow

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A03`, `A07`
- Proposed phase: `5G`
- Status: `Open`

**Preconditions**

1. Launch EmuChef with a valid managed Platform-Tools installation.
2. Proceed into the normal device/setup workflow.

**Reproduction**

1. Inspect the persistent `System status` panel in the main workflow.
2. Observe the `Replace Platform-Tools` and `Remove Platform-Tools` actions.

**Expected**

The primary workflow should emphasize device connection, setup selection, inputs, review, and execution. Platform-Tools replacement and removal are infrequent maintenance actions and should be available from a secondary settings, support, or maintenance surface rather than occupying persistent space in the main workflow.

**Actual**

`Replace Platform-Tools` and the destructive `Remove Platform-Tools` action are always visible in the primary workflow beside normal setup content.

**User impact**

The actions add visual noise, compete with the current task, and overemphasize implementation maintenance. Persistently exposing a destructive removal action also increases the chance of accidental activation and makes the main screen feel like an administrative/debug interface.

**Workaround**

Ignore the actions during normal use.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the Setup stage and persistent `System status` panel.
- Exact UI state/text: `Replace Platform-Tools` and `Remove Platform-Tools` remain visible while choosing a setup.
- Related contract/code, when useful: Platform-Tools import, replacement, and removal remain trusted Tauri-owned operations; relocating their entry points must not move filesystem or validation authority into React.

**Notes for later implementation**

Move maintenance actions to a Settings or Support & Storage subview. Keep status visibility in the main workflow only when it affects the current task. When Platform-Tools are missing or invalid, present the required setup/recovery action contextually. Preserve a confirmation step for removal and keep all native picker and validation ownership in Tauri.

### UX-011 — Configuration management is embedded in the primary workflow instead of the platform-native application menu

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A14`, `A15`, `A16`
- Proposed phase: `5F`
- Status: `Open`

**Preconditions**

1. Launch EmuChef with the main workflow available.

**Reproduction**

1. Inspect the top-level application window.
2. Observe that `New`, `Open...`, `Save`, and `Save As...` are presented in a persistent in-window configuration panel.
3. Inspect the macOS application menu bar for equivalent native File-menu commands.

**Expected**

Core document-management commands should use the platform-native application menu system, with standard macOS placement and keyboard shortcuts. The workflow may show compact current-configuration status, but it should not permanently devote primary content space to document commands.

**Actual**

Configuration management is embedded as a large persistent panel inside the primary workflow, competing with the device-setup task and making the application feel more like a web page than a native desktop application.

**User impact**

The primary workflow is visually cluttered, standard desktop conventions are not used, and common commands are harder to discover through expected macOS menu locations and shortcuts.

**Workaround**

Use the in-window controls.

**Evidence**

- Screenshot: User-provided screenshots from 2026-07-14 showing the persistent `Unsaved configuration` panel and its `New`, `Open...`, `Save`, and `Save As...` controls.
- Exact UI state/text: `Unsaved configuration`, `New`, `Open...`, `Save`, and `Save As...`.
- Related contract/code, when useful: Later implementation should preserve Tauri-owned native dialogs, dirty-state prompts, recovery behavior, and saved-configuration authority while moving command presentation into native menus.

**Notes for later implementation**

Add standard native menu commands, preferably under `File`: New, Open, Open Recent, Save, Save As, Close, and any supported duplicate or rename actions. Use standard shortcuts such as Cmd+N, Cmd+O, Cmd+S, and Cmd+Shift+S. Keep a compact document title and dirty indicator in the window rather than the current full-width action panel.

### UX-012 — Restart runtime is exposed as a primary workflow action instead of a utility command

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A27`
- Proposed phase: `5G`
- Status: `Open`

**Preconditions**

1. Launch EmuChef with the main workflow available.

**Reproduction**

1. Inspect the persistent configuration panel at the top of the window.
2. Observe the `Restart runtime` action alongside normal configuration commands.

**Expected**

Runtime recovery should be available as a secondary maintenance action in a platform-native `Utilities` menu, Support & Storage, or an error-specific recovery surface. It should appear in the main workflow only when runtime failure makes the action directly relevant.

**Actual**

`Restart runtime` is persistently displayed next to New/Open/Save controls in the primary workflow.

**User impact**

An implementation-oriented recovery command receives excessive prominence, adds clutter, and can imply that routine runtime restarts are part of normal configuration management.

**Workaround**

Ignore the action unless runtime recovery is needed.

**Evidence**

- Screenshot: User-provided screenshots from 2026-07-14 showing `Restart runtime` inside the persistent configuration panel.
- Exact UI state/text: `Restart runtime`.
- Related contract/code, when useful: Runtime restart must retain existing safeguards for active operations, pending prompts, recovery staging, stale handles, and focus restoration.

**Notes for later implementation**

Move the command to a native `Utilities` menu. Disable it with an accessible reason while an operation cannot be safely interrupted. Surface a contextual restart action in runtime-failure states so recovery remains obvious when actually needed.

#### UX-013 — Review plan remains enabled when no recipes are selected

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A08`, `A10`
- Proposed phase: `5C`
- Status: `Open`

**Preconditions**

1. Connect and confirm a supported device.
2. Choose a valid device setup.
3. Reach the recipe-selection screen.
4. Leave every recipe unselected.

**Reproduction**

1. Observe the enabled `Review plan` action while no recipe checkbox is selected.
2. Attempt to continue toward plan review.

**Expected**

`Review plan` should be disabled until the current selection can produce a meaningful valid plan. The screen should provide a visible explanation such as `Select at least one recipe to continue.`

**Actual**

`Review plan` remains enabled while every recipe is unselected.

**User impact**

The primary action advertises a valid next step when the setup contains no requested work. This permits avoidable validation failure or an empty-plan transition and makes the selection requirements unclear.

**Workaround**

Select at least one recipe before using `Review plan`.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing all recipe choices unselected while `Review plan` remains enabled.
- Exact UI state/text: `Customize your setup`, five unselected recipe controls, and enabled `Review plan`.
- Related contract/code, when useful: The planner baseline rejects an empty execution plan; the frontend should prevent this known-invalid transition rather than relying on later validation.

**Notes for later implementation**

Derive the action state from the authoritative current selection and validation state. Disable the button when zero recipes are selected, associate it with a visible disabled reason, and preserve backend empty-plan validation as defense in depth.

### UX-014 — Selecting a recipe immediately presents a blocking configuration error

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A08`, `A09`
- Proposed phase: `5D`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Reach the setup customization screen.
2. XaniteOG is not selected and its required APK binding is empty.

**Reproduction**

1. Select `Install XaniteOG`.
2. Observe the validation area immediately after selection.

**Expected**

Selecting a recipe should reveal its required inputs and explain what the user must provide. A blocking error summary should appear only after the user attempts to continue, review, or explicitly validate, or after the relevant input has been touched and remains invalid.

**Actual**

The interface immediately shows a prominent red `Resolve 1 configuration error` panel stating that the required XaniteOG APK binding is missing, before the user has had a reasonable opportunity to supply the newly revealed input.

**User impact**

A normal selection action is presented as an error. This makes the workflow feel punitive and suggests the user has done something wrong when they have only enabled a recipe with a required input.

**Workaround**

Ignore the error panel, scroll to the newly revealed XaniteOG APK input, and provide the file.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the error immediately after selecting `Install XaniteOG`.
- Exact UI state/text: `Resolve 1 configuration error` and `Required binding 'app.xaniteog.install/xaniteog_apk' is missing.`
- Related contract/code, when useful: Phase 5D calls for early input validation, but validation timing and presentation should distinguish an unmet newly introduced requirement from a user-caused error.

**Notes for later implementation**

Keep the requirement visible immediately, but present it as neutral required-input guidance. Promote it to a blocking error only when the user attempts to advance, reviews the plan, invokes validation, or leaves a touched required input unresolved.

**Resolution evidence (2026-07-21)**

The Inputs stage renders an untouched `binding_missing` requirement as neutral authored guidance. Explicit validation, review, or editing the affected field promotes the sanitized diagnostic to the field and summary. The DOM regression `new required inputs stay neutral until validation is requested` covers the documented reproduction and promotion behavior.

### UX-015 — Validation UI exposes internal binding identifiers and error codes

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A09`
- Proposed phase: `5D`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Select a recipe with a missing required input, such as `Install XaniteOG`.

**Reproduction**

1. Trigger or observe the missing-input validation panel.
2. Expand `Technical details`.

**Expected**

The normal UI should identify the missing user-facing input, for example `Select the XaniteOG APK`, without exposing authored binding paths or internal machine codes. Stable sanitized codes should appear only where they materially help diagnose a failure, such as support diagnostics or a copyable error-reference surface.

**Actual**

The validation message exposes the internal binding identifier `app.xaniteog.install/xaniteog_apk`, and the expanded technical details expose `binding_missing`.

**User impact**

Internal schema and code terminology distracts from the action the user needs to take, makes the product appear unfinished, and unnecessarily expands the amount of implementation detail visible in ordinary workflow use.

**Workaround**

Infer from the recipe and surrounding fields that the XaniteOG APK must be selected.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the internal binding identifier and `binding_missing` technical code.
- Exact UI state/text: `Required binding 'app.xaniteog.install/xaniteog_apk' is missing.` and `binding_missing`.
- Related contract/code, when useful: Phase 5H explicitly calls for removing raw IDs, schema language, and internal implementation terms from normal UI.

**Notes for later implementation**

Map diagnostics to authored user-facing labels and actionable instructions. Retain stable sanitized codes in exported diagnostics or an intentionally support-oriented surface, not as routine expandable details for ordinary validation.

**Resolution evidence (2026-07-21)**

Tauri maps input diagnostics to label-based actionable messages and removes routine technical-code presentation from the Inputs stage. DOM and security-policy regressions assert that internal codes and binding identifiers are absent from ordinary validation UI while support-oriented diagnostics remain separately controlled.

#### UX-016 — Device destination path incorrectly uses a host filesystem Browse control

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A09`
- Proposed phase: `5D`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Reach the Inputs stage.
2. Select the `Copy ROM library` recipe.

**Reproduction**

1. Inspect the generated inputs for the selected recipe.
2. Locate `Device ROM folder (required)`.

**Expected**

A device destination path should be presented as a device-path value, using a text field, validated preset, or device-aware destination selector. It must not invoke the host macOS filesystem picker because the destination exists on the connected Android device rather than the Mac.

**Actual**

`Device ROM folder (required)` displays a `Browse...` button identical to host file and folder inputs. The field contains `/sdcard/ROMs`, but the control implies that the user can browse the Mac filesystem to choose the Android destination.

**User impact**

The UI conflates host source paths with device destination paths. Users can reasonably expect `Browse...` to show folders on the connected device, but a native host picker cannot select `/sdcard/...`. This creates a misleading and potentially nonfunctional input workflow.

**Workaround**

Manually type or retain the device path in the text field and ignore the Browse control.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing `Device ROM folder (required)` with `/sdcard/ROMs` and a `Browse...` button.
- Exact UI state/text: `ROM source folder (required)` correctly needs a host folder picker, while `Device ROM folder (required)` incorrectly presents the same picker treatment.
- Related contract/code, when useful: React must not gain generic device-filesystem authority. Any future destination browser would need a bounded Tauri/backend-owned device-path contract rather than reuse of the host picker.

**Notes for later implementation**

Differentiate input rendering by path authority and semantic role. Host file/directory inputs may use native Browse actions. Device destination paths should use validated text or curated destination choices unless a separately designed, backend-owned device browser is added.

**Resolution evidence (2026-07-21)**

The Tauri projection no longer assigns host-picker authority to `device_path` inputs, and the Inputs stage renders them as text values. The DOM regression `device destinations stay textual and sensitive values use concealed controls` covers the documented device-destination reproduction.

### UX-017 — Save dialog uses confusing internal terminology and does not clearly explain saved contents

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A14`
- Proposed phase: `5F`
- Status: `Open`

**Preconditions**

1. Create or modify a setup that can be saved.
2. Invoke Save or Save As so the configuration-name dialog appears.

**Reproduction**

1. Open the save dialog.
2. Read the title and explanatory copy before naming the configuration.

**Expected**

The dialog should use familiar user-facing language and plainly state what will be saved, what will not be saved, and what reopening the file will require. The user should not need to understand internal concepts such as portability or runtime authority.

**Actual**

The dialog is titled `Name this portable configuration` and says `The name identifies this portable configuration. Runtime authority and device details are not saved.` The terms `portable configuration` and `runtime authority` are undefined and do not clearly explain the practical contents of the file.

**User impact**

Users cannot confidently determine whether selected recipes, setup choices, file references, device identity, generated plans, or execution state will be retained. This creates uncertainty at the point where they are committing work to disk.

**Workaround**

Infer the intended behavior from prior product knowledge or save the file and inspect behavior after reopening.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the configuration-name dialog.
- Exact UI text: `Name this portable configuration` and `The name identifies this portable configuration. Runtime authority and device details are not saved.`
- Related contract/code, when useful: Saved configurations retain user intent but require fresh device detection, validation, planning, and review when reopened.

**Notes for later implementation**

Use plain language such as `Name this setup`. Explain concretely that the file saves the selected setup, recipes, and reusable input references, while device identity, generated plans, execution progress, and results are not saved. Avoid `portable`, `runtime authority`, `document`, `binding`, and similar schema or architecture terms in the normal save flow. Keep the explanation concise, with optional secondary disclosure if more detail is needed.

### UX-018 — Updates dialog uses a non-standard Close control

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A21`
- Proposed phase: `5H`
- Status: `Open`

**Preconditions**

1. Launch EmuChef with the main application window available.
2. Open the Updates surface.

**Reproduction**

1. Select `Updates` from the application header.
2. Inspect the dialog dismissal control.

**Expected**

The dialog should use a conventional close affordance that is visually and behaviorally consistent with the application's other modal surfaces and with macOS conventions. Suitable implementations include a clearly styled footer `Close` button, a standard close icon in the dialog header, and Escape-key dismissal when safe.

**Actual**

`Close` appears as an isolated text-style link directly below the dialog title. It does not read as a standard dialog control and is visually disconnected from the primary actions at the bottom of the dialog.

**User impact**

The dismissal action is harder to discover and makes the dialog feel inconsistent with native and conventional desktop modal behavior.

**Workaround**

Select the text-style `Close` control or use Escape if supported.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the EmuChef Updates dialog.
- Exact UI state/text: `Close` appears between the `EmuChef Updates` heading and the explanatory body text.
- Related contract/code, when useful: Phase 3C requires accessible, safely dismissible modal surfaces but does not require this non-standard visual placement.

**Notes for later implementation**

Use one consistent modal dismissal pattern across Updates, Support & Storage, recovery prompts, and other dialogs. Preserve focus containment, deterministic focus restoration, Escape handling, and safe-dismissal rules.

### UX-019 — Simulation start timestamp is shown as a raw machine-formatted value

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A11`, `A13`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Generate and run a simulated plan.
2. Allow the run to complete or fail so the execution summary is shown.

**Reproduction**

1. Open the simulated-run results screen.
2. Inspect the `Started` value in the execution summary.

**Expected**

The start time should use a localized, human-readable date and time format appropriate to the user's system settings, with timezone detail available only when useful.

**Actual**

The UI displays a raw ISO-style timestamp with fractional seconds, for example `2026-07-15T04:43:38.906683Z`.

**User impact**

The timestamp is difficult to scan and looks like internal telemetry rather than end-user report content.

**Workaround**

The user can manually interpret the UTC timestamp.

**Evidence**

- Screenshot: User-provided simulated-run failure screenshot from 2026-07-14.
- Exact UI state/text: `Started 2026-07-15T04:43:38.906683Z`.

**Notes for later implementation**

Format timestamps at the presentation layer using the user's locale. Preserve the canonical timestamp in exported diagnostics and machine-readable reports.

**Resolution evidence (2026-07-21)**

The execution UI formats retained RFC 3339 timestamps with the user's locale and time zone while leaving backend and export timestamps canonical. `ExecutionStep.dom.test.tsx` asserts that the raw ISO value is absent from the visible failed-run report.

### UX-020 — Simulation failure cards have insufficient visual separation

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A12`, `A13`
- Proposed phase: `5H`
- Status: `Open`

**Preconditions**

1. Run a simulated plan that produces multiple failed or blocked results.

**Reproduction**

1. Open the simulated-run results screen.
2. Inspect the stacked red failure cards below the summary.

**Expected**

Each failure should be visually distinct, with enough vertical spacing, grouping, or dividers to make the number and boundaries of failures immediately clear.

**Actual**

The red cards are placed directly against one another with little or no visible gap, making the group read like one large error block.

**User impact**

Users cannot quickly distinguish separate failures, understand how many issues occurred, or map each message to a specific failed or blocked step.

**Workaround**

Read each repeated text block carefully and infer card boundaries from subtle edge changes.

**Evidence**

- Screenshot: User-provided simulated-run failure screenshot from 2026-07-14.
- Exact UI state/text: two `Completed work could not be verified.` cards followed by one `Required work was blocked because a dependency did not complete.` card.

**Notes for later implementation**

Add consistent card spacing and stronger per-item hierarchy. Consider grouping each failure with the affected recipe or step name rather than presenting detached generic cards.

### UX-021 — Simulation failure messages are generic, repetitive, and do not identify the affected work

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A12`, `A13`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Run a simulated plan that produces failed and blocked steps.

**Reproduction**

1. Open the simulated-run results screen.
2. Read the failure summary cards and their `Repair and retry` guidance.

**Expected**

Each failure should identify the affected recipe or step, explain the specific problem in user-facing language, and provide a concrete next action. Blocked work should identify the dependency that failed.

**Actual**

The UI repeats generic text such as `Completed work could not be verified.` and `Resolve the reported feature problem, then generate and review a fresh plan.` without identifying which completed work, feature, verification, or dependency is involved.

**User impact**

The user knows the simulation failed but does not know what to fix. The repeated generic recovery text adds visual noise without making the result actionable.

**Workaround**

Scroll into the detailed recipe and step results and infer which underlying step caused each summary failure.

**Evidence**

- Screenshot: User-provided simulated-run failure screenshot from 2026-07-14.
- Exact UI state/text: `Completed work could not be verified.`, `Required work was blocked because a dependency did not complete.`, and repeated `Repair and retry: Resolve the reported feature problem, then generate and review a fresh plan.`

**Notes for later implementation**

Build each summary from structured failure context: affected recipe, affected step, concise cause, dependency when applicable, and a specific recovery action. Avoid duplicating identical guidance across adjacent cards when one grouped recovery section would be clearer.

**Resolution evidence (2026-07-21)**

Trusted execution projection resolves executor identity against retained authored feature/action metadata and combines that context with backend-classified cause and remediation copy. The Tauri regression `failures_and_events_use_authored_action_context_without_exposing_identity` proves that the failed action and feature are named while raw verifier text, codes, recipe IDs, and step IDs remain absent.

### UX-022 — Execution report exposes raw verifier and dependency identifiers

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A11`, `A12`, `A13`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Run a simulated plan containing RetroArch provisioning work.
2. Allow one or more verification steps to fail or become blocked.

**Reproduction**

1. Open the failed simulated-run report.
2. Scroll to the individual failed and blocked steps.
3. Inspect the secondary error text.

**Expected**

Each failed or blocked step should present a concise, sentence-cased explanation of what could not be verified and what the user should do next. Internal verifier names, recipe IDs, step IDs, and dependency identifiers should remain in exported diagnostics rather than the primary report.

**Actual**

The report displays raw, lowercase implementation values such as `verify failed: path_exists`, repeated `path_exists` identifiers, and dependency IDs including `app.retroarch.provision/copy_cheats` and `app.retroarch.provision/copy_core_system_files`.

**User impact**

The error text is difficult to understand, visually unfinished, and does not tell the user which expected file or condition is missing. Internal recipe and verifier identifiers add noise and expose implementation details without helping recovery.

**Workaround**

Infer the affected operation from the step title and return to Review, but the report does not identify the missing path or provide a direct corrective action.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing failed RetroArch copy and launch steps in the simulated-run report.
- Exact UI state/text: `verify failed: path_exists`, `verify failed: path_exists, path_exists, path_exists`, and `dependency blocked: app.retroarch.provision/copy_cheats, app.retroarch.provision/copy_core_system_files`.
- Related contract/code, when useful: Pending later implementation inspection.

**Notes for later implementation**

Map verifier and dependency outcomes to user-facing messages at the presentation boundary. Preserve stable raw codes and identifiers in the exported report or diagnostic payload. Use sentence case and name the failed expectation, for example `RetroArch cheats could not be verified at the destination` or `RetroArch could not be launched because required files were not copied successfully`.

**Resolution evidence (2026-07-21)**

Ordinary execution DTOs no longer contain issue codes, recipe IDs, step IDs, event types, phases, or raw executor messages. Trusted Tauri projection uses fixed backend classifications and retained authored action context. Rust projection tests assert the absence of raw identity and failure text, and the execution DOM renders only the sanitized message and remediation.

### UX-019 — Simulated-run start time is displayed as a raw ISO timestamp

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A11`, `A13`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Generate and start a simulated run.
2. Allow the run to reach a completion or failure report.

**Reproduction**

1. Open the simulated-run result screen.
2. Inspect the `Started` summary value.

**Expected**

The start time should use a readable local date and time format appropriate to the operating system locale, while preserving the precise timestamp in exported diagnostics if needed.

**Actual**

The UI displays a raw UTC ISO timestamp such as `2026-07-15T04:43:38.906683Z`.

**User impact**

The value is harder to scan than a normal date and time and looks like internal diagnostic data rather than a finished application summary.

**Workaround**

The user can manually interpret or convert the timestamp.

**Evidence**

- Screenshot: User-provided simulated-run failure screenshot from 2026-07-14.
- Exact UI state/text: `Started 2026-07-15T04:43:38.906683Z`.

**Notes for later implementation**

Render the timestamp using the user locale and time zone, for example a medium date plus short time. Keep the original machine-readable value only in diagnostics or exported reports.

This duplicate entry is resolved by the same localized-timestamp implementation and `ExecutionStep.dom.test.tsx` regression documented for the canonical UX-019 entry above.

### UX-020 — Simulated-run failure cards have insufficient visual separation

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A12`, `A13`
- Proposed phase: `5H`
- Status: `Open`

This is a duplicate description of the canonical UX-020 entry above. UX-020 remains assigned only to Phase 5H; Phase 5E does not claim failure-card spacing or broad visual separation work.

**Preconditions**

1. Complete a simulated run with multiple failed or blocked results.

**Reproduction**

1. Open the simulated-run failure summary.
2. Inspect the stacked red failure cards.

**Expected**

Each distinct failure should have clear vertical spacing or another visual boundary so users can quickly distinguish separate results.

**Actual**

The failure cards are stacked with little or no visible gap. Their matching background and border treatment makes multiple errors appear like one continuous block.

**User impact**

Users can miss that several separate failures occurred and have difficulty mapping each message to an individual failed or blocked operation.

**Workaround**

Read each repeated heading and recovery line carefully to infer the boundaries.

**Evidence**

- Screenshot: User-provided simulated-run failure screenshot from 2026-07-14 showing three adjacent red cards.
- Exact UI state/text: two `Completed work could not be verified.` cards followed by `Required work was blocked because a dependency did not complete.`

**Notes for later implementation**

Add consistent card spacing and consider including the affected recipe or step name in each card header so separation is both visual and semantic.

### UX-021 — Simulated-run failure messages do not explain what failed or what the user should change

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A12`, `A13`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Complete a simulated run with failed verification and a blocked dependency.

**Reproduction**

1. Open the simulated-run failure summary.
2. Read the top-level error cards and their `Repair and retry` guidance.

**Expected**

Each failure should identify the affected recipe or step, describe the specific condition that failed, and provide a concrete corrective action. Repeated failures should not use identical generic wording when their causes differ.

**Actual**

The report shows generic messages such as `Completed work could not be verified.` and repeatedly instructs the user to `Resolve the reported feature problem, then generate and review a fresh plan.` The phrase `reported feature problem` does not identify the failed item, cause, or required correction.

**User impact**

The user cannot determine from the summary what needs to be fixed before retrying. The recovery guidance forces them to inspect lower-level report details and guess which input, artifact, dependency, or verification condition caused the failure.

**Workaround**

Scroll through the detailed recipe and step results and infer the root cause from lower-level entries.

**Evidence**

- Screenshot: User-provided simulated-run failure screenshot from 2026-07-14.
- Exact UI state/text: `Completed work could not be verified.`, `Required work was blocked because a dependency did not complete.`, and `Repair and retry: Resolve the reported feature problem, then generate and review a fresh plan.`

**Notes for later implementation**

Generate user-facing summaries from structured failure data. Include the affected recipe or step name, a plain-language cause, and a specific next action. Keep raw error codes and full diagnostic context in expandable support details or exported reports.

This duplicate entry is resolved by the trusted action-context and remediation projection documented for the canonical UX-021 entry above.

### UX-022 — `Retry failed work` implies immediate retry but returns the user to planning

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed + code-supported`
- Scenario(s): `A12`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

This historical duplicate is tracked canonically as UX-040 below. The action is now labeled `Repair setup`, explains that completed steps remain report evidence, and requires a fresh plan and review rather than claiming an in-place retry.

**Preconditions**

1. Complete a simulated execution with at least one failed or blocked step.
2. Reach the terminal execution report.

**Reproduction**

1. Select `Retry failed work`.
2. Observe the destination and required workflow.

**Expected**

The action label should accurately describe its behavior. A button labeled `Retry failed work` implies that EmuChef will immediately re-attempt failed steps, or at least begin a focused retry flow.

**Actual**

The action returns the user to Step 4 / planning so inputs and validation can be repaired, a fresh plan generated, and the plan reviewed again before another execution. No failed step is retried in place.

**User impact**

The action behaves differently from its label, making the user uncertain whether retry failed, whether the app lost execution state, or whether they are expected to modify the configuration manually.

**Workaround**

Understand that the action begins a fresh repair-and-review flow rather than retrying immediately.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the terminal simulated-run report and `Retry failed work` action.
- Exact UI state/text: Selecting `Retry failed work` returns to Step 4 rather than re-executing failed steps.
- Related contract/code, when useful: `docs/product/phase-2c-completion-and-recovery.md` explicitly excludes retry-in-place and states that `Retry failed work` and `Repair configuration` always return to planning, refresh state, and require a fresh review.

**Notes for later implementation**

Preserve the existing fresh-plan safety contract. Rename the action to match the implemented behavior, such as `Repair and review`, `Fix setup`, or `Return to setup`, and add concise supporting text explaining that EmuChef must generate and review a fresh plan before another attempt. Do not add retry-in-place without a separate product and authority design.

### UX-023 — Step 4 is labeled `Inputs` even though it combines recipe selection and input collection

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A08`, `A09`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Select a supported device and setup.
2. Continue to Step 4.

**Reproduction**

1. Observe the left-hand workflow label `Inputs`.
2. Inspect the Step 4 content.

**Expected**

The workflow label should accurately describe the primary task performed in the step. Recipe selection and recipe-specific input collection should either be represented by distinct steps or by a label broad enough to describe both activities without misleading the user.

**Actual**

Step 4 is labeled `Inputs`, but its first major task is selecting recipes. Recipe selection and all resulting file, directory, path, and policy inputs are combined in the same long page.

**User impact**

The workflow label does not set the correct expectation. Users may assume recipes were already chosen in Setup, overlook recipe selection, or have difficulty understanding why selecting recipes and providing files are treated as one activity.

**Workaround**

Users can infer the combined behavior from the page contents.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing Step 4 labeled `Inputs` while the page begins with `Choose recipes`.
- Exact UI state/text: `Inputs`, `Customize your setup`, and `Choose recipes`.
- Related contract/code, when useful: Pending later implementation inspection.

**Notes for later implementation**

If recipe selection remains combined with inputs, rename the step to something broader such as `Customize`. The stronger correction is to split recipe selection and recipe-specific inputs into separate workflow stages as described in UX-024.

### UX-024 — Recipe selection and recipe-specific inputs are combined in a catalog UI that will not scale

- Type: `Missing MVP feature`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A08`, `A09`, `A10`
- Proposed phase: `5C`
- Status: `Open`

**Preconditions**

1. Select a supported device and setup.
2. Continue to Step 4 with the current bundled recipe catalog.

**Reproduction**

1. Review the recipe list at the top of Step 4.
2. Select multiple recipes.
3. Scroll through the recipe-specific inputs appended below the full list.

**Expected**

Recipe discovery and selection should remain understandable as the catalog grows. Users should be able to browse, filter, inspect, and select recipes without mixing that task with a potentially long set of generated inputs. After recipe selection, the workflow should present only the inputs required by the chosen recipes.

**Actual**

All recipes are displayed as a single flat list above all recipe-specific inputs in one page. The current catalog is already long enough to require substantial scrolling, and the layout has no visible search, category, filtering, compact summary, or progressive-disclosure model. Adding more recipes will make both recipe discovery and input completion progressively harder.

**User impact**

The design does not scale with the core product goal of supporting a growing recipe catalog. Users will have difficulty finding recipes, understanding what they selected, and locating the required inputs associated with each selection.

**Workaround**

Users must scan and scroll through the full combined page.

**Evidence**

- Screenshot: User-provided screenshot from 2026-07-14 showing the combined recipe-selection and input page.
- Exact UI state/text: `Customize your setup`, `Choose recipes`, and the Step 4 label `Inputs`.
- Related contract/code, when useful: Phase 5C owns recipe and setup selection experience.

**Notes for later implementation**

Create a dedicated recipe-selection stage before input collection. The recipe catalog should support a compact scalable presentation with at least search or filtering, categories or meaningful grouping, clear selected-state summaries, compatibility/recommendation indicators, and an explicit Continue action. The following stage should render only inputs required by selected recipes. Preserve dependency auto-inclusion and planner validation, but communicate auto-added dependencies clearly rather than silently expanding a large flat list.

### UX-025 — Platform-Tools setup copy exposes implementation details instead of guiding the user

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A02`
- Proposed phase: `5H`
- Status: `Open`

**Preconditions**

1. Launch EmuChef without a managed Platform-Tools installation.
2. View the one-time Platform-Tools setup panel.

**Reproduction**

1. Read the explanatory text under `Android SDK Platform-Tools is required`.
2. Read the smaller disclosure text below the import actions.

**Expected**

The setup panel should explain, in plain language, that the user needs to download the macOS Platform-Tools ZIP from Google and select it in EmuChef. Additional technical ownership or storage details should be omitted from the primary workflow unless they help the user make a decision.

**Actual**

The panel says the ZIP is imported `for local validation and managed installation` and adds that the selected ZIP `is never copied into the app bundle or repository`. These phrases describe implementation and repository boundaries rather than the user's task.

**User impact**

The wording increases cognitive load during first-time setup and introduces terms that are not useful to a nontechnical user. It makes a simple download-and-select task feel more complicated and developer-oriented.

**Workaround**

Users can ignore the technical language and use the two setup buttons.

**Evidence**

- Screenshot: User-provided A02.1 screenshot from 2026-07-18 showing the complete one-time setup panel.
- Exact UI text: `for local validation and managed installation` and `The selected ZIP remains yours and is never copied into the app bundle or repository.`

**Notes for later implementation**

Prefer direct wording such as: `Download the macOS Platform-Tools ZIP from Google, then select it here.` A short privacy/storage note may remain in Help or Support documentation, but repository and app-bundle terminology should not appear in the normal setup path.

### UX-026 — Platform-Tools import reports validation before the user selects a file

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A02`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Launch EmuChef without a managed Platform-Tools installation.
2. The `Import Platform-Tools ZIP` action is available.

**Reproduction**

1. Click `Import Platform-Tools ZIP`.
2. Observe the button while the native file picker is open.
3. Cancel the picker.

**Expected**

Opening the native picker should either leave the button label unchanged or show a neutral file-selection state such as `Choose ZIP...`. Validation should begin only after the user has selected a file.

**Actual**

The button immediately changes to `Validating...` while the native file picker is still open, before any file has been selected. Cancelling the picker returns the button to its original label without an error.

**User impact**

The status is temporally inaccurate and can make users believe EmuChef is already processing a file or has become busy while they are still choosing one.

**Workaround**

Ignore the temporary label and continue using or cancel the native picker.

**Evidence**

- Observation: Owner reproduced the label change during A02.2 and confirmed cancellation otherwise passes without an error or state change.
- Exact UI state: `Import Platform-Tools ZIP` changes to `Validating...` as soon as the file picker opens.

**Notes for later implementation**

Separate native-dialog-open state from archive-validation state. Disable duplicate invocation while the picker is open, but do not claim validation has started until a path is returned and backend processing begins.

### UX-027 — Platform-Tools replacement has no visible in-progress action wording

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A02`, `A03`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. A valid managed Platform-Tools installation is active.
2. The System Status panel shows `Replace Platform-Tools`.

**Reproduction**

1. Click `Replace Platform-Tools`.
2. Select a valid Platform-Tools ZIP.
3. Observe the replacement action while the archive is being processed.

**Expected**

The control should communicate the active operation with wording such as `Replacing...`, `Reinstalling...`, or `Updating...`, and conflicting Platform-Tools actions should remain unavailable until completion.

**Actual**

The available action is correctly labeled `Replace Platform-Tools`, but no replacement/import/reinstall/update wording is shown while processing.

**User impact**

The user receives weak feedback that the requested replacement is underway and may be uncertain whether the click registered or whether the application is still processing.

**Workaround**

Wait for the operation to finish and verify the Platform-Tools version in System Status.

**Evidence**

- Screenshot: User-provided A02.6 screenshot showing Platform-Tools 37.0.0 and the `Replace Platform-Tools` action in System Status.
- Observation: Owner reports no operation-specific progress wording during replacement.

**Notes for later implementation**

Use a dedicated replacement busy label and retain the existing operation guard. Avoid generic `Validating...` before file selection; after selection, a staged sequence such as `Validating...` followed by `Replacing...` is acceptable if those states reflect actual work.

### UX-028 — Successful Platform-Tools replacement reports a false device disconnect

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Unknown`
- Evidence: `Observed`
- Scenario(s): `A03`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Platform-Tools 37.0.0 is installed and working.
2. A supported Pocket S Mini is connected and visible as available.

**Reproduction**

1. Select `Replace Platform-Tools`.
2. Import the same valid official Platform-Tools ZIP.
3. Wait for replacement to complete.
4. Observe the workflow notice and device list.

**Expected**

A successful replacement should briefly refresh device detection and either preserve the selected device when identity remains valid or report a neutral refresh state. A disconnect warning should appear only when the device is actually unavailable.

**Actual**

After successful same-version replacement, EmuChef displays: `Attention: The selected device disconnected. Connect it again to continue.` The Pocket S Mini remains listed as `Status: available`, can still be selected, and the workflow can continue.

**User impact**

The application presents contradictory device state and falsely suggests user action is required. This undermines confidence in device identity and stale-state handling after a runtime dependency changes.

**Workaround**

Ignore the warning and select the still-available device again.

**Evidence**

- Screenshot: User-provided A03.3 screenshots from 2026-07-18 showing the Pocket S Mini as available while the disconnect warning is displayed.
- Exact UI text: `Attention: The selected device disconnected. Connect it again to continue.` and `Status: available`.

**Notes for later implementation**

Treat Platform-Tools replacement as a controlled device-redetection event. Clear stale selected-device authority while replacement is active, then reconcile by stable device identity after refresh. Do not emit a disconnect warning when the same device is immediately rediscovered and available.

### UX-029 — Removing Platform-Tools has no confirmation and exposes internal authority terminology

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A03`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. A valid Platform-Tools installation is active.
2. A supported device may be selected or visible.
3. `Remove Platform-Tools` is available in System Status.

**Reproduction**

1. Click `Remove Platform-Tools`.
2. Observe whether confirmation is requested.
3. Observe the result message and resulting workflow state.

**Expected**

Because removal disables device detection and invalidates current device-dependent work, EmuChef should ask for confirmation and explain the user-visible consequence in plain language. The user should be able to cancel safely.

**Actual**

Removal proceeds immediately without confirmation. The resulting setup panel says: `Platform-Tools removed. Device, review, and execution authority were invalidated.` The app correctly returns to the one-time setup state, but `authority` and `invalidated` are internal implementation terms.

**User impact**

A destructive setup action can be triggered accidentally with no chance to cancel. The completion message does not clearly explain that device selection and any current review must be repeated after reinstalling Platform-Tools.

**Workaround**

Re-import Platform-Tools and repeat device selection. There is no pre-removal cancellation path after clicking the action.

**Evidence**

- Screenshot: User-provided A03.4 screenshot from 2026-07-18 showing the post-removal setup state.
- Exact UI text: `Platform-Tools removed. Device, review, and execution authority were invalidated.`
- Observation: no confirmation dialog was presented.

**Notes for later implementation**

Add a confirmation dialog that states the practical impact, for example: `Remove Platform-Tools? Device detection will stop and you will need to select your device and review your setup again after reinstalling.` Use `Remove` and `Cancel` actions. Replace authority terminology in the completion notice with direct user-facing wording.

### UX-030 — Device refresh gives no visible progress or completion feedback

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A04`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Platform-Tools is installed and verified.
2. No Android device is connected.
3. The Connect stage shows the empty device list.

**Reproduction**

1. Click `Refresh devices`.
2. Repeat the action several times.
3. Observe the action label and empty-state area while and after detection runs.

**Expected**

The refresh action should indicate that detection is in progress, prevent conflicting duplicate actions, and provide a subtle completion result when no devices are found. The user should be able to distinguish an idle empty state from a refresh that has not yet completed.

**Actual**

Clicking `Refresh devices` produces no visible progress state and no completion feedback. The screen looks unchanged before, during, and after the refresh.

**User impact**

Users cannot tell whether the click registered, whether ADB detection is still running, or whether the completed result is genuinely zero devices. Repeated clicking is encouraged because the interface provides no acknowledgement.

**Workaround**

Wait and infer completion from the unchanged device list.

**Evidence**

- Screenshot: User-provided A04 screenshot from 2026-07-18 showing the no-device state.
- Observation: repeated refreshes produced no visible progress or completion feedback.
- Exact UI text: `No ADB devices detected yet. Refresh after connecting a device.` and `Refresh devices`.

**Notes for later implementation**

Use an operation-specific state such as `Refreshing...`, disable duplicate refresh invocation while detection is active, and announce a stable result such as `No devices found` without accumulating stale notices.

### UX-031 — No-device state does not explain USB debugging or device authorization

- Type: `Missing MVP feature`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A04`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Platform-Tools is installed.
2. No authorized Android device is detected.
3. The user is on the Connect stage.

**Reproduction**

1. Read the Connect-stage instructions and empty-state message.
2. Look for guidance on enabling Developer Options, USB debugging, accepting the device authorization prompt, USB connection mode, or troubleshooting an undetected device.

**Expected**

The first connection screen should provide concise, actionable guidance for the common setup path: connect by USB, enable Developer Options and USB debugging, unlock the device, accept the authorization prompt, then refresh. A compact troubleshooting link or expandable help section should cover common detection failures without overwhelming the primary screen.

**Actual**

The page says `Connect with USB debugging enabled` and `No ADB devices detected yet. Refresh after connecting a device.` It does not explain how to enable USB debugging, that an authorization prompt may appear on the device, or what to check when refresh still finds nothing.

**User impact**

Nontechnical users can become blocked at the first workflow step without knowing which device-side action is required. The omission is particularly significant because authorization and USB-debugging setup are common first-use barriers.

**Workaround**

Consult external Android or device-specific instructions.

**Evidence**

- Screenshot: User-provided A04 screenshots from 2026-07-18 showing the complete no-device state before and after relaunch.
- Exact UI text: `Connect with USB debugging enabled. EmuChef only reads device information in this phase.`
- Observation: no inline or linked explanation of USB debugging and authorization is available on this screen.

**Notes for later implementation**

Keep the primary copy concise, but add a visible `How to connect a device` disclosure or help action. Explain the authorization prompt and provide a short checklist. Avoid device-specific instructions in the core flow unless a supported device profile can supply them safely.

### UX-032 — Unauthorized devices are indistinguishable from no device being connected

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A05`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Platform-Tools is installed and working.
2. An Android device is connected by USB with USB debugging enabled.
3. The device has not authorized the host computer for debugging.

**Reproduction**

1. Revoke USB debugging authorizations or connect the device without accepting the RSA prompt.
2. Click `Refresh devices`.
3. Observe the device list.
4. Accept the authorization prompt on the device and refresh again.

**Expected**

EmuChef should distinguish an unauthorized connected device from an absent device. It should explain that the user must unlock the device and accept the USB debugging authorization prompt, then refresh. After authorization succeeds, the same device should become available without stale warning text.

**Actual**

While the device is connected but unauthorized, EmuChef shows the same empty state used when no device is connected: `No ADB devices detected yet. Refresh after connecting a device.` The device does not appear in the list and there is no authorization-specific guidance. After the prompt is accepted on the device and the list is refreshed, the device appears and works as expected.

**User impact**

The app reports the wrong problem at a common first-use failure point. Users may reconnect cables, reinstall tools, or assume the device is unsupported when the required action is simply accepting the on-device authorization prompt.

**Workaround**

Notice and accept the authorization prompt on the device, then manually refresh the list.

**Evidence**

- Screenshot: User-provided A05 screenshot from 2026-07-18 showing the no-device empty state while an unauthorized device is physically connected.
- Exact UI text: `No ADB devices detected yet. Refresh after connecting a device.`
- Observation: authorizing USB debugging on the device and refreshing causes the device to appear normally.

**Notes for later implementation**

Preserve ADB-reported unauthorized devices in the discovery result and render a nonselectable row or dedicated guidance state such as `Authorization required`. Explain that the device should be unlocked and the USB debugging prompt accepted. Do not expose the full serial; retain existing redaction behavior.

### UX-033 — Unsupported devices are presented as available and skip the Device stage

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A06`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Platform-Tools is installed and working.
2. An authorized Android device is connected.
3. The device does not match any supported device plan.

**Reproduction**

1. Refresh the Connect stage.
2. Select the unsupported device, shown as `Status: available`.
3. Observe the workflow transition.

**Expected**

EmuChef should distinguish connection availability from product support. Selecting an unsupported device should lead to a Device-stage explanation that the hardware was detected but no supported device plan matches it. The user should be given a clear next action, such as returning to Connect, choosing another device, or viewing support information.

**Actual**

The unsupported Pocket DMG appears in the Connect list with the same green `Status: available` treatment as a supported device. Selecting it skips the Device stage entirely and jumps directly to Setup. The Setup screen shows `Match confidence: none` while still offering `AYANEO Base Setup` and `Choose a safe setup`, without explicitly stating that the device is unsupported or why no exact plan was selected.

**User impact**

The workflow implies that the device is fully supported and safe to continue with, even though no device plan matched. Skipping the Device stage removes the natural place to explain the mismatch and may lead users to apply a generic setup without understanding the risk or limitation.

**Workaround**

Use Back to return to Connect and choose a known supported device. The current UI provides no direct unsupported-device explanation.

**Evidence**

- Screenshots: User-provided A06 screenshots from 2026-07-18 showing the Pocket DMG as `Status: available`, the workflow jumping to Setup, and `Match confidence: none` with `AYANEO Base Setup` offered.
- Exact UI text: `Status: available`, `Match confidence: none`, `Choose a safe setup`, and `Starter setup for supported AYANEO devices.`
- Observation: the Device stage is skipped.

**Notes for later implementation**

Separate transport state from support state. A connected and authorized device may be `Connected` while still being `Unsupported`. Route device selection through the Device stage for all devices, show the match result in user language, and block or clearly gate generic setup selection when no supported plan matches. Do not expose raw match-confidence terminology in the normal workflow.

### UX-034 — Plan review exposes implementation metadata and private host-path details

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A08`, `A10`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Select one or more recipes and provide enough required inputs to generate a plan.
2. Reach the Review stage.

**Reproduction**

1. Inspect `Selected options` and the generated step list.
2. Scroll to steps requiring elevated or app-specific capabilities.
3. Inspect the bottom of the review panel.

**Expected**

The normal review should summarize what EmuChef will do in plain language, identify meaningful device impact, and protect private host-path details. Internal capability names, schema-oriented action labels, and plan integrity hashes should be absent from the primary review. Full paths and hashes may remain in explicitly technical diagnostics or exported reports when necessary.

**Actual**

The review displays implementation-oriented metadata including:

- the binding path `app.retroarch.provision/retroarch_cfg`;
- the full host path `/Users/.../Downloads/retroarch.cfg`;
- capability labels such as `Requires: apk install`, `Requires: app data write`, and `Requires: app launch`;
- `Elevated access` on multiple steps;
- a raw `Plan digest` SHA-256 value.

The step list also repeats internal action categories such as `Copy files`, `Launch app`, and `Device setup action` alongside user-facing step names.

**User impact**

The review is harder to scan, exposes a private local filesystem path during an ordinary workflow, and reads like a planner/debug representation rather than a decision-focused summary. Users cannot easily distinguish important risk or device impact from internal capability plumbing.

**Workaround**

Ignore the implementation metadata and infer the intended actions from the recipe and step titles.

**Evidence**

- Screenshots: User-provided A08 screenshots from 2026-07-18 showing `Selected options`, the full RetroArch configuration path, capability labels, repeated technical step categories, and the raw plan digest.
- Exact UI text includes `app.retroarch.provision/retroarch_cfg`, `Requires: app data write`, `Elevated access`, and `Plan digest:` followed by a long hexadecimal hash.

**Notes for later implementation**

Create a user-facing plan-review projection rather than rendering planner metadata directly. Show concise action groups, meaningful warnings, download/copy/install impact, and whether elevated access is needed in plain language. Redact host paths to a filename or user-approved abbreviated path. Keep binding IDs, capability tokens, action-kind names, and the digest in diagnostics or expandable developer details only.

**Resolution evidence (2026-07-21)**

`planConfiguration` now emits a Rust-authored feature-first review projection tied to the exact normalized plan. Tauri only attaches its opaque handle and defensively redacts the exact serial. React renders authored setup, target, feature, section, action, option, warning/blocker, destination, and deterministic work summaries without the digest, binding keys, recipe/step IDs, capability tokens, raw parameters, codes, or full host paths. Backend contract and React DOM regressions assert the complete safe shape and absence properties.

### UX-035 — Selected file inputs cannot be cleared

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A09`
- Proposed phase: `5D`
- Status: `Resolved 2026-07-21`

**Actual**

After selecting an optional file such as the RetroArch config, the field displays the path but provides no clear/remove action. The user cannot return the input to its unset state through the UI.

**User impact**

Optional inputs become effectively permanent for the current configuration unless the user starts over or edits persisted data elsewhere. This also makes it difficult to intentionally omit an optional file after reconsidering the setup.

**Evidence**

- Screenshot: User-provided A09 screenshot from 2026-07-18 showing the selected RetroArch config path and only a `Browse...` action.
- Observation: owner could not clear the selected value.

**Notes for later implementation**

Add an explicit `Clear` action for optional file and directory inputs. Clearing must remove the binding, refresh validation, invalidate stale review state, and omit the optional action from the generated plan.

**Resolution evidence (2026-07-21)**

Selected single-value and multi-value path inputs expose an explicit Clear action. The reducer regression `clearing an input removes its binding and invalidates downstream authority` proves that clearing removes portable intent and invalidates the prior description and review authority.

### UX-036 — Deleted input files produce a generic review failure without naming the invalid field

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A09`, `A10`
- Proposed phase: `5D`
- Status: `Resolved 2026-07-21`

**Actual**

When a previously selected RetroArch config file is deleted from disk, `Review plan` fails with `Action could not be completed` and `Resolve the setup validation errors before reviewing the plan.` The visible message does not identify the RetroArch config field or explain that the selected file no longer exists.

**User impact**

The user knows validation failed but cannot tell which path is stale or what must be reselected. This is especially problematic in configurations with several file inputs.

**Evidence**

- Screenshot: User-provided A09 screenshot from 2026-07-18 showing the generic review failure after deleting the selected config file.
- Observation: no field-specific missing-file message was presented.

**Notes for later implementation**

Validate selected paths before review and attach the diagnostic to the user-facing field label. Use wording such as `RetroArch config could not be found. Select the file again or clear this optional input.` Provide a direct relink or clear action.

**Resolution evidence (2026-07-21)**

Rust validates retained paths for existence, readability, kind, and format before review. Tauri projects the affected entry index and a label-based message, and the Inputs stage offers Relink and Clear without exposing the path in errors. Rust projection tests and the multi-file DOM regression cover missing-entry detection and direct repair.

### UX-037 — Deselecting a recipe leaves stale bindings that block validation

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A09`
- Proposed phase: `5D`
- Status: `Resolved 2026-07-21`

**Actual**

After selecting `Copy ROM library`, entering or retaining its values, and then deselecting the recipe, validation reports that `feature.copy_roms/policy` and `feature.copy_roms/source` are outside the selected recipe dependency set. The recipe is no longer selected, but its bindings remain active enough to produce blocking errors.

**User impact**

Removing a recipe creates new validation failures and prevents plan review. Users must understand internal dependency-set terminology or manually recover state that should have become irrelevant when the recipe was deselected.

**Evidence**

- Screenshots: User-provided A09 screenshots from 2026-07-18 showing both field-level and summary errors after `Copy ROM library` was deselected.
- Exact UI text: `Binding 'feature.copy_roms/policy' is outside the selected recipe dependency set.` and the corresponding `source` error.

**Notes for later implementation**

On recipe deselection, remove inactive bindings from authoritative validation input or retain them only as dormant remembered values that are excluded from validation and planning. Re-selecting the recipe may restore those remembered values, but they must not block unrelated configurations while inactive.

**Resolution evidence (2026-07-21)**

Recipe selection reconciliation removes bindings that no longer belong to the dependency-expanded active set while retaining bindings shared with active dependencies. The logic regression `deselecting a recipe removes its bindings without removing bindings for active dependencies` covers the documented stale-binding failure.

### UX-038 — Reconnecting the same device after a review-stage disconnect discards setup and input state

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A10`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Select a supported device.
2. Choose a setup and recipes.
3. Provide valid inputs and generate a plan.
4. Reach the Review stage.

**Reproduction**

1. Disconnect the selected device while the plan is visible.
2. Observe that EmuChef returns to Connect and invalidates execution.
3. Reconnect the same physical device.
4. Refresh and select it again.
5. Continue through the workflow.

**Expected**

Disconnecting must invalidate device authority, the generated plan, and execution readiness. When the same device reconnects, EmuChef should restore still-valid user intent such as the selected setup, recipes, and reusable input values, then require fresh device confirmation, validation, planning, and review. If restoration is unsafe, the app should clearly warn before discarding the state.

**Actual**

The disconnect safely returns the user to the Connect stage with `The selected device disconnected. Connect it again to continue.` After reconnecting and selecting the same device, the workflow starts over and the previously selected setup, recipes, and inputs are not restored.

**User impact**

A temporary cable interruption can discard substantial configuration work even though the same device returns. This makes review and execution fragile and discourages users from trusting the workflow with complex setups.

**Workaround**

Recreate the setup, recipe selections, and inputs manually after reconnecting.

**Evidence**

- Screenshot: User-provided A10 screenshot from 2026-07-18 showing the Connect stage and disconnect warning after leaving Review.
- Observation: reconnecting and selecting the same Pocket S Mini restarted the workflow without restoring prior data.

**Notes for later implementation**

Separate portable intent from ephemeral device authority. Preserve setup, recipe selections, and reusable input bindings across a temporary disconnect; invalidate the device snapshot, plan, review approval, and execution handles. Rebind preserved intent only after confirming the same device identity and rerunning validation. Do not silently restore device-specific or unsafe authority.

### UX-039 — The normal simulated-run baseline cannot reach a successful terminal result

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed + contract-supported`
- Scenario(s): `A11`, `A12`, `A13`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Select the supported Pocket S Mini setup.
2. Generate and review a valid RetroArch provisioning plan.
3. Start a simulated dry run without intentionally corrupting inputs or dependencies.

**Reproduction**

1. Allow the simulation to run to completion.
2. Repeat the run from a fresh valid review.
3. Inspect the terminal step results.

**Expected**

The standard valid setup should be capable of reaching `succeeded` or `succeeded_with_warnings`. Dry-run verification should simulate the expected post-step filesystem and app state closely enough for valid copy and launch dependencies to pass. Deliberate failure fixtures may remain available for failure-path testing, but ordinary end-user simulations must have a successful baseline.

**Actual**

No tested simulated run reached a fully successful terminal result. Expected RetroArch copy steps failed verification with internal messages such as `verify failed: path_exists, path_exists, path_exists` and `verify failed: file_exists`, after which `Launch RetroArch` was blocked by raw dependency identifiers.

The Phase 2A product contract explicitly supports `succeeded` and `succeeded_with_warnings` reports, so persistent failure is not an intentional requirement of simulation mode.

**User impact**

Users cannot use the advertised dry run to confirm that a valid setup is internally coherent. Every run ends in failure, making the feature appear broken and preventing the success, completion, and repeat-run paths from being meaningfully exercised.

**Workaround**

None within the current UI. The user can inspect the partial report, but cannot obtain successful simulated evidence for the standard setup.

**Evidence**

- Screenshots: User-provided A11 screenshots from 2026-07-18 showing progress, terminal verifier failures, and a blocked launch.
- Exact UI text includes `verify failed: path_exists, path_exists, path_exists`, `verify failed: file_exists`, and `dependency blocked: app.retroarch.provision/copy_cheats, app.retroarch.provision/copy_core_system_files`.
- Contract: `docs/product/phase-2a-simulated-execution.md` states that reports handle `succeeded` and `succeeded_with_warnings` in addition to failed and cancelled outcomes.

**Notes for later implementation**

Review the fake-device and dry-run verifier state transitions for extract, copy, and launch steps. Ensure successful simulated actions update the simulated filesystem and dependency state consumed by later verifiers. Add an end-to-end regression test proving that the canonical Pocket S Mini RetroArch setup reaches a successful terminal report, while retaining separate deterministic failure and cancellation fixtures.

**Resolution evidence (2026-07-21)**

The executor regression `resolve_extract_and_copy_flow_matches_compatibility_and_stays_in_sandbox` proves that successful extraction and copy actions materialize the fake-device filesystem consumed by later `file_exists` verification, which is the transition that caused the reported canonical failure. `repo_plan_e2e_normalized_steps_match_runtime_contract` separately proves the checked-in Pocket S Mini RetroArch plan's ordered resolve/extract/copy/launch dependency contract. The checked-in plan depends on remote release artifacts and is therefore not used as a network-dependent full execution fixture; no authored recipe/profile or executor behavior change was introduced solely for this audit.

### UX-040 — `Retry failed work` is mislabeled and does not retry the failed work

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A12`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Actual**

After a failed simulated run, the primary action is labeled `Retry failed work`. Selecting it does not retry failed steps or preserve the terminal report as an active retry context. Instead, EmuChef returns to the Inputs stage with `Configuration refreshed. Resolve any diagnostics, then create and review a new plan.`

**Expected**

The action label should accurately describe the operation. If failed steps cannot be retried safely from the retained report, the action should say `Repair setup` or `Create a fresh plan`, explain that prior successful simulated steps are only report evidence, and require a new review before another run.

**User impact**

The current label promises a direct retry but performs a full recovery workflow. Users cannot predict whether completed work will be reused, whether the same plan remains authoritative, or why they have been returned to Inputs.

**Evidence**

- User-provided A12 screenshots from 2026-07-18 showing the failed report, the `Retry failed work` button, and the resulting Inputs-stage notice.
- Exported report confirms each error remediation is `generate_fresh_plan`, not a retry of retained failed steps. fileciteturn2file0

**Notes for later implementation**

Rename the action to match the authoritative remediation. Preserve selected setup and reusable input intent, invalidate the failed review and execution authority, focus the first actionable diagnostic, and clearly state that a fresh plan and review are required.

**Resolution evidence (2026-07-21)**

Failed and cancelled reports expose `Repair setup`, preserve only reusable portable intent, and explain that a fresh plan and review are required and completed work is not retried in place. `ExecutionStep.dom.test.tsx` and the reducer regressions `repair keeps authoritative failed and cancelled labels while preserving safe intent` and `failed execution makes the retained review stale and blocks another start` cover the flow.

### UX-041 — Return to Review exposes the failed plan without clearly marking it stale

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A12`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Actual**

Selecting `Return to Review` from the failed report returns to the same prior review screen and plan digest. The screen looks ready to start another simulated run and does not prominently explain that the failed outcome requires repair and generation of a fresh plan.

**Expected**

Returning from a failed terminal report should either present the previous plan as read-only evidence or mark it clearly stale and non-executable. The UI should identify the failed features, direct the user to repair inputs or configuration, and require regeneration and review before another execution.

**User impact**

The user may interpret the old review as still valid and restart an unchanged plan that is known to fail. The distinction between reviewing historical evidence and approving a new authoritative plan is unclear.

**Evidence**

- User-provided A12 screenshot from 2026-07-18 showing the same review and plan digest after selecting `Return to Review`.
- The exported report classifies the execution as failed and assigns `generate_fresh_plan` remediation to all four reported errors. fileciteturn2file0

**Notes for later implementation**

Introduce an explicit stale/failed-review state. Disable execution from the old review, show a concise failure summary, and provide a single action that returns to the relevant setup/input fields while preserving portable intent.

**Resolution evidence (2026-07-21)**

Failed, cancelled, and unavailable simulations return only to a clearly labeled previous review. The stale-review alert states that the plan cannot run again, both execution controls are disabled, and repair requires a fresh validation and review. `ReviewStep.dom.test.tsx` covers the read-only stale state and `workflow.test.ts` covers failed-review invalidation.

### UX-042 — Report export success state persists across a different execution report

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A13`
- Proposed phase: `5E`
- Status: `Resolved 2026-07-21`

**Preconditions**

1. Complete and export a failed or cancelled simulated execution report.
2. Start or complete a different simulated execution so the visible report changes.

**Reproduction**

1. Export one execution report successfully.
2. Observe the export control change to `Report saved`.
3. Return to Review, re-execute the plan, or cancel a later run.
4. Inspect the export control on the new report.

**Expected**

Export confirmation should describe only the current report and should reset when a different execution starts or becomes the active terminal report. The control should return to `Export report` until the current report has been saved.

**Actual**

After exporting a report, the control remains labeled `Report saved` after cancelling and re-executing the plan. The label therefore refers to a prior report rather than the currently displayed execution.

**User impact**

The user may incorrectly believe the current cancelled or failed report has already been exported. This creates uncertainty about which execution evidence exists on disk.

**Workaround**

Click the stale `Report saved` control and export again. Existing filenames can be overwritten successfully.

**Evidence**

- Screenshot: User-provided A13 screenshot from 2026-07-18 showing `Report saved` on a later failed execution.
- Export behavior: cancelling the save dialog was silent; overwriting an existing report succeeded.
- Exported cancelled report: status `cancelled`, 3 completed steps, and 26 cancelled steps; absolute catalog paths are redacted and the target is omitted.
- Exported failed report: status `failed`, 25 completed steps, 3 failed steps, and 1 blocked step; absolute catalog paths are redacted and the target is omitted.

**Notes for later implementation**

Scope export state to the active execution handle and terminal report generation. Clear `saved` state whenever execution identity changes, a new run starts, or a newer snapshot replaces the displayed report. Consider showing the saved filename or a transient confirmation rather than permanently replacing the action label.

**Resolution evidence (2026-07-21)**

Export presentation identity is scoped to the execution generation and opaque execution handle, so ordinary sequence updates do not misidentify a report and a different run resets `Report saved`. `useExecution.dom.test.tsx` covers both identity reset and rejection of a late export completion from an older execution.

### UX-043 — Saving portable intent unexpectedly resets the workflow to Connect

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A15`
- Proposed phase: `5F`
- Status: `Open`

**Preconditions**

1. Select a supported device and setup.
2. Reach Inputs with valid recipe selections and portable input values.
3. The configuration has not yet been saved at its current path.

**Reproduction**

1. Select `Save` or `Save As...` from the Inputs stage.
2. Complete the native save dialog.
3. Observe the workflow stage after the save completes.

**Expected**

Saving portable intent should preserve the current workflow position and all still-valid session state. Device authority and generated plans may remain transient, but a normal save should not behave like reopening the configuration or starting over.

**Actual**

After saving from Inputs, the application returns to Connect and asks the user to select the current device again. The configuration is saved, but the active workflow context is discarded.

**User impact**

Saving interrupts the primary workflow and forces the user to repeat device selection and setup confirmation. It makes a routine persistence action feel destructive and discourages saving work in progress.

**Workaround**

Reconnect or reselect the same device and proceed through the workflow again.

**Evidence**

- Screenshot: User-provided A15 screenshot from 2026-07-18 showing the saved `Pocket S Mini base` configuration active while the workflow has returned to Connect.
- Exact UI text: `Opened Pocket S Mini base. Connect and select the current device to validate it.`

**Notes for later implementation**

Separate save completion from open/reload behavior. Preserve device selection, setup choice, recipes, inputs, and current stage when they remain valid; invalidate only generated plan and execution authority when required by the persistence contract.

### UX-044 — Invalid or incompatible saved configurations remain actionable before being blocked

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A15`
- Proposed phase: `5F`
- Status: `Open`

**Preconditions**

1. Create or edit a configuration so it contains unknown recipe, binding, or device-plan references.
2. Open the configuration through Recents or `Open...`.

**Reproduction**

1. Open the malformed or catalog-incompatible configuration.
2. Observe its recent-file status and compatibility warnings.
3. Select an available device and continue.
4. Observe the later Setup-stage failure.
5. Expand diagnostic details.

**Expected**

A configuration that cannot be used with the current catalog should be clearly blocked or placed into a guided repair state before device selection advances. Diagnostics should identify user-facing missing recipes or setup compatibility without exposing raw catalog internals by default.

**Actual**

The configuration is marked `requires Attention`, but device selection remains available and the user can proceed. The next stage then reports that the saved device plan is unavailable or incompatible. Diagnostics expose raw recipe IDs, device-plan IDs, binding IDs, internal result codes such as `unknown_recipe` and `device_plan_not_found`, and the complete list of available device-plan IDs.

**User impact**

The user is allowed to invest effort in a configuration that is already known to be unusable. Recovery is delayed and framed in implementation terminology rather than actionable choices such as replacing a missing recipe or selecting a supported setup.

**Workaround**

Manually edit the YAML or abandon the configuration and recreate it using current catalog entries.

**Evidence**

- Screenshots: User-provided A15 screenshots from 2026-07-18 showing `requires Attention`, raw recipe and device-plan diagnostics, and the later incompatible-device-plan message.
- Exact UI text includes `Selected recipe ... was not found`, `Unknown device plan`, `references an unknown recipe`, and `The saved device plan reference is unavailable or incompatible with this current device.`

**Notes for later implementation**

Introduce a compatibility gate and repair workflow when opening saved configurations. Keep the document open for inspection, but prevent normal progression until blocking references are repaired or explicitly removed. Present friendly names and concise recovery actions; reserve raw IDs and codes for expandable technical details.

### UX-045 — Saved configurations lack first-class rename and duplicate actions

- Type: `Missing MVP feature`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A16`
- Proposed phase: `5F`
- Status: `Open`

**Preconditions**

1. Open a valid saved configuration.
2. Inspect the configuration panel, Recents list, application menus, and available context actions.

**Reproduction**

1. Attempt to rename the active configuration within EmuChef.
2. Attempt to duplicate it without replacing the current file.
3. Use `Save As...` as the only available substitute.
4. Rename the YAML file externally and reopen it.

**Expected**

A reusable-configuration workflow should provide explicit Rename and Duplicate actions. The resulting configuration identity should be understandable, and multiple files with the same internal title should be distinguishable in Recents.

**Actual**

There is no Rename or Duplicate action. `Save As...` is the only duplication mechanism. The displayed configuration name comes from the YAML name field rather than the filename, so externally renaming the file does not change its visible identity. Copies retain the same visible name unless the YAML title is edited separately.

**User impact**

Users cannot organize reusable setups entirely within the application. Duplicate configurations can appear identical in Recents, and changing a filename outside EmuChef does not clarify which copy is active.

**Workaround**

Use `Save As...`, then edit the YAML name field outside EmuChef before reopening the copy.

**Evidence**

- Screenshots: User-provided A16 screenshots from 2026-07-18 showing only `Save As...` and showing a file whose visible title follows the edited YAML name.
- Observed behavior: changing only the external filename does not change the title shown in EmuChef.

**Notes for later implementation**

Add explicit Duplicate and Rename actions. Decide and document the identity model: the internal display name may remain authoritative, but Recents should also expose enough filename or location context to distinguish files with duplicate titles. Rename should clearly specify whether it changes the internal title, filename, or both.

### UX-046 — Successful Save is paired with misleading disabled-state guidance

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A17`
- Proposed phase: `5F`
- Status: `Open`

**Preconditions**

1. Open a saved configuration with a selected device plan.
2. Change and relink one or more portable inputs.
3. Save the updated configuration successfully.

**Reproduction**

1. Resolve missing file or directory inputs.
2. Select `Save`.
3. Observe the success notice and the disabled Save guidance shown in the configuration panel.

**Expected**

After a successful save, the interface should clearly confirm completion. Any disabled-state explanation should be neutral and should not imply that the just-completed save failed or remains incomplete.

**Actual**

The configuration saves and a success notice appears, but the panel simultaneously states `Save requires a selected device plan and unsaved portable changes.` The message reads like an unmet requirement or failure even though the save completed correctly.

**User impact**

The user may believe the configuration was not saved, repeat the operation, or distrust the current document state.

**Workaround**

Infer success from the separate `Saved ...` notice and the absence of unsaved-edits status.

**Evidence**

- Screenshot: User-provided A17 screenshot from 2026-07-18 showing the successful save notice alongside the disabled-state guidance.
- Exact disabled-state text: `Save requires a selected device plan and unsaved portable changes.`
- Both missing inputs were detected; resolving one left only the other unresolved, and resolving both allowed saving.

**Notes for later implementation**

Change the post-save disabled explanation to a neutral status such as `Configuration saved. Save becomes available after another change.` Keep requirement-oriented copy only for states where the current document has never been eligible to save.

### UX-047 — Support diagnostics retains stale export-success state across modal sessions

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always after a prior successful export`
- Evidence: `Observed`
- Scenario(s): `A19`, `A22`
- Proposed phase: `5G`
- Status: `Open`

**Preconditions**

1. Open Support & Storage.
2. Export support diagnostics successfully.
3. Close the modal.

**Reproduction**

1. Reopen Support & Storage later without exporting a new archive.
2. Observe the Support diagnostics section.

**Expected**

Operation feedback should describe only the current modal session or current export attempt. A new modal session should begin without a success message unless a new export completes.

**Actual**

The modal continues to display `Success: diagnostics saved.` from the earlier export. The stale message remains visible while reviewing unrelated cache information and after other application activity.

**User impact**

The user can reasonably interpret the message as confirmation that a new diagnostic archive was just created, even though no export occurred in the current session.

**Workaround**

Ignore the success banner and use the native save dialog or filesystem to verify whether a new archive was actually created.

**Evidence**

- Screenshot: User-provided A22 constrained-window screenshot from 2026-07-18 showing `Success: diagnostics saved.` immediately after reopening Support & Storage.
- Exact UI text: `Success: diagnostics saved.`

**Notes for later implementation**

Reset export status when the modal opens or closes, and whenever a new export begins or is cancelled. Scope success state to the specific export operation rather than retaining it as persistent support-panel state.

### UX-048 — Runtime restart discards active workflow state and portable intent

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A27`
- Proposed phase: `5B`
- Status: `Resolved`

**Preconditions**

1. Progress beyond Connect with a selected device, setup, recipes, and inputs, or generate a reviewed plan.
2. Ensure no unrelated failure is active.

**Reproduction**

1. Select `Restart runtime`.
2. Observe the workflow after the Rust runtime reports ready again.

**Expected**

Restart should invalidate runtime-owned authority and generated plans, but preserve portable user intent where safe. Before discarding unsaved setup, recipe, or input changes, the app should either preserve them automatically or require explicit confirmation.

**Actual**

Restart always returns the application to Connect and shows `Rust runtime restarted. Reopen a portable configuration before continuing.` Active device selection, setup choice, recipe selections, input values, and review position are cleared. A saved configuration must be reopened, and unsaved portable intent has no recovery path.

**User impact**

A troubleshooting action can erase substantial setup work. The user cannot directly resume or execute a previously reviewed plan after restart, even when only the runtime process needed recovery.

**Workaround**

Save the configuration before restarting, then reopen it and repeat device selection, validation, and plan review. Unsaved changes cannot be recovered.

**Evidence**

- Screenshot: User-provided A27 screenshot from 2026-07-18 showing the post-restart Connect screen and the notice `Rust runtime restarted. Reopen a portable configuration before continuing.`
- Observed behavior: restart consistently returns to Connect and prevents direct execution of the prior plan.

**Notes for later implementation**

Separate portable frontend intent from runtime-owned authority. Preserve configuration identity, setup, recipes, and input bindings across a runtime restart; clear device authority and generated plans only. Add a confirmation when unsaved portable state cannot be preserved. Continue rejecting stale async responses from the prior runtime generation.

**Resolution evidence**

- Commit `8f274ae4faeb80d7c34df1e6cf7f3445ceb8db29` introduced the committed UX-048 restart baseline: recovery staging, omission confirmation, portable-intent restoration, runtime-authority invalidation, and runtime-generation guards.
- DOM regression coverage exercises clean restart, cancellation with dirty omitted values, confirmed nonsensitive restoration, sanitized sensitive re-entry messaging, stale pre-restart device responses, and restart failure settlement.
- Logic and Tauri recovery tests prove that generated review/execution authority is cleared, restored intent contains only portable fields, and persisted recovery records omit sensitive values and transient authority fields.

### UX-049 — Exact device matches hide other applicable setup choices

- Type: `Missing MVP feature`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `Post-5B packaged-app smoke test`
- Proposed phase: `5C`
- Status: `Open`

**Actual**

When EmuChef finds an exact device match, the Setup stage presents only the directly matched setup. Other applicable setup plans are not shown.

**Expected**

The exact match should remain recommended and preselected, but the user should still be able to review and choose other backend-approved plans that are applicable to the detected device.

**User impact**

Users cannot intentionally choose a broader, alternate, or differently scoped setup even when the catalog marks it applicable to the device.

**Evidence**

- Screenshot: User-provided packaged-app screenshot from 2026-07-19 showing only `AYANEO Pocket S Mini Base Setup` after an exact Pocket S Mini match.

**Notes for later implementation**

Phase 5C should distinguish `recommended`, `applicable`, and `generic` plans. Keep the exact match prominent and selected by default, but do not suppress other backend-authoritative applicable choices.

### UX-050 — Setup selection lacks a blank configuration-from-scratch option

- Type: `Missing MVP feature`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `Post-5B packaged-app smoke test`
- Proposed phase: `5C`
- Status: `Open`

**Actual**

The Setup stage requires choosing a catalog-defined setup plan. There is no blank setup that lets the user begin with no recipes selected and build a configuration from scratch.

**Expected**

Offer a clearly labeled blank setup, such as `Start from scratch`, that creates an empty recipe selection while retaining the detected device context and normal compatibility validation.

**User impact**

Advanced users and users with narrow goals must begin from a predefined setup and remove unwanted selections instead of starting from a minimal empty configuration.

**Evidence**

- Screenshot: User-provided packaged-app screenshot from 2026-07-19 showing a single predefined setup and no blank option.

**Notes for later implementation**

The blank setup must remain backend-authoritative as an explicit empty-plan starting mode, not a frontend-invented device plan. Empty recipe selection should remain valid during customization, while Review stays unavailable until at least one recipe is selected.

### UX-051 — Runtime restart restoration uses technical and unclear recovery language

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always after restart with preserved unsaved setup choices`
- Evidence: `Observed`
- Scenario(s): `Post-5B packaged-app smoke test`
- Proposed phase: `5H`
- Status: `Resolved`

**Actual**

After runtime restart, the UI labeled the state `Recovered configuration` and displayed: `The recovered intent is unsaved and will not overwrite its source until you save. Connect a device and validate again.` The terms `recovered intent`, `source`, and the distinction between restoration and overwrite behavior were technical and unclear.

**Expected**

The UI should plainly explain that EmuChef restored the user's setup choices, that device selection and review must be repeated, and whether those restored changes have been saved.

**User impact**

Users may think a damaged file was recovered, may not understand what was restored, and may be uncertain whether Save will update an existing configuration or create a new one.

**Evidence**

- Screenshot: User-provided packaged-app screenshot from 2026-07-19 showing `Recovered configuration` and the recovery notice after runtime restart.

**Notes for later implementation**

Prefer language such as `Setup restored after restart` and `Your setup choices were restored, but they have not been saved. Select your device and review the setup again before continuing.` When a saved configuration was reopened successfully, identify it by name and state that unsaved changes were restored without using `intent`, `source`, `authority`, or `recovery draft` terminology.

**Resolution evidence**

The restored state now uses `Setup restored after restart` for unsaved setup choices and identifies reopened saved configurations by name. The notice explains that setup choices were restored, whether they have been saved, and that device selection and review must be repeated. DOM coverage rejects the former `recovered intent` and `source` wording.

## 8. Prioritized backlog

Phase 5B completed on 2026-07-19. Its foundational workflow-state findings are resolved in the main application and covered by app-local logic, DOM, security, TypeScript, lint, build, Tauri, and backend verification. Phase 5C is next.

### 8.1 Phase 5B — Workflow navigation and state polish — `Completed`

#### Defects

- `UX-002`: Workflow heading receives unexpected initial focus.
- `UX-023`: Step 4 is mislabeled `Inputs` despite combining recipe selection and input collection.
- `UX-026`: Platform-Tools import reports validation before file selection.
- `UX-027`: Platform-Tools replacement lacks in-progress wording.
- `UX-028`: Successful Platform-Tools replacement reports a false device disconnect.
- `UX-029`: Platform-Tools removal has no confirmation and uses internal authority terminology.
- `UX-030`: Device refresh gives no progress or completion feedback.
- `UX-032`: Unauthorized devices are indistinguishable from no device.
- `UX-033`: Unsupported devices are presented as available and skip the Device stage.
- `UX-038`: Reconnecting the same device after a disconnect discards setup and input state.
- `UX-048`: Runtime restart discards active workflow state and portable intent.

#### Missing MVP features

- `UX-031`: The no-device state lacks practical USB-debugging and authorization guidance.

#### Optional enhancements

- None identified by this audit.

#### Implementation evidence

- `UX-002` and `UX-023`: startup presentation no longer moves focus to the workflow heading, and Step 4 is labeled `Customize` without implementing the deferred recipe/input split.
- `UX-026` through `UX-032`: Platform-Tools picker, processing, replacement, removal, device refresh, connection guidance, and ADB authorization states have distinct UI and operation semantics.
- `UX-033`: unsupported devices remain in the Device stage until the session-only warning is acknowledged; acknowledgment does not select a plan, and React exposes the backend `safeGenericPlans` list unchanged.
- `UX-038`: Tauri retains at most 32 process-local serial-to-handle identities, while React preserves only portable setup, recipes, and backend-classified nonsensitive bindings across same-device reconnect. Different-device continuation requires confirmation and starts fresh.
- `UX-048`: restart stages clean or dirty portable recovery intent, confirms backend-reported omissions using friendly labels or a count, resets every runtime-owned authority surface, and rejects prior-runtime responses.
- Regression coverage is in `apps/emuchef-app/tests/workflow.test.ts`, `tests/App.dom.test.tsx`, `tests/security-policy.test.mjs`, and the Tauri `commands`, `handles`, and `recovery` unit modules.
- Required verification passed on 2026-07-19. The full backend suite passed with 548 tests and 7 ignored; the previously observed `tests/editor_sessions.rs` parallel-only baseline failure did not reproduce in the final full-suite run.
- UX-048 was revalidated on 2026-07-21 with explicit clean, cancel, continue, omission, stale-response, and failure regressions; the EmuChef proper frontend, security, Tauri, and shared Rust backend suites passed.
- Manual real-device timing and packaged-GUI focus behavior remain follow-up qualification risks, not blockers to the code-level Phase 5B acceptance criteria.

### 8.2 Phase 5C — Recipe and setup selection experience

#### Defects

- `UX-013`: Review Plan remains enabled when no recipes are selected.

#### Missing MVP features

- `UX-024`: Recipe discovery and recipe-specific inputs need separate scalable stages.
- `UX-049`: Exact device matches should still show other backend-approved applicable setup plans.
- `UX-050`: Setup selection needs a backend-authoritative blank `Start from scratch` option.

#### Optional enhancements

- None identified by this audit.

### 8.3 Phase 5D — Input collection and file-management polish

#### Defects

- `UX-014` (resolved 2026-07-21): Selecting a recipe immediately presents a blocking error.
- `UX-015` (resolved 2026-07-21): Validation exposes internal binding identifiers and codes.
- `UX-016` (resolved 2026-07-21): Device destination paths incorrectly use a host Browse control.
- `UX-035` (resolved 2026-07-21): Selected optional file inputs cannot be cleared.
- `UX-036` (resolved 2026-07-21): Missing selected files produce generic validation failures.
- `UX-037` (resolved 2026-07-21): Deselecting a recipe leaves stale bindings that block validation.

#### Missing MVP features

- None identified by this audit.

#### Optional enhancements

- None identified by this audit.

### 8.4 Phase 5E — Plan review and execution experience

#### Defects

- `UX-019` (resolved 2026-07-21): Simulation timestamps used raw ISO values.
- `UX-021` (resolved 2026-07-21): Simulation failure summaries were generic and non-actionable.
- `UX-022` (resolved 2026-07-21): Execution reports exposed raw verifier and dependency identifiers.
- `UX-034` (resolved 2026-07-21): Plan review exposed bindings, private host paths, capability tokens, and the plan digest.
- `UX-039` (resolved 2026-07-21): The normal simulation filesystem transition prevented a successful terminal baseline.
- `UX-040` (resolved 2026-07-21): `Retry failed work` did not retry and was mislabeled.
- `UX-041` (resolved 2026-07-21): Return to Review exposed the same failed plan without a strong stale state.
- `UX-042` (resolved 2026-07-21): Report-export success persisted across a different execution report.

#### Missing MVP features

- None identified by this audit.

#### Optional enhancements

- None identified by this audit.

### 8.5 Phase 5F — Saved configurations and reusable setups

#### Defects

- `UX-011`: Configuration management is embedded in the primary workflow instead of native menus.
- `UX-017`: Save terminology does not clearly explain persisted contents.
- `UX-043`: Saving from Inputs unexpectedly resets the workflow to Connect.
- `UX-044`: Invalid or incompatible saved configurations remain actionable too long.
- `UX-046`: Successful Save is paired with misleading disabled-state guidance.

#### Missing MVP features

- `UX-045`: Saved configurations lack first-class Rename and Duplicate actions.

#### Optional enhancements

- None identified by this audit.

### 8.6 Phase 5G — Support, diagnostics, and recovery polish

#### Defects

- `UX-003`: Normal Cmd+Q termination is reported as an unexpected shutdown.
- `UX-006`: Cache refresh leaves stale operation notices visible.
- `UX-007`: Cache notifications expose internal result codes.
- `UX-008`: Bulk cache-clear actions remain enabled when the cache is empty.
- `UX-010`: Platform-Tools maintenance actions are persistently exposed in the primary workflow.
- `UX-012`: Restart Runtime is exposed as a primary workflow action.
- `UX-047`: Support diagnostics retains stale export-success state across modal sessions.

#### Missing MVP features

- None identified by this audit.

#### Optional enhancements

- None identified by this audit.

### 8.7 Phase 5H — Visual consistency and final product polish

#### Defects

- `UX-001`: Unsaved-configuration panel has a broken horizontal layout.
- `UX-004`: Runtime status badge exposes an internal catalog identifier.
- `UX-005`: Application uses the wrong icon.
- `UX-009`: System Status exposes the implementation language.
- `UX-018`: Updates dialog uses a non-standard Close control.
- `UX-020`: Simulation failure cards have insufficient visual separation.
- `UX-025`: Platform-Tools setup copy is overly technical.

#### Missing MVP features

- None identified by this audit.

#### Optional enhancements

- None identified by this audit.

## 9. Run log

| Run | Date | Commit | Scenarios | Result | Notes |
|---|---|---|---|---|---|
| 1 | Pending | Pending | Pending | Pending | Initial first-user audit |

## 10. Exit criteria

- [x] Every primary user workflow has been exercised in the running application.
- [x] Every scenario in the matrix is marked `Passed`, `Findings`, `Blocked`, or `Not applicable` with a reason.
- [x] High-friction points have reproducible steps and impact statements.
- [x] Findings include type, severity, frequency, evidence level, and proposed owning phase.
- [x] Defects, missing MVP features, and optional enhancements are explicitly separated.
- [x] The prioritized Phase 5B–5H backlog is populated.
- [x] The next implementation phase is selected from audit evidence.
- [x] `docs/product/phase-5-app-quality-roadmap.md` is updated to mark 5A completed and Phase 5B as `Next`.
