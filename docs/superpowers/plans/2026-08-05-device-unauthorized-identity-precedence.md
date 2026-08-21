# Device-Unauthorized Identity-Precedence Qualification Implementation Plan

**Goal:** Accept the production identity-precedence branch for the safe-boundary authorization qualification without changing runtime behavior.

**Architecture:** Expand only the Phase 6D.6 evidence contract and serializer. The production executor continues to perform pre-operation identity verification before the intended mutation.

## Constraints

- Preserve the two untracked blocked evidence records and traces unchanged.
- Do not edit production identity, ADB classification, issue precedence, retry/resume, or public interfaces.
- Keep `device_unauthorized` mandatory with two passing repetitions.
- Do not stage, commit, or push.

## Tasks

1. Add failing Rust and Node regressions for both accepted terminal branches and all required negative cases.
2. Add the pre-change safe-boundary contract to `legacyAuditContracts.device_unauthorized` for non-passing records only.
3. Expand the current manifest contract and JSON schema to accept `device_identity_unverified`.
4. Serialize the actual accepted terminal issue from `authorization_transition_evidence`.
5. Make Rust and Node validation require transition issue equality with the terminal issue and membership in the contract allowlist.
6. Update the runbook and current-state documentation to explain identity-first precedence.
7. Run formatting, focused Rust tests, Node evidence tests, the validator, and `git diff --check`.

## Verification

```sh
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all -- --check
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution --lib executor_real_adb_tests::physical_interruption_qualification::tests -- --nocapture
node --test tools/phase-6d6-evidence.test.mjs
node --test tools/phase-6d6-evidence-regression.test.mjs
node tools/phase-6d6-evidence.mjs
git diff --check
```
