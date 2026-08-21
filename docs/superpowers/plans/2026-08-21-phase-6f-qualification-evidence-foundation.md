# Phase 6F Qualification Evidence Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the repository-owned Phase 6F physical-device qualification evidence foundation without performing or claiming any real physical qualification.

**Architecture:** Follow the existing Phase 6D.6 evidence-tooling precedent: keep production EmuChef as the system under test, and implement qualification definitions, strict evidence validation, compatibility projection, support-tier derivation, and deterministic matrix generation in dependency-free Node 22 tooling outside the product runtime. Store versioned definitions under `docs/testing/phase-6f/`, synthetic-only fixtures under `tests/fixtures/phase-6f/`, real evidence under a separate empty production evidence directory, and generate the human-readable matrix from canonical inputs. Do not add an alternate planner, executor, device-probe, or support-authority path.

**Tech Stack:** Node.js 22 built-ins (`node:fs`, `node:path`, `node:crypto`, `node:test`), strict JSON contracts, existing authored YAML/profile file identities, Markdown documentation, GitHub Actions, Makefile.

**Spec:** `docs/superpowers/specs/2026-08-21-phase-6f-qualification-evidence-foundation-design.md`

## Global Constraints

- The primary qualification unit is `device target x workflow`; recipe identity alone is not the qualification key.
- A matching authored device profile does not itself imply physical support.
- Qualification definitions, immutable physical evidence, and derived current support state remain separate layers.
- Compatibility fingerprints are structured and inspectable; any digest is derived, not authoritative by itself.
- Compatibility changes are deterministically classified as `compatible`, `invalidating`, or `not_applicable`; do not use a blanket repository-change invalidation rule.
- Workflow applicability is deterministically `required`, `not_applicable`, or `deferred`; `deferred` never counts as passing evidence.
- Phase 6E automated recipe qualification may be referenced as supporting context but cannot satisfy Phase 6F physical evidence.
- Human checkpoints are typed evidence with stable IDs and bounded outcomes; missing or `unable_to_verify` required checkpoints cannot produce `qualified`.
- Invalid harness/infrastructure runs are distinct from valid product qualification failures.
- Workflow state is derived as `qualified`, `failed`, `stale`, `deferred`, `missing`, or `not_applicable`.
- Device support tier is derived as `qualified`, `limited`, or `unqualified`; only explicitly modeled target-wide prerequisite/safety failures may invalidate a whole target.
- Historical physical evidence is immutable. Newer evidence may supersede older evidence for current projection but never rewrites history.
- Synthetic fixtures must be structurally and path-wise isolated from real physical evidence and must never appear in the generated production matrix.
- Rust/Tauri remain authoritative for existing planning, execution, device facts, filesystem behavior, and validation responsibilities. Phase 6F tooling must observe those systems rather than replace them.
- This task must not perform a real physical-device run, add a real evidence record, enable production real execution, resume deferred Phase 6D physical/manual evidence collection, or begin Phase 6G release promotion.
- Use TDD for each behavior slice and keep the production qualification evidence directory empty in this task.

---

## File Structure

Create and modify the following focused units:

- `docs/testing/phase-6f/workflow-catalog.json` — canonical workflow definitions, compatibility dimensions, capability/prerequisite requirements, automated observation contracts, and typed human checkpoints.
- `docs/testing/phase-6f/device-targets.json` — versioned production device-target registry. It starts with an empty `targets` array in this foundation task so no support claim is implied.
- `docs/testing/phase-6f/evidence-schema.json` — human-readable machine contract describing immutable run-record fields and enum values. Runtime validation remains in the dependency-free Node validator.
- `docs/testing/phase-6f/evidence/README.md` — production-evidence boundary and immutability rules; no JSON physical evidence records are added in this task.
- `tests/fixtures/phase-6f/` — synthetic catalogs, device targets, evidence histories, invalid records, and expected matrix fixtures used only by tests.
- `tools/phase-6f-qualification.mjs` — pure loading, validation, canonical-digest, compatibility, applicability, current-evidence selection, workflow-state, support-tier, and matrix-rendering functions plus a small CLI.
- `tools/phase-6f-qualification.test.mjs` — Node test suite proving the full synthetic contract and failure cases.
- `docs/qualification/phase-6f-device-matrix.md` — deterministic generated projection. With no production device targets/evidence in this foundation task, it truthfully states that no physical targets are yet qualified.
- `docs/manual/phase-6f-qualification-operator.md` — exact later operator workflow, including evidence capture boundary and checkpoint semantics; no actual device run is performed.
- `.github/workflows/emuchef-execution-feature-matrix.yml` — run Phase 6F validator/tests and drift check when Phase 6F files change.
- `Makefile` — expose a local `phase-6f-qualification-check` target and include it in `test`.
- `docs/product/product-roadmap.md` — change Phase 6F from `Planned` to `In progress` only after the foundation is implemented, explicitly recording that no real physical evidence exists yet.

