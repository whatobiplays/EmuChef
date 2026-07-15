# Phase 5A End-to-End UX and Feature-Gap Audit

## 1. Status

- Phase: `5A`
- Status: `In progress`
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
| A02 | Platform-Tools import | Valid current macOS ZIP; cancellation; invalid ZIP; unsupported/older ZIP where available | Not run | — | — |
| A03 | Platform-Tools replacement and removal | Existing valid managed installation; failed replacement preservation; successful replacement | Not run | — | — |
| A04 | No device connected | Runtime/catalog available; device list empty; refresh/redetection | Not run | — | — |
| A05 | Unauthorized device | One ADB device awaiting authorization | Not run | — | — |
| A06 | Unsupported device | One connected device that matches no supported plan | Not run | — | — |
| A07 | Supported device detection and confirmation | Supported target; disconnect/reconnect; identity change where practical | Findings | UX-002 | Initial workflow heading receives unexpected focus on window appearance. |
| A08 | Setup and recipe selection | Recommended path; manual selection; incompatible/dependent/conflicting recipes | Not run | — | — |
| A09 | Input collection | Missing, invalid, sensitive, moved, multi-file, and optional values | Not run | — | — |
| A10 | Plan generation and review | Valid setup; invalidated/stale review; back navigation | Not run | — | — |
| A11 | Simulated execution | Successful run; progress; cancellation; completion | Not run | — | — |
| A12 | Partial failure and retry | Retryable and non-retryable outcomes where supported | Not run | — | — |
| A13 | Completion report | Succeeded, failed, skipped, needs-attention, export/display behavior | Not run | — | — |
| A14 | Save and Save As | New unsaved setup; overwrite prompts; naming; cancelled native dialogs | Findings | UX-001 | Unsaved-configuration action panel has a visibly broken and difficult-to-scan layout. |
| A15 | Reopen and recent files | Valid recent file; missing file; malformed file; stale catalog references | Not run | — | — |
| A16 | Rename and duplicate saved configuration | Where supported; verify identity and dirty-state behavior | Not run | — | — |
| A17 | Relink moved inputs | Saved configuration with moved or missing file/directory bindings | Not run | — | — |
| A18 | Dirty-intent close and crash recovery | Normal close, forced termination, Restore, Discard, Not now, sensitive re-entry | Findings | UX-003 | Normal Cmd+Q is reported as an unexpected prior shutdown on next launch. |
| A19 | Support diagnostics | Runtime available/unavailable; export success/cancel/failure; disclosure clarity | Not run | — | — |
| A20 | Cache inventory and cleanup | Empty cache; removable entry; in-use entry; cleanup cancellation/failure | Findings | UX-006, UX-007 | Cache refresh leaves stale success notices visible; technical details expose internal result codes. |
| A21 | Update panel | Production trust unconfigured; repeated check; external-navigation state | Not run | — | — |
| A22 | Narrow window and high zoom | Minimum window; 200% zoom; long text; wrapping; no page-level horizontal scroll | Not run | — | — |
| A23 | Keyboard-only workflow | Full primary workflow; dialogs; native-dialog return; visible focus; skip link | Findings | UX-002 | Initial focus appears on the workflow heading without user navigation. |
| A24 | Screen-reader workflow | Headings, landmarks, fieldsets, summaries, live announcements, dialogs | Not run | — | — |
| A25 | Reduced motion | OS preference enabled; transitions and progress remain understandable | Not run | — | — |
| A26 | Forced colors / increased contrast | Supported browser/WebView and macOS contrast settings | Not run | — | — |
| A27 | Runtime restart and stale async responses | Restart during safe idle state and during pending frontend work where supported | Not run | — | — |
| A28 | Start Over and backward navigation | Every workflow stage; dirty and clean intent; expected preservation/loss | Not run | — | — |

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
- Status: `Open | Needs reproduction | Deferred | Resolved as audit blocker`

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
- Scenario(s): `A07`, `A23`
- Proposed phase: `5B`
- Status: `Open`

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
- Scenario(s): `A18`
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
- Status: `Open`

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

