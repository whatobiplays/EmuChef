# ROM/Content Recipe Qualification Design

## Scope

Qualify the next remaining Phase 6E authored workflow: copying ROM/content files. This is an automated, source-bound qualification task for EmuChef proper and the shared runtime. It does not resume owner-deferred manual or physical qualification and does not change Phase 6D closure requirements.

## Authority and provenance

The canonical product roadmap determines feature priority. Device-plan membership and current authored artifacts are evidence to inspect, not authority for deciding whether a workflow is a product feature.

Before implementation, identify the production-intended authored recipe(s) that implement ROM/content copying and bind qualification to their actual source bytes. If no production-intended authored ROM/content workflow exists, stop rather than inventing one or deriving provenance from a device plan.

## Qualification boundary

Follow the established Phase 6E qualification pattern where it remains semantically correct:

1. Load the real authored catalog through production catalog machinery.
2. Bind a strict qualification contract to the relevant authored recipe source using SHA-256.
3. Plan through `runtime_configuration::plan_configuration`, using a real device/profile context only for capability/planning context.
4. Select the target recipe explicitly unless the product roadmap and authored defaults independently establish that default composition itself is the behavior under qualification.
5. Assert exact recipe expansion, required input contracts, operation families, production review projection, execution ordering, and blocker state.
6. Execute the unchanged generated plan through deterministic sandbox executor adapters using controlled local fixtures; do not use ADB or live network access.
7. Exercise success and meaningful repeat/skip or failure behavior supported by the production semantics, while preserving truthful execution/reporting state.

## Inputs and filesystem semantics

Qualification must prove the actual authored ROM/content input multiplicity and destination semantics rather than assuming they match BIOS copying. Controlled fixtures must cover the structure required by the authored workflow, including multiple files or directory structure when the production recipe supports them. Destination assertions must preserve Android/device path semantics and executor sandbox boundaries.

Do not add new executor capabilities, planner behavior, authored schema, or product behavior solely to make the qualification pass. A discovered production defect may justify a separately bounded correction, but qualification itself should characterize the existing intended workflow.

## Documentation and status

Add a dedicated ROM/content qualification record and update the canonical product roadmap only with evidence actually established by the automated qualification. Keep Phase 6E In progress. State explicitly that automated qualification does not prove physical Android storage behavior, hardware compatibility, packaged-GUI behavior, or full end-to-end success.

Do not weaken or rewrite existing RetroArch, BIOS, combined RetroArch + BIOS, Obtainium, Phase 6D, or physical qualification claims.

## Testing

Use focused backend qualification tests plus the repository-supported validation route applicable to the changed backend/docs scope. Tests should fail closed on authored-source drift, planning/review drift, input/destination drift, or executor outcome drift.

## Success criteria

- A production-intended ROM/content authored workflow is identified from repository evidence without treating device-plan membership as product provenance.
- A strict source-bound contract detects recipe drift.
- Production planning and review prove the exact ROM/content inputs, expansion, ordering, capabilities, and blocker-free intended path.
- Deterministic executor qualification proves controlled ROM/content copying through the unchanged generated plan.
- At least one meaningful negative or repeat-state case proves truthful failure/skip behavior appropriate to the recipe.
- No live network, ADB, manual, physical, packaged-GUI, release, or cleanup qualification is claimed.
- Phase 6D remains In progress with all deferred evidence requirements unchanged; Phase 6E remains In progress.