---

### Task 1: Canonical Phase 6F Definitions and Strict Contract Loader

**Files:**
- Create: `docs/testing/phase-6f/workflow-catalog.json`
- Create: `docs/testing/phase-6f/device-targets.json`
- Create: `docs/testing/phase-6f/evidence-schema.json`
- Create: `docs/testing/phase-6f/evidence/README.md`
- Create: `tests/fixtures/phase-6f/definitions-valid/workflow-catalog.json`
- Create: `tests/fixtures/phase-6f/definitions-valid/device-targets.json`
- Create: `tests/fixtures/phase-6f/definitions-invalid/duplicate-workflow-id.json`
- Create: `tests/fixtures/phase-6f/definitions-invalid/unknown-profile.json`
- Create: `tools/phase-6f-qualification.mjs`
- Create: `tools/phase-6f-qualification.test.mjs`

**Interfaces:**
- Produces: `loadWorkflowCatalog(path)`, `loadDeviceTargets(path, { authoredProfilesDir })`, `validateWorkflowCatalog(value)`, `validateDeviceTargets(value, { authoredProfilesDir })`, `canonicalize(value)`, `canonicalDigest(value)`.
- Consumes later: Tasks 2–6 import these functions from `tools/phase-6f-qualification.mjs`.

- [ ] **Step 1: Write the failing definition-loader tests**

Add tests that prove strict top-level keys, strict nested keys, schema version `1`, stable ID uniqueness, allowed enums, non-empty user-visible purpose, recipe/composition references, compatibility dimensions, checkpoint definitions, target capability lists, and authored device-profile file existence.

```js
import test from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import {
  loadWorkflowCatalog,
  loadDeviceTargets,
} from "./phase-6f-qualification.mjs";

const repoRoot = path.resolve(import.meta.dirname, "..");
const fixtures = path.join(repoRoot, "tests/fixtures/phase-6f");

test("loads a strict version-1 workflow catalog", () => {
  const catalog = loadWorkflowCatalog(
    path.join(fixtures, "definitions-valid/workflow-catalog.json"),
  );
  assert.equal(catalog.schemaVersion, 1);
  assert.equal(catalog.workflows[0].id, "retroarch-plus-bios");
});

test("rejects duplicate workflow ids", () => {
  assert.throws(
    () => loadWorkflowCatalog(path.join(fixtures, "definitions-invalid/duplicate-workflow-id.json")),
    /duplicate workflow id/,
  );
});

test("rejects device targets whose authored profile does not exist", () => {
  assert.throws(
    () => loadDeviceTargets(
      path.join(fixtures, "definitions-invalid/unknown-profile.json"),
      { authoredProfilesDir: path.join(repoRoot, "authored/device_profiles") },
    ),
    /unknown authored device profile/,
  );
});
```

- [ ] **Step 2: Run the new test file and verify RED**

Run:

```bash
node --test tools/phase-6f-qualification.test.mjs
```

Expected: FAIL because `tools/phase-6f-qualification.mjs` and loader exports do not exist.

- [ ] **Step 3: Implement strict JSON helpers and definition validation**

Use only Node built-ins. Reject unknown fields rather than silently ignoring them. Canonical JSON must sort object keys recursively while preserving array order.

```js
export function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value).sort().map((key) => [key, canonicalize(value[key])]),
    );
  }
  return value;
}

export function canonicalDigest(value) {
  return `sha256:${createHash("sha256")
    .update(JSON.stringify(canonicalize(value)))
    .digest("hex")}`;
}

function assertExactKeys(value, expected, label) {
  assertObject(value, label);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    fail(`${label} fields must be exactly ${wanted.join(", ")}`);
  }
}
```

