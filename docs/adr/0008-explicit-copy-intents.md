# ADR 0008: Explicit copy intents, previewed deletion, and execution parity

**Status:** Accepted product direction; interrupted-operation mechanics open.

EmuChef distinguishes non-destructive Add or Update Copy, intentional Replace Destination Copy, and Mirror Copy, which removes destination-only files. Non-destructive transfer is the default; replacement or deletion is deliberate. For Mirror Copy, EmuChef prepares a concrete deletion manifest before review, presents a bounded summary with expandable details, and revalidates deletion scope immediately before execution. If scope cannot be safely determined or revalidated, the operation is blocked rather than approximated by a generic warning.

Real-device and simulated execution must honor identical semantics. The current fake directory `sync` deletes destination-only files while real ADB `push --sync` does not, so the mismatch and its dependent qualifications require correction before relying on that behavior. The pre-v1.0 authored and saved-setup schemas may change incompatibly. No old `sync` policy migration or legacy parser is required: obsolete test/qualification saved setups may be rejected and recreated, but user files should not be silently deleted.

A failed step blocks its dependents; independent operations may continue only when safe. Results must distinguish completed, failed, skipped, and partial actions without implying whole-setup rollback. Exact replacement boundaries and interrupted-operation recovery mechanisms remain implementation design questions.

## Open engineering contract (not implemented)

[EmuChef product realignment proposal](../product/emuchef-product-realignment-proposal.md) assigns T03 explicit source/destination Replace and Mirror boundaries, actual deletion manifests, staging and transfer/deletion order, plan-wide overlap detection, interrupted-operation reporting, and fake/real behavior parity. T13a uses only the qualified copy contract; T15 expands physical workflow evidence. Existing `sync` tests and evidence are not proof of the proposed real Mirror implementation.