### UX-015 — Validation UI exposes internal binding identifiers and error codes

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A09`
- Proposed phase: `5D`
- Status: `Open`

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

#### UX-016 — Device destination path incorrectly uses a host filesystem Browse control

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A09`
- Proposed phase: `5D`
- Status: `Open`

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
- Status: `Open`

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
- Status: `Open`

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

### UX-022 — Execution report exposes raw verifier and dependency identifiers

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A11`, `A12`, `A13`
- Proposed phase: `5E`
- Status: `Open`

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

### UX-019 — Simulated-run start time is displayed as a raw ISO timestamp

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A11`, `A13`
- Proposed phase: `5E`
- Status: `Open`

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

### UX-020 — Simulated-run failure cards have insufficient visual separation

- Type: `Defect`
- Severity: `S3 — Minor`
- Frequency: `Always`
- Evidence: `Observed`
- Scenario(s): `A12`, `A13`
- Proposed phase: `5E`
- Status: `Open`

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
- Status: `Open`

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

### UX-022 — `Retry failed work` implies immediate retry but returns the user to planning

- Type: `Defect`
- Severity: `S2 — Major`
- Frequency: `Always`
- Evidence: `Observed + code-supported`
- Scenario(s): `A12`
- Proposed phase: `5E`
- Status: `Open`

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
- Status: `Open`

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

## 8. Prioritized backlog

This section is populated after scenario execution. Findings must remain separated by type even when they target the same later phase.

### 8.1 Phase 5B — Workflow navigation and state polish

#### Defects

- Pending audit evidence.

#### Missing MVP features

- Pending audit evidence.

#### Optional enhancements

- Pending audit evidence.

### 8.2 Phase 5C — Recipe and setup selection experience

#### Defects

- Pending audit evidence.

#### Missing MVP features

- Pending audit evidence.

#### Optional enhancements

- Pending audit evidence.

### 8.3 Phase 5D — Input collection and file-management polish

#### Defects

- Pending audit evidence.

#### Missing MVP features

- Pending audit evidence.

#### Optional enhancements

- Pending audit evidence.

### 8.4 Phase 5E — Plan review and execution experience

#### Defects

- Pending audit evidence.

#### Missing MVP features

- Pending audit evidence.

#### Optional enhancements

- Pending audit evidence.

### 8.5 Phase 5F — Saved configurations and reusable setups

#### Defects

- Pending audit evidence.

#### Missing MVP features

- Pending audit evidence.

#### Optional enhancements

- Pending audit evidence.

### 8.6 Phase 5G — Support, diagnostics, and recovery polish

#### Defects

- Pending audit evidence.

#### Missing MVP features

- Pending audit evidence.

#### Optional enhancements

- Pending audit evidence.

### 8.7 Phase 5H — Visual consistency and final product polish

#### Defects

- `UX-001`: Unsaved-configuration panel has a broken horizontal layout.
- `UX-004`: Runtime status badge exposes an internal catalog identifier.

#### Missing MVP features

- Pending audit evidence.

#### Optional enhancements

- Pending audit evidence.

## 9. Run log

| Run | Date | Commit | Scenarios | Result | Notes |
|---|---|---|---|---|---|
| 1 | Pending | Pending | Pending | Pending | Initial first-user audit |

## 10. Exit criteria

- [ ] Every primary user workflow has been exercised in the running application.
- [ ] Every scenario in the matrix is marked `Passed`, `Findings`, `Blocked`, or `Not applicable` with a reason.
- [ ] High-friction points have reproducible steps and impact statements.
- [ ] Findings include type, severity, frequency, evidence level, and proposed owning phase.
- [ ] Defects, missing MVP features, and optional enhancements are explicitly separated.
- [ ] The prioritized Phase 5B–5H backlog is populated.
- [ ] The next implementation phase is selected from audit evidence.
- [ ] `docs/product/phase-5-app-quality-roadmap.md` is updated to mark 5A completed and the selected next phase as `Next`.