The canonical workflow shape must be explicit and versioned. Use this production catalog shape:

```json
{
  "schemaVersion": 1,
  "workflows": [
    {
      "id": "retroarch-plus-bios",
      "version": 1,
      "purpose": "Provision RetroArch and copy the required BIOS set through the production EmuChef workflow.",
      "productionRecipes": ["app.retroarch.provision", "feature.copy_bios"],
      "requiredCapabilities": [],
      "prerequisites": ["clean_or_deliberately_reset_device"],
      "compatibilityDimensions": [
        "emuchef_build",
        "workflow_version",
        "authored_content",
        "runtime_contract",
        "device_profile",
        "android_api",
        "firmware_build",
        "abi_soc_class",
        "root_state"
      ],
      "automatedObservations": [
        {"id": "execution-report", "required": true}
      ],
      "humanCheckpoints": []
    }
  ]
}
```

Populate the production catalog only with workflows already production-intended in the current roadmap/authored catalog: `retroarch-plus-bios`, `obtainium-install`, `xaniteog-install`, and `rom-library-sync`. Do not invent Daijisho or ES-DE workflows because the roadmap explicitly conditions them on future authored recipes/assets.

`docs/testing/phase-6f/device-targets.json` must begin as:

```json
{
  "schemaVersion": 1,
  "targets": []
}
```

This empty production registry is intentional: the foundation must not manufacture a device support claim.

- [ ] **Step 4: Add the descriptive evidence schema and production boundary README**

The schema document must enumerate the exact run-record fields that Task 2 will enforce, including `schemaVersion`, `runId`, `capturedAt`, `workflowId`, `workflowVersion`, `deviceTarget`, `fingerprint`, `fingerprintDigest`, `runValidity`, `qualificationOutcome`, `automatedObservations`, `humanCheckpoints`, `targetWideFailure`, and `limitations`.

`docs/testing/phase-6f/evidence/README.md` must state:

```markdown
# Phase 6F Physical Evidence

This directory is reserved for immutable, validated physical-device qualification records.
Synthetic fixtures belong only under `tests/fixtures/phase-6f/` and must never be copied here.
The Phase 6F foundation intentionally contains no physical evidence JSON records.
```

- [ ] **Step 5: Run the definition tests and verify GREEN**

Run:

```bash
node --test tools/phase-6f-qualification.test.mjs
```

Expected: PASS for all Task 1 definition-loader tests.

- [ ] **Step 6: Commit the definitions slice**

```bash
git add docs/testing/phase-6f tests/fixtures/phase-6f tools/phase-6f-qualification.mjs tools/phase-6f-qualification.test.mjs
git commit -m "feat: add phase 6f qualification definitions"
```

---

### Task 2: Immutable Evidence Validation and Compatibility Fingerprints

**Files:**
- Modify: `tools/phase-6f-qualification.mjs`
- Modify: `tools/phase-6f-qualification.test.mjs`
- Create: `tests/fixtures/phase-6f/evidence-valid/passing-retroarch-bios.json`
- Create: `tests/fixtures/phase-6f/evidence-valid/failed-retroarch-bios.json`
- Create: `tests/fixtures/phase-6f/evidence-invalid/bad-digest.json`
- Create: `tests/fixtures/phase-6f/evidence-invalid/missing-required-checkpoint.json`
- Create: `tests/fixtures/phase-6f/evidence-invalid/impossible-run-result.json`

**Interfaces:**
- Consumes: Task 1 workflow/device definitions and canonical digest helpers.
- Produces: `validateEvidenceRecord(record, context)`, `evidenceFingerprintDigest(fingerprint)`, `classifyCompatibility({ workflow, currentFingerprint, evidenceFingerprint })`.

- [ ] **Step 1: Add failing evidence-validation tests**

Test strict immutable record structure, unique run identity, workflow/version binding, current target identity shape, checkpoint IDs/outcomes, run-validity versus qualification-outcome rules, target-wide failure allowlist, and recomputed fingerprint digest equality.

