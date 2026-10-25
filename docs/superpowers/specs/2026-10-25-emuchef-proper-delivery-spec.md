# EmuChef Proper — Reviewed Setup and OEM-Assisted Execution Specification

**Product decisions:** Q1–Q83 and UX-1 A accepted.  
**Status:** Implementation specification for documentation and tracking; **not implementation, qualification, production-execution or release authorization**.  
**Scope:** Initial Apple Silicon macOS EmuChef proper release, shared trusted Rust/Tauri runtime and configured Android targets.  
**Companion authority:** The normative product requirements are in `docs/product/emuchef-requirements.md`; architectural decisions and rationale are in `docs/adr/` (ADRs 0005–0011); current implemented runtime behavior is described by `docs/architecture/planner-executor.md` and the related `docs/architecture/` contracts; outstanding qualification obligations are inventoried in `docs/product/qualification-obligations.md` (original evidence and validators remain decisive). These are **repository-local paths**, not remote GitHub blob links; several source documents remain uncommitted. This specification translates those authorities into testable delivery seams and does **not** supersede them.

## Problem Statement

An Android handheld owner needs a coherent way to install and configure emulators, choose ROM and BIOS locations, and apply recommended app settings without understanding Recipe IDs, Android app-private filesystem conventions or root commands. The current EmuChef runtime has trustworthy App Definitions, typed Recipes, a plan/review/executor foundation and a development qualification harness, but **does not yet provide the accepted generated-configuration, safe-update, portable-storage and OEM-assisted workflow as a qualified end-user product**.

Non-rooted handhelds may include a manufacturer menu that lets the owner invoke a script as root without providing EmuChef with direct ADB root. EmuChef must support this without pretending ADB has gained root, bypassing Recipe preconditions, accepting an unreviewed overwrite, or allowing a retained script to execute after its reviewed stage has closed. An uncertain second invocation cannot be mistaken for first-attempt success; failed staging cleanup cannot independently turn a reconciled setup into failure.

