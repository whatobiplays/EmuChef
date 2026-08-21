# Conditional Device-Offline Evidence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `device_offline` conditional diagnostic evidence while retaining runtime and schema support, and reduce mandatory Phase 6D.6 physical closure from 26 to 24 repetitions.

**Architecture:** The scenario manifest remains the single source of truth but gains explicit mandatory and conditional scenario lists. The Node validator continues accepting every supported scenario while computing completeness from only the mandatory list and restricting mandatory transport UI-smoke binding to USB disconnect evidence.

**Tech Stack:** JSON manifest, dependency-free Node.js validator and `node:test`, Markdown current-state documentation.

## Global Constraints

- Do not change executor, ADB, Tauri, frontend, schema-enum, or physical harness behavior.
- Keep `device_offline` selectable and evidence-valid.
- Do not delete or rewrite existing physical evidence.
- Do not stage, commit, or push.

---

### Task 1: Contract regressions

**Files:**
- Modify: `tools/phase-6d6-evidence.test.mjs`
- Modify: `tools/phase-6d6-evidence-regression.test.mjs`

- [ ] Add assertions for 13 supported, 12 mandatory, and one conditional scenario.
- [ ] Add a completeness regression proving two passing offline records are optional.
- [ ] Add a UI-binding regression rejecting offline evidence and accepting both USB-disconnect scenarios.
- [ ] Run the two Node test files and confirm the new assertions fail against the old contract.

### Task 2: Manifest and validator

**Files:**
- Modify: `docs/testing/phase-6d6/scenario-manifest.json`
- Modify: `tools/phase-6d6-evidence.mjs`

- [ ] Add `mandatoryScenarios` and `conditionalScenarios` to the manifest.
- [ ] Export both lists from the validator while retaining `SCENARIOS` as all supported scenarios.
- [ ] Validate list uniqueness, partitioning, and exact manifest parity.
- [ ] Compute missing repetitions from `MANDATORY_SCENARIOS` only.
- [ ] Restrict transport UI binding to `usb_disconnect_active` and `usb_disconnect_boundary`.
- [ ] Update validation wording from mandatory matrix to supported scenario set where applicable.
- [ ] Run the Node tests and confirm they pass.

### Task 3: Closure documentation

**Files:**
- Modify: `docs/manual/phase-6d6-physical-interruption-qualification.md`
- Modify: `CONTEXT.md`
- Modify: `docs/product/phase-6d6-physical-interruption-qualification.md`
- Modify: `docs/product/phase-6d1-execution-safety-audit.md`
- Modify: `docs/product/product-roadmap.md`

- [ ] Mark `device_offline` as conditional diagnostic evidence.
- [ ] State the mandatory matrix is 12 scenarios × 2 repetitions.
- [ ] State USB disconnect is the mandatory physical transport proof.
- [ ] Preserve runtime support and deterministic automated coverage language.
- [ ] Run the repository validator and all Phase 6D.6 Node tests.

### Task 4: Final review

- [ ] Review the full unstaged diff for accidental runtime or evidence changes.
- [ ] Confirm existing evidence files are untouched.
- [ ] Confirm the validator reports 12 mandatory physical repetitions missing from the current accepted evidence set, plus 2 UI-smoke repetitions.
