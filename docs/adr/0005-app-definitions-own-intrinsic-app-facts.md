# ADR 0005: App definitions own intrinsic app facts

**Status:** Accepted

EmuChef separates authored authority into four layers: App Definitions own intrinsic application facts and policy, Recipes own executable procedure, Device Plans own recipe composition for supported setups, and Execution Plans own the fully resolved immutable intent consumed by review and execution. Recipes may reference App Definitions at app-aware steps, but must not duplicate package identity, app-owned artifact source policy, permission definitions, launcher identity, or semantic app paths; the planner resolves those authored references into a self-contained Execution Plan.

This deliberately rejects the previous model where App Definitions were catalog/tracking metadata and Recipes remained the complete execution authority. It also rejects recipe-wide app ownership and automatic planner selection among app build variants. Independently selectable builds are named App Artifacts selected explicitly by Recipes, while Device Plans choose the appropriate Recipes for a device setup.

Because App Definition IDs, App Artifact IDs, and App Target IDs may be referenced across authored documents, renaming them is an explicit atomic catalog refactor rather than a local field edit. The v1 authored contract is changed directly with no legacy compatibility or fallback semantics because EmuChef is still pre-1.0.

These decisions describe the accepted final v1 authority model. Implementation is proceeding incrementally, so transitional authority forms may remain until the final authored-data cutover; the completed cutover leaves no legacy authority forms or fallback semantics.