```js
test("rejects a fingerprint digest that does not match structured inputs", () => {
  const record = readJson(path.join(fixtures, "evidence-invalid/bad-digest.json"));
  assert.throws(
    () => validateEvidenceRecord(record, syntheticContext()),
    /fingerprintDigest does not match canonical fingerprint/,
  );
});

test("invalid infrastructure runs cannot claim a product qualification failure", () => {
  const record = readJson(path.join(fixtures, "evidence-invalid/impossible-run-result.json"));
  assert.throws(
    () => validateEvidenceRecord(record, syntheticContext()),
    /invalid run must use qualificationOutcome "not_observed"/,
  );
});
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
node --test --test-name-pattern="fingerprint|invalid infrastructure|required checkpoint" tools/phase-6f-qualification.test.mjs
```

Expected: FAIL because Task 2 exports/validation do not exist yet.

- [ ] **Step 3: Implement structured fingerprint validation**

Use an explicit version-1 fingerprint shape rather than arbitrary maps:

```js
const FINGERPRINT_FIELDS = [
  "schemaVersion",
  "emuchefBuild",
  "workflowVersion",
  "authoredContent",
  "runtimeContract",
  "deviceProfile",
  "androidApi",
  "firmwareBuild",
  "abiSocClass",
  "rootState",
  "connectionType",
];

export function evidenceFingerprintDigest(fingerprint) {
  validateFingerprint(fingerprint);
  return canonicalDigest(fingerprint);
}
```

`authoredContent` is an ordered array of `{ id, sha256 }` entries for the production recipe/composition inputs relevant to the selected workflow. Do not fingerprint unrelated repository files.

- [ ] **Step 4: Implement deterministic compatibility classification**

Compatibility must be workflow-dimension-driven. Compare only dimensions declared in `workflow.compatibilityDimensions`.

```js
export function classifyCompatibility({ workflow, currentFingerprint, evidenceFingerprint }) {
  for (const dimension of workflow.compatibilityDimensions) {
    const result = compareDimension(dimension, currentFingerprint, evidenceFingerprint);
    if (result === "invalidating") return "invalidating";
  }
  return "compatible";
}
```

`compareDimension` must return `not_applicable` for dimensions not declared by the workflow and must not invalidate evidence because an unrelated field changed.

- [ ] **Step 5: Implement strict run-validity and checkpoint semantics**

Use these enums:

```js
const RUN_VALIDITY = new Set(["valid", "invalid"]);
const QUALIFICATION_OUTCOME = new Set(["passed", "failed", "not_observed"]);
const CHECKPOINT_OUTCOMES = new Set(["pass", "fail", "unable_to_verify"]);
const TARGET_WIDE_FAILURES = new Set([
  null,
  "device_identity_unverified",
  "device_identity_changed",
  "required_device_prerequisite_unavailable",
  "safety_invariant_failed",
]);
```

Rules:

- `runValidity === "invalid"` requires `qualificationOutcome === "not_observed"`.
- `runValidity === "valid"` forbids `not_observed`.
- A passed outcome requires every required automated observation and human checkpoint to pass.
- Missing or `unable_to_verify` required checkpoints cannot pass.
- Checkpoint IDs must be declared by the selected workflow.
- `targetWideFailure` is nullable and restricted to the narrow allowlist above.

- [ ] **Step 6: Run the full Phase 6F test file and verify GREEN**

Run:

```bash
node --test tools/phase-6f-qualification.test.mjs
```

Expected: PASS through Tasks 1–2.

- [ ] **Step 7: Commit the evidence contract slice**

```bash
git add tools/phase-6f-qualification.mjs tools/phase-6f-qualification.test.mjs tests/fixtures/phase-6f docs/testing/phase-6f/evidence-schema.json
git commit -m "feat: validate phase 6f physical evidence"
```

---

### Task 3: Applicability, Historical Evidence Selection, and Derived Support State

**Files:**
- Modify: `tools/phase-6f-qualification.mjs`
- Modify: `tools/phase-6f-qualification.test.mjs`
- Create: `tests/fixtures/phase-6f/projection/device-targets.json`
- Create: `tests/fixtures/phase-6f/projection/evidence/qualified.json`
- Create: `tests/fixtures/phase-6f/projection/evidence/failed-newer.json`
- Create: `tests/fixtures/phase-6f/projection/evidence/stale.json`
- Create: `tests/fixtures/phase-6f/projection/evidence/invalid-newer.json`