The previous simulation-first roadmap, Config Editor release evidence and registered device profiles are not equivalent to implementation or physical qualification of this new EmuChef proper workflow. Work and evidence must be traceable through one authoritative product contract and the accepted [23-ticket map](https://github.com/whatobiplays/EmuChef/issues/43).

## Solution

Provide one guided **Set up app** journey on Apple Silicon macOS. EmuChef discovers a selected connected Android device and its Device Profile, suggests compatible app/device settings, resolves logical internal or portable-SD Setup Locations, prepares typed configuration and content operations, presents the exact reviewed effects and applies only with separately authorized, qualified execution. Simulation is optional; no user-authored shell execution exists.

Keep App Definitions as the authority for application identity, App Artifacts, App Targets, permission sets and managed configuration schema. Keep Recipes as typed, ordered procedures with per-app exposed settings and declared base configuration source. Device Profiles own device capabilities, including a distinct **OEM Root Script Runner** capability. The trusted Rust/Tauri runtime compiles the immutable reviewed Execution Plan, resolves target and device authority, stages inputs, and reports verified, uncertain, failed and cleanup-pending outcomes. The frontend displays sanitized projections and cannot change the plan or device independently.

When direct ADB root is unavailable at planning time and the applicable Device Profile confirms an OEM runner, compile the same supported typed privileged actions into a user-invoked reviewed OEM bundle. Preserve the same mandatory file preconditions, backup mode, dependencies and result semantics. Permit safe serial reruns **only while the bundle's device-enforced admission window remains open and its OEM stage barrier is held**. Make admission and closure atomic relative to actual script start; prove closure, reconcile all admitted/possibly started attempts, and establish none remains active/uncertain **before** barrier release. A retained script of a closed or revoked bundle must refuse mutation, even after failed cleanup. After proven closure, cleanup failures remain Q81 C warnings with retry, not setup failures.

## User Stories

1. As an Android handheld owner, I want to connect one device and see its trustworthy identity and setup support, so that I do not configure the wrong handheld.
2. As an owner, I want clear guidance when Android Platform-Tools are missing, so that I can supply the supported tools myself.
3. As an owner, I want an installation and configuration journey in one place, so that I need not learn separate internal workflows.
4. As an owner, I want to see compatible app and device recommendations, so that I can start from suitable defaults.
5. As an owner, I want explicit settings to override recommendations, so that my valid choice remains authoritative.
6. As an owner, I want to restore a device recommendation, so that I can undo my own customization deliberately.
7. As an owner, I want clear notices when device matches conflict, so that I do not accidentally accept an arbitrary profile.
8. As an owner, I want to choose internal storage or an available removable SD card, so that content is placed where I expect.
9. As an owner, I want to save logical Setup Locations, so that changing a device's mount path does not silently retarget content.
10. As an owner, I want notice when the selected SD card is removed or replaced, so that the old plan is not replayed against another volume.
11. As an owner, I want to select ROMs, BIOS and required app resources, so that the emulator has complete usable inputs.
12. As an owner, I want Add/Update to preserve unselected destination content, so that setup does not erase unrelated files.
13. As an owner, I want Replace to affect only the selected target boundary, so that sibling files remain untouched.
14. As an owner, I want Mirror to show exact planned deletions, so that destructive synchronization is explicit and bounded.
15. As an owner, I want optional simulation of the reviewed plan, so that I can inspect an expected result without making it mandatory.
16. As an owner, I want app-specific settings rather than raw configuration editing, so that I can configure an emulator safely.
17. As an app author, I want App Definitions to own typed managed settings and targets, so that Recipes do not duplicate intrinsic application facts.
18. As a Recipe author, I want a base configuration file, typed substitutions and direct property overrides, so that reusable Recipes remain declarative.
19. As an owner, I want unknown untouched settings and comments in an existing supported config preserved, so that EmuChef edits do not erase my other choices.
20. As an owner, I want incompatible templates, ambiguous keys and unsafe substitutions rejected before mutation, so that invalid configuration is never applied.
21. As an owner, I want multiple selected Recipes to result in one exact reviewed artifact per destination/stage, so that overlapping writers cannot overwrite each other unpredictably.
22. As an owner, I want a newly created configuration file to be published only if its target is still absent, so that first-run setup cannot overwrite a file that appeared after review.
23. As an owner, I want stale existing-file state detected and a fresh review requested, so that delayed execution does not silently replace another change.
24. As an owner, I want installation to succeed independently of later configuration, so that partial success is reported truthfully.
25. As an owner, I want newly initialized app defaults inspected before configuration, so that my final review includes what the app actually created.
26. As an owner, I want a running app to be prepared or stopped through a reviewed step, so that settings are applied against an accurate post-preparation snapshot.
27. As an owner, I want my original config backed up by default before an existing-file overwrite, so that I have a recovery path.
28. As an owner, I want backup failure to stop before overwriting, so that the script cannot turn failed protection into unapproved data loss.
29. As an owner, I want to consent or decline to a specific file/execution waiver after a backup failure, so that the choice is informed and cannot silently apply to another write.
30. As an owner, I want an explicit persistent backup opt-out, so that I can accept that risk once in Advanced Settings without repeated waivers.
31. As an owner, I want all invocations of a reviewed script bundle to retain its frozen backup mode and bytes, so that preference changes cannot retroactively alter an already staged action.
32. As an owner, I want restoration to use retained original bytes and a new review, so that recovery never assumes all previous actions can be rolled back automatically.
33. As an owner of a non-rooted AYN, Retroid or AYANEO handheld with the OEM runner, I want EmuChef to prepare a supported root script, so that I can configure protected app storage without enabling direct ADB root.
34. As an owner, I want exact on-device instructions and an awaiting-action state, so that I know when I must manually run the manufacturer script.
35. As an owner, I want normal ADB work and root-script stages sequenced according to Recipe dependencies, so that no successor runs while privileged work is uncertain.
36. As an owner, I want safe serial reruns of the same reviewed bundle while its stage remains open, so that I can recover from a harmless interrupted attempt without an arbitrary permanent one-shot restriction.
37. As an owner, I want a racing invocation to be admitted before closure or rejected before mutation, so that closing the stage cannot leave untracked privileged writes.
38. As an owner, I want a retained script from a closed, cancelled or superseded bundle to refuse mutation, so that failed cleanup cannot recreate execution authority.
39. As an owner, I want each manual script invocation to have distinct immutable start and ZIP result evidence, so that an earlier partial failure survives a later success.
40. As an owner, I want incomplete, late or out-of-order invocation evidence reconciled before continuation, so that apparent success does not hide a later interrupted attempt.
41. As an owner, I want protected-only inspection to remain inside the same held barrier with its own evidence, so that a verification step does not permit unrelated Recipe execution.
42. As an owner, I want EmuChef to import verified ZIP results and check accessible postconditions separately, so that a script's success report is not mistaken for emulator usability.
43. As an owner, I want setup progress recovered after reconnect without automatic replay, so that closing the Mac app does not reauthorize a stale script.
44. As an owner, I want temporary files cleaned from the device once no longer needed, so that setup does not leave unnecessary storage behind.
45. As an owner, I want a failed cleanup after proven authority closure shown as a separate warning with retry/manual guidance, so that otherwise successful setup can finish honestly.
46. As an owner, I want root loss, transport failure and target identity change to fail closed, so that subsequent operations are not silently redirected or elevated.
47. As an owner, I want exact RetroArch configuration, ROM and BIOS effects validated in the app, so that file presence alone is not misreported as usable setup.
48. As an owner, I want the same configuration workflow to work for a second genuinely different emulator format, so that the system is not RetroArch-specific.
49. As an owner, I want saved templates and explicitly saved generated configurations treated as portable intent, so that reuse always gets a new resolved review.
50. As an owner, I want keyboard and screen-reader-accessible instructions, error recovery and progress, so that advanced setup remains usable without relying on visual cues.
51. As a qualification operator, I want exact build/device/strategy evidence distinguished from historical simulation and editor results, so that unsupported claims are not published.
52. As a release reviewer, I want outstanding evidence assigned to a downstream gate even when its inventory ticket is finished, so that missing physical qualification remains visible.
53. As a release reviewer, I want EmuChef proper's Apple Silicon checks separated from deferred Config Editor and other-platform releases, so that release readiness is scoped accurately.

## Implementation Decisions

1. **Reuse one authority chain.** App Definition → Recipe/Device Plan → fully resolved reviewed Execution Plan → trusted Rust executor/sidecar → authoritative report. Neither UI rendering nor saved intent acquires device, root, filesystem or execution authority.
2. **Preserve current and intended behavior separately.** Existing typed planner, normal executor, Tauri review boundary, operation reports and qualification observations are the baseline. Generated configuration, OEM strategy, backup/recovery and their new UI are *future work* until independently implemented.
3. **Use one guided setup with conditional reviews (UX-1 A).** Installation and app initialization precede a new config review when defaults/materialized bytes only become knowable after first run. Additional OEM stages are integrated in the same journey, never hidden.
4. **Keep Config Editor enhancements deferred.** Only non-lossy new schema round trip or refuse-unsafe-save compatibility belongs in scope; it is not a parallel consumer interface.
5. **Keep App Definition schema authority centralized.** Managed property types, validation, App Target and intrinsic permission/artifact rules are app-owned; Recipes expose selected settings in their app-specific config section and compose procedural actions.
6. **Keep device-scoped recommendations and storage bindings portable.** Precedence is explicit valid override > exact model > brand > generic; equal-specificity ambiguous device matches fail. Logical volume identity is revalidated, with no silent SD-to-internal fallback.
7. **Treat copy intent as typed semantics.** Add/Update, Replace and Mirror must have identical bounded meanings in simulated and real adapters. Mirror is not implemented merely by ADB's sync flag.
8. **Compose exact configuration artifacts before review.** Typed template substitution and managed direct overrides are safe and deterministic. Unknown untouched parsed device values are preserved when safely parseable. Multiple writers to one canonical destination/stage yield one final reviewed output, not ordered blind overwrites.
9. **Gate mutation on observed preconditions.** Review binds source/output bytes, target/device/volume identity, original hash or confirmed absence, required app quiescence and copy intent. Create-only requires actual no-clobber publication; stale or unsupported conditions require new review.
10. **Keep one frozen effective backup mode per reviewed bundle.** Default-enabled backup requires a verified Recovery Original before existing-file overwrite. A failed enabled backup blocks the write; a file/execution-specific waiver is possible only in a newly reviewed continuation. Persistent opt-out is explicit and applies only to newly reviewed bundles. Every invocation of the same bundle inherits the same bytes/mode.
11. **Retain Recovery Originals independently.** ZIPs and staging are temporary evidence/payload, not qualified original-byte recovery. Restoration is a fresh review against the current target and prepared app.
12. **Make direct ADB root and OEM runner separate strategies.** EmuChef proper planning reuses valid root qualification or performs a bounded planning-time probe. Direct ADB root is preferred when verified; an eligible Device Profile may select OEM only when direct root is confirmed unavailable. Unknown/transport errors fail closed. No automatic switch following root loss after review; no firmware-specific allowlist or per-model feature rollout gate.
13. **Compile the whole supported typed root vocabulary through the OEM runner.** Include protected file reads, conditions, file changes, ownership/mode, permission and app-op operations where the direct-root executor supports them. The chosen strategy must enforce every mandatory stale-state, quiescence, backup and no-clobber precondition; "where feasible" cannot waive one. Required unsupported actions fail planning rather than being pruned.
14. **Stage a reviewed Privileged Setup Bundle on device internal shared storage.** The Device Profile declares its safe runner menu/instructions and staging location. Stage only needed privileged inputs and bind hashes, stage/run/device, destination and exact reviewed plan. No arbitrary Recipe shell fragments.
15. **Treat an invocation as different from staging.** Awaiting manual action is not evidence of actual script start. Each user invocation has a distinct attempt ID and immutable start/ZIP report, ordered by a qualified device-side mechanism rather than ZIP arrival order or wall timestamps.
16. **Enforce OEM admission and closure on the device.** An open reviewed bundle permits safe serial state-checked reruns under a held barrier. An actual start must atomically test admission, claim exclusive mutation and register the attempt relative to a concurrent close. Closing/revocation denies future starts; a script still on-device after cleanup failure is inert. This is an **outcome contract**, not a prescribed lock implementation.
17. **Drain and reconcile under the same barrier.** T19 establishes device-side closure to new starts, reconciles all admitted/possibly started attempts and necessary separately reviewed root-only inspections, and proves none active/uncertain before allowing another Recipe step. Unproven closure is an execution-authority blocker, not a cleanup warning.
18. **Use strict evidence import and truthful results.** Each invocation writes an immutable, bounded ZIP under a separate results location. Import verifies identity, schema, digests, paths, entry types, compression and size limits. Script reports are not independent proof of protected target state or app usability. Missing or conflicting evidence remains unresolved.
19. **Keep cleanup separate from authority.** Only owned, no-longer-needed temporary artifacts become cleanup-eligible after admission closes, attempts are reconciled and Recovery Originals are safely retained. EmuChef attempts safe cleanup, retries on reconnection, and offers manual guidance if no reconnection. **Q81 C** means failed deletion after proven closure is a warning with cleanup_pending, not independent setup failure.
20. **Preserve one-active execution and fail-stop rules.** An OEM stage pauses all other Recipe operations, even independent work, until reconciled; bounded status/result collection is not a new Recipe operation. Transport/identity/authorization loss has its existing fail-closed impact.
21. **Do not infer physical qualification from design.** T06a determines early feasible access and required implementation/evidence contracts; T18 qualifies typed-operation compiler protocol; T19 qualifies stage/attempt protocol; T13b proves integration and complete unpruned operation graph; T15's first OEM physical proof is the reviewed RetroArch main configuration copy; T06b independently proves emulator-side usability.
22. **Scope the first product and final release.** Target native Apple Silicon macOS; user-supplied Android Platform-Tools; selected RetroArch AArch64 build and qualified device/workflow. T16 proves a second different format. T17Q completes obligation inventory/dispositions without claiming evidence satisfied; T17 performs exact-release-build assessment and requests separately granted authorization. Deferred Config Editor/Windows/Linux/Intel public releases are not unconditional blockers.
23. **Preserve the approved delivery tracker.** The existing [EmuChef wayfinder map](https://github.com/whatobiplays/EmuChef/issues/43) contains **23 child tickets and 33 direct body-level blocking relationships**. Reuse their IDs and scopes; no second issue hierarchy or new dependency edges are authorized by this spec. The first-open-unassigned frontier follows map order, with T05 → T06a → T18 → T19 ahead of library work.

## Testing Decisions

**Preferred highest practical seam:** Exercise the **entire selected device-plan journey through trusted product planning, review, Tauri/sidecar invocation, a controlled fake-device/OEM-runner adapter, and authoritative result/report projection**. Assert observable accepted/blocked/uncertain behavior and exact resulting artifacts—not internal implementation class names, lock layout or React timing details. Add narrowly scoped lower-seam tests only where a separate adapter, parser, script runner or safety invariant cannot be reached through that flow. **Scope rule:** Choose the highest practical test boundary *within each ticket's accepted scope and completed prerequisites*. Controlled interfaces may prove a bounded handoff when later implementation is unavailable; actual library-origin T09 composition/review and qualified deployment remain mandatory for T17/G4, and physical qualification remains with its named downstream owners. Neither is an implicit new prerequisite for independently completing T12.

1. **Schema and authoring fixture seam:** Validate App Definition, Device Profile, Device Plan and Recipe authored contracts, typed managed settings, unknown untouched configuration properties, template safety and correct app target authority. Follow existing authored-catalog/parser test patterns.
2. **Copy and storage seam:** Compare fake and real adapter semantics for Add/Update, Replace and Mirror, bounded deletion, source/destination overlap and late destination drift. Verify logical internal/SD identity and target denial/ambiguity, never rely on literal SD UUID from saved intent.
3. **Configuration composition seam:** Golden test final bytes and provenance for one and multiple Recipe writers; preserve unmanaged content/comments and fail on incompatible or unsafe source inputs.
4. **G1 create-only seam:** End-to-end setting → reviewed plan → confirmed-absent publication → exact result, including a target that appears between final check and attempted creation. If no qualified no-clobber adapter exists, report deployment unready instead of asserting success.
5. **First-run/existing-app seam:** One guided journey through install/initialization → observed defaults → new review, and through running-app preparation → post-stop new bytes → review. Assert no hidden mutation or review reuse.
6. **Backup/restore seam:** Verified original-byte backup, failed backup with accepted/declined file-and-execution waiver, global opt-out, restoration with changed target, retention and crash interruption. A ZIP report does not prove recoverable original bytes.
7. **Frozen-preference seam:** Change persistent backup preference before proven OEM start, while start is racing, during a running invocation and between serial reruns of the same bundle. Assert same-bundle frozen mode and explicit newly reviewed replacement.
8. **OEM capability/strategy seam (T18):** Confirm root available, unavailable, denied and unknown; distinguish direct-root flags from OEM availability. Test supported Device Profile/menu/staging, mandatory operation parity and fail-closed unsupported predicates on representative manufacturer fixture classes; device-specific physical tests remain separately authorized.
9. **Admission/closure seam (T18):** Simulate actual invocation racing closure. It must be admitted and fully tracked before closure or rejected before any mutation. Verify safe serial reruns while open, no concurrent mutation, and retained closed script refusing mutation even when bundle cleanup fails.
10. **Reconciliation/barrier seam (T19):** Reverse ZIP arrival order; complete first attempt then interrupt the second; begin another invocation while one is active; start but never complete; require a reviewed root-only inspection within the held barrier. Never release while a start, result or admission closure is uncertain.
11. **Invalidation/recovery seam:** Preference invalidation races start; cancellation, replanning, replacement bundle and host restart cannot revive an old bundle's privilege or silently select a different strategy. Preserve all immutable per-invocation results.
12. **ZIP validation seam:** Reject missing, malformed, duplicate, unsafe-entry, oversized, stage/device mismatch or wrong-hash ZIPs; report uncertain rather than fabricate success. Independently verify target postconditions where qualified.
13. **Cleanup-versus-authority seam:** After proven closure and successful result reconciliation, fail on-device cleanup, show cleanup_pending warning and allow valid later Recipe work. Retained script must refuse launch. Verify reconnect retry, ownership guards and safe abandoned-run manual guidance. Separately assert unproven admission closure **blocks** continuation.
14. **Recipe dependency seam:** Alternate OEM stage → reconciliation → ordinary ADB work → OEM stage, preserving dependencies with **no concurrent Recipe execution during either OEM barrier**.
15. **Physical qualification seam:** Under a separately authorized target/build/operator run, prove actual RetroArch main config copy into protected app storage, correct required ROM/BIOS/resources and emulator-side use, then a second real emulator/format. T06a's early feasibility does not discharge T13b/T15 evidence.
16. **Release and evidence seam:** Retain exact source/validator status for the eight known missing mandatory Phase 6D.6 repetitions, inherited manual and OEM obligations; verify T17Q inventory/ownership gates independent from actual physical passes. Evaluate current EmuChef proper Apple Silicon signing/entitlements/notarization/Gatekeeper/clean-Mac/update trust separately from historic Config Editor artifacts and unsupported host platforms.
17. **Accessibility and privacy seam:** Test one-journey keyboard and screen-reader interactions, bounded progress/recovery, no leaked raw device identities/paths, and clear distinction among complete, partial, unverified and cleanup warning results.

**Test evidence must name the exact selected target/build, adapter, authority, reviewed plan, operation graph and relevant artifact digests.** Simulation and previous phase evidence are useful prior art, never automatic physical proof for new features. A passing unit test alone does not authorize a physical device run or release.

## Out of Scope

- Any runtime implementation, staging, commit/push, physical device operation, production real-execution enablement or release under this specification-publication task.
- A parallel or separately authored OEM-specific Recipe system; arbitrary Recipe shell scripts; automatic OEM menu invocation; silently treating OEM runner availability as rooted ADB.
- Firmware-specific OEM feature allowlists, per-model feature rollout gates or blanket claims that all AYN, Retroid and AYANEO configurations are physically qualified.
- Automatic adapter fallback between rooted ADB and OEM after review or on lost root authority.
- Whole-device or whole-setup transactional rollback, unreviewed existing-file overwrite, assumed remote atomicity, unsafe create-only overwrite or unsupported change to config.
- New Config Editor UX/feature roadmap, Config Editor public release, old pre-v1 debug saved-setup migration or changed public package identity.
- Cross-platform public release for Intel/universal macOS, Windows or Linux, and broader apps/devices beyond the accepted first RetroArch and selected second-emulator path.
- A new ticket hierarchy, new graph dependencies or premature T17Q evidence waivers; closed T01/T02 history remains intact.

## Further Notes

### Delivery and evidence gates

- **G0** — Canonical requirements, documented architectural authority, ticket graph and inherited evidence inventory (local T01/T02 work already documented; not source-code implementation).
- **G1** — One-setting/one-writer/confirmed-absent configuration creates a reviewed artifact through the trusted end-to-end path and verified no-clobber publication.
- **G2** — New/existing/running RetroArch setup with internal/SD content, guarded backups, optional OEM strategy when needed, full required unpruned operation graph and actual app usability.
- **G3** — Repeat the same model for a second emulator with a genuinely different config format.
- **G4** — Library-origin configuration through the same reviewed composition/deployment path, exact current-build release evidence and **separate owner release authorization**.

### Existing 23-ticket delivery crosswalk (not a second backlog)

| Area | Tickets and verified outcomes |
| --- | --- |
| Authority/tracker | [T01](https://github.com/whatobiplays/EmuChef/issues/44) requirements/evidence and [T02](https://github.com/whatobiplays/EmuChef/issues/45) GitHub cutover — completed as authorized local documentation/tracking work; original 34-edge T02 record is historical |
| Copy/storage | [T03](https://github.com/whatobiplays/EmuChef/issues/46) copy semantics, [T04](https://github.com/whatobiplays/EmuChef/issues/47) volumes, [T05](https://github.com/whatobiplays/EmuChef/issues/50) Setup Locations, [T05b](https://github.com/whatobiplays/EmuChef/issues/55) portable saved intent |
| Early feasibility | [T06a](https://github.com/whatobiplays/EmuChef/issues/53) early mechanism/evidence contract after T05; [T06b](https://github.com/whatobiplays/EmuChef/issues/63) later emulator usability, blocked through T13b |
| Configuration | [T07](https://github.com/whatobiplays/EmuChef/issues/48) schema, [T08](https://github.com/whatobiplays/EmuChef/issues/51) rendering, [T09](https://github.com/whatobiplays/EmuChef/issues/54) safe merge, [T10](https://github.com/whatobiplays/EmuChef/issues/58) backup/recovery, [T11](https://github.com/whatobiplays/EmuChef/issues/52) recommendations |
| Consumer & library | [T13a](https://github.com/whatobiplays/EmuChef/issues/56) create-only path, [T13b](https://github.com/whatobiplays/EmuChef/issues/61) complete lifecycle, [T14](https://github.com/whatobiplays/EmuChef/issues/62) one guided UX, [T12](https://github.com/whatobiplays/EmuChef/issues/59) reusable library |
| OEM strategy | [T18](https://github.com/whatobiplays/EmuChef/issues/57) device Profile and typed compiler including admission/revocation; [T19](https://github.com/whatobiplays/EmuChef/issues/60) stage orchestration, immutable attempts, drain, results and cleanup |
| Target acceptance | [T15](https://github.com/whatobiplays/EmuChef/issues/64) full RetroArch, [T16](https://github.com/whatobiplays/EmuChef/issues/65) second emulator, [T17Q](https://github.com/whatobiplays/EmuChef/issues/49) evidence inventory/disposition, [T17](https://github.com/whatobiplays/EmuChef/issues/66) exact-build release review |

The corrected direct blockers remain **T06a ← T05**, **T18 ← T03/T05**, **T19 ← T18/T13a**, **T13b ← T10/T19/T06a**, **T14 ← T13b/T11**, and **T06b ← T13b**. There are **23 child tickets / 33 direct blockers**, with task-list order [T05 → T06a → T18 → T19](https://github.com/whatobiplays/EmuChef/issues/43) for sequential priority and deliberate parallel work allowed. The original T02 publication had 34 edges and remains an unchanged historical record.

### Publication and authority limits

This file is authored locally, not yet committed or pushed. Its GitHub issue publication is a separate reference artifact, **not a new child work ticket, blocker, release approval or replacement for the map**. The original requirements, ADRs, current implementation account and source-specific evidence retain their distinct authorities. GitHub blocker links are the repository-approved body-level fallback rather than native GitHub issue-dependency relations.