**Interfaces:**
- Consumes: Tasks 1–2 definitions, validated evidence, and compatibility classifier.
- Produces: `deriveApplicability(workflow, target)`, `selectCurrentEvidence({ workflow, target, currentFingerprint, records })`, `deriveWorkflowState(args)`, `deriveDeviceSupportTier(workflowStates)`.

- [ ] **Step 1: Write failing applicability and projection tests**

Cover all six workflow states and all three support tiers. Include the critical regressions: newer invalid evidence must not replace older valid evidence; stale evidence stays historical; one failed required workflow leaves unrelated passing workflows qualified and produces device tier `limited`; target-wide safety failure produces `unqualified`.

```js
test("a failed required workflow does not erase unrelated qualified evidence", () => {
  const tier = deriveDeviceSupportTier([
    { workflowId: "retroarch-plus-bios", applicability: "required", state: "qualified", targetWideFailure: null },
    { workflowId: "rom-library-sync", applicability: "required", state: "failed", targetWideFailure: null },
  ]);
  assert.equal(tier, "limited");
});

test("an explicit target-wide safety failure makes the target unqualified", () => {
  const tier = deriveDeviceSupportTier([
    { workflowId: "retroarch-plus-bios", applicability: "required", state: "qualified", targetWideFailure: "safety_invariant_failed" },
  ]);
  assert.equal(tier, "unqualified");
});
```

- [ ] **Step 2: Run projection tests and verify RED**

Run:

```bash
node --test --test-name-pattern="applicability|workflow state|support tier|newer invalid" tools/phase-6f-qualification.test.mjs
```

Expected: FAIL because projection exports do not exist.

- [ ] **Step 3: Implement applicability from production intent plus target capabilities**

Target entries must contain explicit modeled facts, not a manual support tier:

```json
{
  "id": "synthetic-pocket-s2",
  "profileId": "ayaneo.pocket_s2",
  "manufacturer": "AYANEO",
  "model": "Synthetic Pocket S2",
  "androidVersion": "15",
  "androidApi": 35,
  "abiSocClass": "arm64-snapdragon",
  "rootState": "non_root",
  "connectionType": "usb3",
  "firmwareBuild": "synthetic/build",
  "capabilities": ["apk_install", "shared_storage_write"],
  "deferredWorkflows": []
}
```

Derivation rules:

```js
export function deriveApplicability(workflow, target) {
  if (target.deferredWorkflows.includes(workflow.id)) {
    return { state: "deferred", reason: "explicitly_deferred" };
  }
  const missing = workflow.requiredCapabilities.filter(
    (capability) => !target.capabilities.includes(capability),
  );
  if (missing.length > 0) {
    return { state: "not_applicable", reason: `missing_capabilities:${missing.join(",")}` };
  }
  return { state: "required", reason: "production_intent_and_capabilities" };
}
```

Do not derive applicability from profile matching alone.

- [ ] **Step 4: Implement deterministic current-evidence selection**

Filter records by target ID, workflow ID/version, `runValidity === "valid"`, and compatibility. Order eligible records by parsed RFC3339 `capturedAt`, then `runId` as deterministic tie-breaker. Invalid runs remain visible in history but are never current qualification evidence.

- [ ] **Step 5: Implement workflow state derivation**

Use exact precedence:

1. `not_applicable` applicability -> `not_applicable`.
2. `deferred` applicability -> `deferred`.
3. compatible valid current evidence with passed outcome -> `qualified`.
4. compatible valid current evidence with failed outcome -> `failed`.
5. no compatible valid evidence but historical valid evidence exists -> `stale`.
6. otherwise -> `missing`.

- [ ] **Step 6: Implement device support-tier derivation**

Use exact rules:

```js
export function deriveDeviceSupportTier(workflowStates) {
  if (workflowStates.some((row) => row.targetWideFailure !== null)) return "unqualified";
  const required = workflowStates.filter((row) => row.applicability === "required");
  if (required.length === 0) return "unqualified";
  if (required.every((row) => row.state === "qualified")) return "qualified";
  if (required.some((row) => row.state === "qualified")) return "limited";
  return "unqualified";
}
```

This intentionally prevents an arbitrary workflow failure from globally invalidating successful evidence while still ensuring no-evidence targets are not called supported.

- [ ] **Step 7: Run the full Phase 6F tests and verify GREEN**

Run:

```bash
node --test tools/phase-6f-qualification.test.mjs
```

Expected: PASS through Tasks 1–3.

- [ ] **Step 8: Commit the projection slice**

```bash
git add tools/phase-6f-qualification.mjs tools/phase-6f-qualification.test.mjs tests/fixtures/phase-6f
git commit -m "feat: derive phase 6f support state"
```

---

### Task 4: Production Evidence Boundary, Matrix Generation, and Drift Detection

**Files:**
- Modify: `tools/phase-6f-qualification.mjs`
- Modify: `tools/phase-6f-qualification.test.mjs`
- Create: `tests/fixtures/phase-6f/matrix/expected-qualified-limited.md`
- Create: `docs/qualification/phase-6f-device-matrix.md`

**Interfaces:**
- Consumes: Tasks 1–3 loaders and projectors.
- Produces: `loadEvidenceDirectory(path, { fixtureMode })`, `projectQualificationState(context)`, `renderQualificationMatrix(projection)`, CLI modes `--check` and `--write-matrix`.

- [ ] **Step 1: Write failing production-boundary and matrix tests**

Tests must prove fixture paths are rejected in production mode, only `.json` evidence files are accepted in the production evidence directory, matrix output is deterministic, and committed matrix drift is detectable.

```js
test("production evidence loading rejects synthetic fixture paths", () => {
  assert.throws(
    () => loadEvidenceDirectory(
      path.join(repoRoot, "tests/fixtures/phase-6f/projection/evidence"),
      { fixtureMode: false },
    ),
    /synthetic fixture path cannot be used as production evidence/,
  );
});

test("matrix rendering is byte deterministic", () => {
  const projection = syntheticProjection();
  assert.equal(
    renderQualificationMatrix(projection),
    readFileSync(path.join(fixtures, "matrix/expected-qualified-limited.md"), "utf8"),
  );
});
```

- [ ] **Step 2: Run the focused matrix tests and verify RED**

Run:

```bash
node --test --test-name-pattern="production evidence|matrix|drift" tools/phase-6f-qualification.test.mjs
```

Expected: FAIL because the production loader and renderer do not exist.

- [ ] **Step 3: Implement hard production/fixture path separation**

Resolve real paths and reject any production evidence path under `tests/fixtures/`.

```js
function assertProductionEvidencePath(evidenceDir, repoRoot) {
  const resolvedEvidence = path.resolve(evidenceDir);
  const fixtureRoot = path.resolve(repoRoot, "tests/fixtures");
  if (resolvedEvidence === fixtureRoot || resolvedEvidence.startsWith(`${fixtureRoot}${path.sep}`)) {
    fail("synthetic fixture path cannot be used as production evidence");
  }
}
```

The default production evidence directory is exactly `docs/testing/phase-6f/evidence`.

- [ ] **Step 4: Implement deterministic projection and Markdown rendering**

The matrix must include:

- target ID and observed configuration;
- authored profile ID;
- overall support tier;
- every applicable workflow;
- workflow state;
- current run ID/date where present;
- failure/stale/deferred/missing reason where applicable;
- limitations.

With the production `device-targets.json` empty, the initial generated file must truthfully state:

```markdown
# Phase 6F Physical-Device Qualification Matrix

Generated from `docs/testing/phase-6f/` definitions and immutable physical evidence.

No physical-device qualification targets have been registered yet. Phase 6F foundation infrastructure exists, but no device or workflow is physically qualified by this repository state.
```

Do not emit fake rows to demonstrate formatting.

- [ ] **Step 5: Implement CLI check/write modes**

Supported commands:

```bash
node tools/phase-6f-qualification.mjs --check
node tools/phase-6f-qualification.mjs --write-matrix
```

`--check` must validate definitions/evidence, render the expected matrix in memory, compare it byte-for-byte with `docs/qualification/phase-6f-device-matrix.md`, and exit non-zero on drift. `--write-matrix` may update only the generated matrix after all inputs validate.

- [ ] **Step 6: Run tests plus production check and verify GREEN**

Run:

```bash
node --test tools/phase-6f-qualification.test.mjs
node tools/phase-6f-qualification.mjs --write-matrix
node tools/phase-6f-qualification.mjs --check
```

Expected: all commands exit 0; the generated matrix contains no physical target rows.

- [ ] **Step 7: Commit matrix generation**

```bash
git add tools/phase-6f-qualification.mjs tools/phase-6f-qualification.test.mjs tests/fixtures/phase-6f docs/qualification/phase-6f-device-matrix.md
git commit -m "feat: generate phase 6f qualification matrix"
```

---

### Task 5: Operator Runbook and Harness Evidence Boundary

**Files:**
- Create: `docs/manual/phase-6f-qualification-operator.md`
- Modify: `tools/phase-6f-qualification.test.mjs`

**Interfaces:**
- Consumes: Tasks 1–4 exact definitions, evidence fields, CLI, and projection semantics.
- Produces: documented future physical-run procedure; no executable alternate product harness is added in this foundation task.

- [ ] **Step 1: Add a documentation-contract test**

Assert the runbook contains the exact supported commands and explicitly states the boundaries required by the design.

```js
test("operator runbook documents the evidence boundary without claiming physical qualification", () => {
  const runbook = readFileSync(
    path.join(repoRoot, "docs/manual/phase-6f-qualification-operator.md"),
    "utf8",
  );
  assert.match(runbook, /production EmuChef remains the system under test/i);
  assert.match(runbook, /node tools\/phase-6f-qualification\.mjs --check/);
  assert.match(runbook, /unable_to_verify/);
  assert.match(runbook, /does not itself imply support/i);
  assert.doesNotMatch(runbook, /first qualified device/i);
});
```

- [ ] **Step 2: Run the documentation test and verify RED**

Run:

```bash
node --test --test-name-pattern="operator runbook" tools/phase-6f-qualification.test.mjs
```

Expected: FAIL because the runbook does not exist.

- [ ] **Step 3: Write the exact later operator workflow**

The runbook must define this sequence:

1. choose an existing canonical workflow ID;
2. register/capture a device target from observed facts and an existing authored profile ID;
3. verify prerequisites and production capability applicability;
4. capture current EmuChef build identity, workflow version, exact relevant authored recipe SHA-256 digests, runtime contract version, device facts, root state, and connection type;
5. execute the real production EmuChef workflow through its ordinary reviewed execution boundary;
6. collect required automated observations from product outputs and device observations;
7. collect only declared human checkpoints using `pass`, `fail`, or `unable_to_verify`;
8. distinguish an invalid harness/infrastructure run from a valid product failure;
9. create a new immutable evidence JSON record without overwriting an older run;
10. run `node tools/phase-6f-qualification.mjs --check` after adding evidence;
11. regenerate with `--write-matrix` only after validation succeeds;
12. rerun `--check` and repository tests before committing evidence.

Explicitly state that the foundation does not yet automate steps 2–8 end-to-end and that future harness work must call production boundaries rather than introducing qualification-only planner/executor behavior.

- [ ] **Step 4: Run the full Phase 6F test suite and verify GREEN**

Run:

```bash
node --test tools/phase-6f-qualification.test.mjs
```

Expected: PASS.

- [ ] **Step 5: Commit the operator contract**

```bash
git add docs/manual/phase-6f-qualification-operator.md tools/phase-6f-qualification.test.mjs
git commit -m "docs: add phase 6f qualification runbook"
```

---

### Task 6: Repository Validation, CI, and Roadmap Truthfulness

**Files:**
- Modify: `Makefile`
- Modify: `.github/workflows/emuchef-execution-feature-matrix.yml`
- Modify: `docs/product/product-roadmap.md`
- Modify: `tools/phase-6f-qualification.test.mjs`

**Interfaces:**
- Consumes: complete Phase 6F validator/test/matrix/runbook stack.
- Produces: local `make phase-6f-qualification-check`, CI enforcement, and truthful Phase 6F roadmap state.

- [ ] **Step 1: Add a failing repository-integration test**

Verify the Makefile and CI workflow name the Phase 6F commands and that the roadmap says `In progress` while explicitly retaining the no-physical-evidence limitation.

```js
test("repository validation wires phase 6f without claiming completion", () => {
  const makefile = readFileSync(path.join(repoRoot, "Makefile"), "utf8");
  const workflow = readFileSync(
    path.join(repoRoot, ".github/workflows/emuchef-execution-feature-matrix.yml"),
    "utf8",
  );
  const roadmap = readFileSync(path.join(repoRoot, "docs/product/product-roadmap.md"), "utf8");

  assert.match(makefile, /phase-6f-qualification-check:/);
  assert.match(workflow, /node tools\/phase-6f-qualification\.mjs --check/);
  assert.match(workflow, /node --test tools\/phase-6f-qualification\.test\.mjs/);
  assert.match(roadmap, /6F \| Physical-device test matrix \| In progress/);
  assert.match(roadmap, /no physical-device qualification evidence has been added/i);
});
```

- [ ] **Step 2: Run the integration test and verify RED**

Run:

```bash
node --test --test-name-pattern="repository validation wires phase 6f" tools/phase-6f-qualification.test.mjs
```

Expected: FAIL before Makefile/CI/roadmap edits.

- [ ] **Step 3: Add the Makefile target and include it in `test`**

Update `.PHONY` and add:

```make
phase-6f-qualification-check:
	node --test tools/phase-6f-qualification.test.mjs
	node tools/phase-6f-qualification.mjs --check
```

Make `test` depend on or invoke `phase-6f-qualification-check` without requiring ADB or physical hardware.

- [ ] **Step 4: Wire Phase 6F into the existing execution feature matrix workflow**

Add path triggers for:

```yaml
- "tools/phase-6f-*"
- "docs/testing/phase-6f/**"
- "docs/qualification/phase-6f-device-matrix.md"
- "docs/manual/phase-6f-qualification-operator.md"
```

Add a Node-only step to the existing qualification job:

```yaml
- name: Validate Phase 6F qualification foundation
  run: |
    node --test tools/phase-6f-qualification.test.mjs
    node tools/phase-6f-qualification.mjs --check
```

Do not add ADB, emulator, or physical-device requirements to CI.

- [ ] **Step 5: Update the roadmap truthfully**

Change the Phase 6F status to `In progress` and add completion evidence for the foundation only. The text must state that versioned definitions, immutable evidence validation, applicability/compatibility projection, support tiers, synthetic tests, deterministic matrix generation, and operator workflow now exist, while **no physical-device qualification evidence has been added and no device is newly claimed as supported**.

Do not mark Phase 6F `Completed`.

- [ ] **Step 6: Run all Phase 6F and repository-focused validation**

Run:

```bash
node --test tools/phase-6f-qualification.test.mjs
node tools/phase-6f-qualification.mjs --check
make phase-6f-qualification-check
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --check
```

Expected: all commands exit 0. The Rust commands confirm the foundation did not disturb the production runtime even though no backend Rust files should need modification.

- [ ] **Step 7: Run the repository integration test and verify GREEN**

Run:

```bash
node --test --test-name-pattern="repository validation wires phase 6f" tools/phase-6f-qualification.test.mjs
```

Expected: PASS.

- [ ] **Step 8: Commit repository integration**

```bash
git add Makefile .github/workflows/emuchef-execution-feature-matrix.yml docs/product/product-roadmap.md tools/phase-6f-qualification.test.mjs
git commit -m "ci: enforce phase 6f qualification foundation"
```

---

## Final Verification

After all tasks, run the complete bounded verification set from a clean working state:

```bash
node --test tools/phase-6f-qualification.test.mjs
node tools/phase-6f-qualification.mjs --check
make phase-6f-qualification-check
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --check
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets --features real-execution
```

Then verify repository truthfulness manually from Git state:

- `docs/testing/phase-6f/evidence/` contains only `README.md` and no physical JSON records.
- `docs/testing/phase-6f/device-targets.json` still contains an empty production `targets` array unless a separately approved physical qualification task has occurred; this plan does not authorize one.
- `docs/qualification/phase-6f-device-matrix.md` contains no fabricated device rows or support claims.
- no product-runtime planner, executor, public DTO, Tauri, or React files changed for this foundation.
- the roadmap says Phase 6F is `In progress`, not `Completed`.
- Phase 6D deferred physical/manual requirements and Phase 6E automated qualification records remain unchanged.
