# Device Qualification Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a development-only, production-bound physical-device qualification harness that registers observed device targets, drives EmuChef's existing reviewed real-execution workflow, captures immutable evidence bundles, and projects repository qualification state without introducing a second planner, executor, device authority, or evidence validator.

**Architecture:** Keep `tools/device-qualification.mjs` as the single canonical repository evidence authority. Trusted Rust/Tauri owns build gating, live session integrity, product-bound observations, opaque candidate persistence, and bounded invocation of that tool; React adds a qualification overlay around the existing application workflow and never receives filesystem/process authority. Target registration and run promotion require clean committed builds, while compatibility uses a material build-content digest rather than Git SHA so evidence-only commits do not self-invalidate prior runs.

**Tech Stack:** Node.js 22 built-ins, Rust/Tauri 2, serde/serde_json, async-process/std::process, React 19, TypeScript 6, Vitest/Testing Library, existing EmuChef Rust backend and reviewed real-execution APIs.

**Spec:** `docs/superpowers/specs/2026-08-23-device-qualification-harness-design.md`

## Global Constraints

- Production EmuChef remains the system under test: reuse existing device discovery/probe/match/qualification, configuration, review, real-execution, and report boundaries.
- Do not add qualification-only planner, executor, device, root, or ADB authority.
- React remains presentation/operator interaction only; it must not receive arbitrary paths, run Git/Node/ADB, compute canonical digests, or author automated observation outcomes.
- `tools/device-qualification.mjs` is the sole canonical authority for schema validation, canonicalization, target/run IDs, record/fingerprint digests, repository mutation, compatibility projection, and matrix rendering.
- Device-target and evidence records use schema version 2. The workflow catalog remains schema version 1 unless its own shape actually changes.
- New active source files, APIs, scripts, runtime directories, tools, fixtures, and qualification data paths must not contain phase/slice nomenclature. Historical roadmap, Superpowers plan/spec, and `.chatgpt` records may retain historical names.
- Qualification mode requires all four gates: debug build, `real-execution`, valid clean-build metadata, and `EMUCHEF_DEVICE_QUALIFICATION=1`.
- Exact Git commit is immutable audit/pre-promotion provenance. `emuchef_build` compatibility compares app version + material build-content digest + real-execution state + qualification contract version, not Git SHA.
- The material build-content digest excludes qualification evidence, generated qualification matrix, qualification/operator docs, tests/fixtures, and ignored runtime candidates so recording evidence does not immediately stale it.
- Root state comes only from the existing explicit root check. Initial operator attestation is restricted to `connectionType: "usb2" | "usb3"`.
- Target IDs are deterministic `device-target-sha256:<64 lowercase hex>` values; run IDs are deterministic `qualification-run-sha256:<64 lowercase hex>` values. Operators author neither.
- Target registration is a separate build lifecycle from recordable workflow qualification: register -> commit -> build clean qualification binary -> qualify.
- Non-authoritative candidates live only under `.emuchef_runtime/qualification-candidates/` and never influence matrix state.
- Every valid run must preserve exactly one digest-bound production `execution-report.json`. An invalid/not-observed audit run may omit it only when report capture itself failed or never became available.
- Invalid runs may be explicitly recorded for audit but are never selectable as current qualification evidence.
- Do not perform or claim physical qualification while implementing this plan. All automated tests must run without connected hardware.
- Use dependency-free Node 22 built-ins for repository qualification tooling; add no npm runtime dependency for the evidence engine.

---

## File Structure

### Canonical repository evidence layer

- Rename `tools/phase-6f-qualification.mjs` -> `tools/device-qualification.mjs`: canonical definitions/evidence loader, schema-v2 validator, canonicalization/digests, material build identity, candidate recording, projection, matrix CLI.
- Rename `tools/phase-6f-qualification.test.mjs` -> `tools/device-qualification.test.mjs`: exhaustive Node contract and mutation tests.
- Rename `docs/testing/phase-6f/` -> `docs/testing/device-qualification/`: workflow catalog, target registry, evidence schema, immutable evidence bundles.
- Rename `tests/fixtures/phase-6f/` -> `tests/fixtures/device-qualification/`: synthetic targets, run bundles, invalid cases, projection fixtures.
- Rename `docs/qualification/phase-6f-device-matrix.md` -> `docs/qualification/device-qualification-matrix.md`: deterministic generated projection.
- Rename `docs/manual/phase-6f-qualification-operator.md` -> `docs/manual/device-qualification-operator.md`: operator procedure.

### Trusted Tauri layer

- Create `apps/emuchef-app/src-tauri/src/qualification_build.rs`: embedded build identity, four-gate mode evaluation, source-state pre-promotion checks.
- Create `apps/emuchef-app/src-tauri/src/qualification_repository.rs`: fixed repository/runtime roots, opaque candidate storage/recovery, bounded Node tool invocation.
- Create `apps/emuchef-app/src-tauri/src/qualification_mode.rs`: target registration and workflow qualification session state machine plus Tauri DTOs/commands.
- Modify `apps/emuchef-app/src-tauri/build.rs`: call canonical Node build-identity operation only for qualification builds and embed validated JSON.
- Modify `apps/emuchef-app/src-tauri/src/lib.rs`: manage qualification stores and register guarded commands.
- Modify `apps/emuchef-app/src-tauri/src/commands.rs`: expose firmware build through the existing production device probe and reusable trusted internal helpers.
- Modify `apps/emuchef-app/src-tauri/src/execution.rs`: share one production report serialization path between normal export and qualification capture.
- Modify `crates/emuchef-rust-backend/src/device_probe.rs`: include Android build fingerprint in the existing `getprop` probe rather than adding qualification-only ADB.

### Qualification launcher and frontend overlay

- Create `apps/emuchef-app/scripts/run-device-qualification.mjs`: pinned, non-hot-reload qualification launcher.
- Create `apps/emuchef-app/src/useDeviceQualificationMode.ts`: React-side state orchestration around opaque Tauri handles and existing `WorkflowState`.
- Create `apps/emuchef-app/src/DeviceQualificationOverlay.tsx`: development-only controller/banner, target registration, checkpoint capture, and explicit promotion UI.
- Create `apps/emuchef-app/tests/useDeviceQualificationMode.dom.test.tsx`: workflow-binding/session transition tests.
- Create `apps/emuchef-app/tests/DeviceQualificationOverlay.dom.test.tsx`: operator UI and safety tests.
- Modify `apps/emuchef-app/src/App.tsx`: mount overlay over the normal app and lock declared device-plan/recipe intent while a qualification session is active.
- Modify `apps/emuchef-app/src/api.ts` and `apps/emuchef-app/src/types.ts`: opaque qualification DTO/command surface.
- Modify `apps/emuchef-app/src/styles.css`: qualification overlay presentation only.

---

### Task 1: Migrate the Active Qualification Foundation to Domain-Oriented Names

**Files:**
- Rename: `tools/phase-6f-qualification.mjs` -> `tools/device-qualification.mjs`
- Rename: `tools/phase-6f-qualification.test.mjs` -> `tools/device-qualification.test.mjs`
- Rename: `docs/testing/phase-6f/**` -> `docs/testing/device-qualification/**`
- Rename: `tests/fixtures/phase-6f/**` -> `tests/fixtures/device-qualification/**`
- Rename: `docs/qualification/phase-6f-device-matrix.md` -> `docs/qualification/device-qualification-matrix.md`
- Rename: `docs/manual/phase-6f-qualification-operator.md` -> `docs/manual/device-qualification-operator.md`
- Modify: `Makefile`
- Modify: `.github/workflows/emuchef-execution-feature-matrix.yml`
- Modify: any non-historical repository references to the renamed active paths

**Interfaces:**
- Consumes: existing schema-v1 qualification behavior unchanged.
- Produces: canonical active paths `tools/device-qualification.mjs`, `docs/testing/device-qualification/`, `tests/fixtures/device-qualification/`, `docs/qualification/device-qualification-matrix.md`, and Make target `device-qualification-check`.

- [ ] **Step 1: Add a failing domain-name hygiene test before renaming**

Append this test to the existing `tools/phase-6f-qualification.test.mjs` so it fails against the current tree:

```js
test("active device qualification artifacts use domain-oriented names", () => {
  const forbidden = [
    "tools/phase-6f-qualification.mjs",
    "tools/phase-6f-qualification.test.mjs",
    "docs/testing/phase-6f",
    "tests/fixtures/phase-6f",
    "docs/qualification/phase-6f-device-matrix.md",
    "docs/manual/phase-6f-qualification-operator.md",
  ];
  for (const relative of forbidden) {
    assert.equal(existsSync(path.join(REPO_ROOT, relative)), false, relative);
  }
  const makefile = readFileSync(path.join(REPO_ROOT, "Makefile"), "utf8");
  assert.doesNotMatch(makefile, /phase-6f-qualification-check/);
});
```

- [ ] **Step 2: Run the focused test and verify the expected failure**

Run:

```sh
node --test --test-name-pattern="active device qualification artifacts use domain-oriented names" tools/phase-6f-qualification.test.mjs
```

Expected: FAIL because the old active paths and Make target still exist.

- [ ] **Step 3: Rename the active artifacts and update every active reference**

Use Git-aware renames and then replace active path/target strings:

```sh
git mv tools/phase-6f-qualification.mjs tools/device-qualification.mjs
git mv tools/phase-6f-qualification.test.mjs tools/device-qualification.test.mjs
git mv docs/testing/phase-6f docs/testing/device-qualification
git mv tests/fixtures/phase-6f tests/fixtures/device-qualification
git mv docs/qualification/phase-6f-device-matrix.md docs/qualification/device-qualification-matrix.md
git mv docs/manual/phase-6f-qualification-operator.md docs/manual/device-qualification-operator.md
```

In the renamed Node tool/test, Makefile, CI workflow, runbook, and generated matrix, use these exact active names:

```text
tools/device-qualification.mjs
tools/device-qualification.test.mjs
docs/testing/device-qualification/workflow-catalog.json
docs/testing/device-qualification/device-targets.json
docs/testing/device-qualification/evidence-schema.json
docs/testing/device-qualification/evidence/
tests/fixtures/device-qualification/
docs/qualification/device-qualification-matrix.md
docs/manual/device-qualification-operator.md
device-qualification-check
```

Do not rewrite historical Superpowers specs/plans, roadmap headings, or `.chatgpt` run artifacts just to remove historical terminology.

- [ ] **Step 4: Keep schema-v1 behavior byte-equivalent after the rename**

The Node imports and production constants must point at the new domain paths, for example:

```js
const FIXTURES = path.join(REPO_ROOT, "tests/fixtures/device-qualification");
const QUALIFICATION_ROOT = path.join(REPO_ROOT, "docs/testing/device-qualification");
const MATRIX_PATH = path.join(REPO_ROOT, "docs/qualification/device-qualification-matrix.md");
```

Update `Makefile` to expose only the new target:

```make
.PHONY: help install ensure-deps build test device-qualification-check cargo-test-freshness-check backend-test-fresh emuchef-app config-editor dev

test: ensure-deps device-qualification-check cargo-test-freshness-check backend-test-fresh

device-qualification-check:
	node --test tools/device-qualification.test.mjs
	node tools/device-qualification.mjs --check
```

- [ ] **Step 5: Run the renamed foundation tests and repository check**

Run:

```sh
node --test tools/device-qualification.test.mjs
node tools/device-qualification.mjs --check
make device-qualification-check
```

Expected: PASS with the same schema-v1 semantics and no physical evidence added.

- [ ] **Step 6: Commit the naming migration**

```sh
git add Makefile .github/workflows/emuchef-execution-feature-matrix.yml tools/device-qualification.mjs tools/device-qualification.test.mjs docs/testing/device-qualification tests/fixtures/device-qualification docs/qualification/device-qualification-matrix.md docs/manual/device-qualification-operator.md
git commit -m "refactor: rename device qualification foundation"
```

---

### Task 2: Upgrade Device Targets and Evidence Facts to Schema Version 2

**Files:**
- Modify: `tools/device-qualification.mjs`
- Modify: `tools/device-qualification.test.mjs`
- Modify: `docs/testing/device-qualification/device-targets.json`
- Modify: `docs/testing/device-qualification/evidence-schema.json`
- Modify: `tests/fixtures/device-qualification/definitions-valid/device-targets.json`
- Modify: `tests/fixtures/device-qualification/definitions-invalid/*.json`
- Modify: `tests/fixtures/device-qualification/evidence-valid/**`
- Modify: `tests/fixtures/device-qualification/evidence-invalid/**`
- Modify: `tests/fixtures/device-qualification/projection/**`

**Interfaces:**
- Consumes: Task 1 domain paths.
- Produces: `deviceTargetId(target) -> "device-target-sha256:<hex>"`, schema-v2 fact wrappers, `targetFactValue(target, field)`, evidence fingerprint schema version 2.

- [ ] **Step 1: Write failing tests for typed provenance and deterministic target IDs**

Add tests with this contract:

```js
const observed = (value) => ({ value, source: "production_observation" });
const rooted = (value) => ({ value, source: "explicit_root_check" });
const attested = (value) => ({ value, source: "operator_attestation" });

test("schema-v2 device targets require legal per-fact provenance and deterministic ids", () => {
  const target = {
    id: "",
    profileId: observed("ayaneo.pocket_s2"),
    manufacturer: observed("AYANEO"),
    model: observed("Pocket S2"),
    androidVersion: observed("15"),
    androidApi: observed(35),
    abiSocClass: observed("arm64"),
    rootState: rooted("non_root"),
    connectionType: attested("usb3"),
    firmwareBuild: observed("vendor/device/build:15/ABC/123:user/release-keys"),
    capabilities: ["apk_install", "shared_storage_write"],
    deferredWorkflows: [],
  };
  const id = deviceTargetId(target);
  assert.match(id, /^device-target-sha256:[0-9a-f]{64}$/);
  target.id = id;
  assert.doesNotThrow(() => validateDeviceTargets(
    { schemaVersion: 2, targets: [target] },
    { authoredProfilesDir: AUTHORED_PROFILES },
  ));

  const illegalRoot = structuredClone(target);
  illegalRoot.rootState.source = "operator_attestation";
  illegalRoot.id = deviceTargetId(illegalRoot);
  assert.throws(
    () => validateDeviceTargets(
      { schemaVersion: 2, targets: [illegalRoot] },
      { authoredProfilesDir: AUTHORED_PROFILES },
    ),
    /rootState.*explicit_root_check/i,
  );
});

test("target identity excludes provenance source and policy fields", () => {
  const target = structuredClone(readJson("definitions-valid/device-targets.json").targets[0]);
  const original = deviceTargetId(target);
  target.capabilities = [];
  target.deferredWorkflows = ["xaniteog-install"];
  target.connectionType.source = "production_observation";
  assert.equal(deviceTargetId(target), original);
  target.connectionType.value = target.connectionType.value === "usb3" ? "usb2" : "usb3";
  assert.notEqual(deviceTargetId(target), original);
});
```

- [ ] **Step 2: Run the focused tests and verify they fail on schema v1**

Run:

```sh
node --test --test-name-pattern="schema-v2 device targets|target identity excludes" tools/device-qualification.test.mjs
```

Expected: FAIL because `deviceTargetId` and schema-v2 fact validation do not exist.

- [ ] **Step 3: Implement the schema-v2 fact helpers and target identity**

Add these canonical constants/helpers to `tools/device-qualification.mjs`:

```js
const FACT_SOURCES = new Set([
  "production_observation",
  "explicit_root_check",
  "operator_attestation",
]);
const TARGET_FACT_FIELDS = [
  "profileId",
  "manufacturer",
  "model",
  "androidVersion",
  "androidApi",
  "abiSocClass",
  "rootState",
  "connectionType",
  "firmwareBuild",
];
const TARGET_ID_PATTERN = /^device-target-sha256:[0-9a-f]{64}$/;

export function targetFactValue(target, field) {
  return target[field].value;
}

export function deviceTargetIdentityPayload(target) {
  return Object.fromEntries(TARGET_FACT_FIELDS.map((field) => [field, targetFactValue(target, field)]));
}

export function deviceTargetId(target) {
  return `device-target-sha256:${canonicalDigest(deviceTargetIdentityPayload(target)).slice("sha256:".length)}`;
}
```

Validate exact `{ value, source }` fact wrappers and enforce this initial legal-source matrix:

```js
const FACT_SOURCE_BY_FIELD = new Map([
  ["profileId", new Set(["production_observation"])],
  ["manufacturer", new Set(["production_observation"])],
  ["model", new Set(["production_observation"])],
  ["androidVersion", new Set(["production_observation"])],
  ["androidApi", new Set(["production_observation"])],
  ["abiSocClass", new Set(["production_observation"])],
  ["rootState", new Set(["explicit_root_check"])],
  ["connectionType", new Set(["operator_attestation", "production_observation"])],
  ["firmwareBuild", new Set(["production_observation"])],
]);
```

Require `target.id === deviceTargetId(target)` and update projection code to read `.value` through `targetFactValue` rather than comparing wrapper objects.

- [ ] **Step 4: Upgrade the JSON schema and all current valid fixtures to schema v2**

Change production `device-targets.json` to:

```json
{
  "schemaVersion": 2,
  "targets": []
}
```

Update `evidence-schema.json` so the top-level record and fingerprint use schema version 2 and the embedded `deviceTarget` carries the same typed fact wrappers as the registered target.

For synthetic target IDs, compute the exact ID through the new canonical helper rather than inventing one:

```sh
node --input-type=module -e 'import fs from "node:fs"; import { deviceTargetId } from "./tools/device-qualification.mjs"; const p="tests/fixtures/device-qualification/definitions-valid/device-targets.json"; const x=JSON.parse(fs.readFileSync(p,"utf8")); for (const t of x.targets) console.log(t.model.value, deviceTargetId(t));'
```

Write those generated IDs back into every fixture reference. Update current valid evidence/projection fixtures to schema version 2 and fact-wrapper values. Keep version-1 fixtures only when a test explicitly proves version-1 rejection.

- [ ] **Step 5: Update compatibility/projection tests for wrapped facts**

`buildCurrentFingerprint` must project values, not source wrappers:

```js
return {
  schemaVersion: 2,
  emuchefBuild: currentBuild,
  workflowVersion: workflow.version,
  authoredContent,
  runtimeContract,
  deviceProfile: targetFactValue(target, "profileId"),
  androidApi: targetFactValue(target, "androidApi"),
  firmwareBuild: targetFactValue(target, "firmwareBuild"),
  abiSocClass: targetFactValue(target, "abiSocClass"),
  rootState: targetFactValue(target, "rootState"),
  connectionType: targetFactValue(target, "connectionType"),
};
```

Update matrix rendering to display `.value` fields while preserving provenance in canonical JSON.

- [ ] **Step 6: Run the complete Node qualification suite**

Run:

```sh
node --test tools/device-qualification.test.mjs
node tools/device-qualification.mjs --write-matrix
node tools/device-qualification.mjs --check
```

Expected: PASS; production target registry still has no physical targets.

- [ ] **Step 7: Commit schema-v2 target provenance**

```sh
git add tools/device-qualification.mjs tools/device-qualification.test.mjs docs/testing/device-qualification tests/fixtures/device-qualification docs/qualification/device-qualification-matrix.md
git commit -m "feat: add device qualification provenance"
```

---

### Task 3: Add Canonical Material Build Identity and Repository Description

**Files:**
- Modify: `tools/device-qualification.mjs`
- Modify: `tools/device-qualification.test.mjs`
- Modify: `docs/testing/device-qualification/evidence-schema.json`
- Modify: schema-v2 evidence/projection fixtures under `tests/fixtures/device-qualification/**`

**Interfaces:**
- Consumes: schema-v2 target/fingerprint model from Task 2.
- Produces:
  - `QUALIFICATION_CONTRACT_VERSION = 1`
  - `RUNTIME_CONTRACT = "real-execution-v1"`
  - `materialBuildDigestFromEntries(entries) -> "sha256:<hex>"`
  - `buildMaterialIdentity({ repoRoot, requireClean }) -> QualificationBuildIdentity`
  - CLI `--build-identity [--require-clean]`
  - CLI `--describe`

- [ ] **Step 1: Write failing unit tests for material build identity semantics**

Add:

```js
test("material build digest changes for product inputs but ignores qualification evidence", () => {
  const base = [
    { path: "authored/recipes/app.retroarch.provision.yaml", sha256: "a".repeat(64) },
    { path: "crates/emuchef-rust-backend/src/planner.rs", sha256: "b".repeat(64) },
  ];
  const original = materialBuildDigestFromEntries(base);
  const changedProduct = materialBuildDigestFromEntries([
    base[0],
    { path: base[1].path, sha256: "c".repeat(64) },
  ]);
  assert.notEqual(changedProduct, original);

  const withEvidence = materialBuildDigestFromEntries([
    ...base,
    { path: "docs/testing/device-qualification/evidence/example/evidence.json", sha256: "d".repeat(64) },
  ]);
  assert.equal(withEvidence, original);
});

test("emuchef build compatibility ignores git commit but honors material content", () => {
  const left = {
    appVersion: "0.1.0",
    gitCommit: "1".repeat(40),
    materialBuildDigest: `sha256:${"a".repeat(64)}`,
    realExecutionEnabled: true,
    qualificationContract: 1,
  };
  const evidence = { ...left, gitCommit: "2".repeat(40) };
  assert.equal(compareBuildIdentity(left, evidence), "compatible");
  assert.equal(
    compareBuildIdentity(left, { ...evidence, materialBuildDigest: `sha256:${"b".repeat(64)}` }),
    "invalidating",
  );
});
```

- [ ] **Step 2: Verify the new tests fail**

Run:

```sh
node --test --test-name-pattern="material build digest|emuchef build compatibility" tools/device-qualification.test.mjs
```

Expected: FAIL because the build-identity helpers are undefined and `emuchefBuild` is still a string.

- [ ] **Step 3: Implement one canonical material-input filter and digest**

Define the qualification-only exclusions in the Node tool; all other tracked files beneath the material product roots participate:

```js
const MATERIAL_ROOTS = [
  "authored/",
  "crates/emuchef-rust-backend/",
  "apps/emuchef-app/src/",
  "apps/emuchef-app/src-tauri/src/",
];
const MATERIAL_EXACT_FILES = new Set([
  "apps/emuchef-app/package.json",
  "apps/emuchef-app/package-lock.json",
  "apps/emuchef-app/src-tauri/Cargo.toml",
  "apps/emuchef-app/src-tauri/Cargo.lock",
  "apps/emuchef-app/src-tauri/tauri.conf.json",
]);
const MATERIAL_EXCLUDED = [
  "apps/emuchef-app/src/DeviceQualificationOverlay.tsx",
  "apps/emuchef-app/src/useDeviceQualificationMode.ts",
  "apps/emuchef-app/src-tauri/src/qualification_build.rs",
  "apps/emuchef-app/src-tauri/src/qualification_repository.rs",
  "apps/emuchef-app/src-tauri/src/qualification_mode.rs",
  "apps/emuchef-app/src/Phase6d6UiSmoke.tsx",
  "apps/emuchef-app/src-tauri/src/phase6d6_ui_smoke.rs",
];

function isMaterialBuildPath(relativePath) {
  if (MATERIAL_EXCLUDED.includes(relativePath)) return false;
  return MATERIAL_EXACT_FILES.has(relativePath)
    || MATERIAL_ROOTS.some((prefix) => relativePath.startsWith(prefix));
}

export function materialBuildDigestFromEntries(entries) {
  const material = entries
    .filter((entry) => isMaterialBuildPath(entry.path))
    .sort((a, b) => a.path.localeCompare(b.path));
  return canonicalDigest(material);
}
```

Use `execFileSync("git", ["ls-files", "-z"], { cwd: repoRoot })` to enumerate tracked files, SHA-256 each file's raw bytes, and pass those `{path, sha256}` entries through this one function. Use `git status --porcelain --untracked-files=no` only for the `requireClean` gate; do not make ignored runtime candidates count as dirtiness.

- [ ] **Step 4: Define the exact build identity and special compatibility comparison**

Use this canonical object in the evidence fingerprint:

```js
export const QUALIFICATION_CONTRACT_VERSION = 1;
export const RUNTIME_CONTRACT = "real-execution-v1";

export function compareBuildIdentity(current, evidence) {
  return current.appVersion === evidence.appVersion
    && current.materialBuildDigest === evidence.materialBuildDigest
    && current.realExecutionEnabled === evidence.realExecutionEnabled
    && current.qualificationContract === evidence.qualificationContract
    ? "compatible"
    : "invalidating";
}
```

`buildMaterialIdentity` returns the values it computes directly, with no caller-authored fields:

```js
export function buildMaterialIdentity({ repoRoot, requireClean = false }) {
  if (requireClean && trackedWorktreeStatus(repoRoot) !== "") {
    fail("device qualification requires a clean tracked worktree");
  }
  const packageJson = JSON.parse(readFileSync(path.join(repoRoot, "apps/emuchef-app/package.json"), "utf8"));
  return {
    appVersion: packageJson.version,
    gitCommit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: repoRoot, encoding: "utf8" }).trim(),
    materialBuildDigest: materialBuildDigest(repoRoot),
    realExecutionEnabled: true,
    qualificationContract: QUALIFICATION_CONTRACT_VERSION,
  };
}
```

Update `compareDimension("emuchef_build", ...)` to call `compareBuildIdentity` instead of deep/equal JSON on `gitCommit`.

- [ ] **Step 5: Add machine-readable CLI operations and remove manual identity environment variables**

Support exactly:

```text
node tools/device-qualification.mjs --build-identity
node tools/device-qualification.mjs --build-identity --require-clean
node tools/device-qualification.mjs --describe
node tools/device-qualification.mjs --check
node tools/device-qualification.mjs --write-matrix
```

`--build-identity` prints only one JSON object to stdout. Build `--describe` from the validated loaders rather than hand-written copies:

```js
const description = {
  schemaVersion: 1,
  runtimeContract: RUNTIME_CONTRACT,
  qualificationContract: QUALIFICATION_CONTRACT_VERSION,
  build: buildMaterialIdentity({ repoRoot: REPO_ROOT, requireClean: false }),
  workflowCatalog: loadWorkflowCatalog(path.join(QUALIFICATION_ROOT, "workflow-catalog.json")),
  deviceTargets: loadDeviceTargets(
    path.join(QUALIFICATION_ROOT, "device-targets.json"),
    { authoredProfilesDir: path.join(REPO_ROOT, "authored/device_profiles") },
  ),
};
```

Serialize exactly that object for `--describe`. Remove all active reads of `EMUCHEF_PHASE_6F_BUILD_IDENTITY` and `EMUCHEF_PHASE_6F_RUNTIME_CONTRACT`; production projection obtains build/runtime identity from these repository-owned helpers.

- [ ] **Step 6: Update evidence schema/fixtures for object build identity and verify Git-SHA-only changes stay compatible**

Change `fingerprint.emuchefBuild` from string to the strict object above, including audit `gitCommit`. Add fixture cases proving:

```js
const movedCommit = structuredClone(record.fingerprint);
movedCommit.emuchefBuild.gitCommit = "f".repeat(40);
assert.equal(classifyCompatibility({
  workflow,
  currentFingerprint,
  evidenceFingerprint: movedCommit,
}), "compatible");
```

and a changed `materialBuildDigest` is invalidating for workflows declaring `emuchef_build`.

- [ ] **Step 7: Run all Node checks and commit**

Run:

```sh
node --test tools/device-qualification.test.mjs
node tools/device-qualification.mjs --build-identity
node tools/device-qualification.mjs --describe
node tools/device-qualification.mjs --write-matrix
node tools/device-qualification.mjs --check
```

Expected: PASS. Then commit:

```sh
git add tools/device-qualification.mjs tools/device-qualification.test.mjs docs/testing/device-qualification tests/fixtures/device-qualification docs/qualification/device-qualification-matrix.md
git commit -m "feat: bind qualification to material build identity"
```

---

### Task 4: Add Candidate Validation, Evidence Bundles, and Atomic Canonical Recording

**Files:**
- Modify: `tools/device-qualification.mjs`
- Modify: `tools/device-qualification.test.mjs`
- Modify: `docs/testing/device-qualification/evidence-schema.json`
- Modify: `docs/testing/device-qualification/workflow-catalog.json`
- Modify: `docs/testing/device-qualification/evidence/README.md`
- Modify: workflow/evidence fixtures under `tests/fixtures/device-qualification/**`
- Restructure: valid/invalid evidence fixtures into bundle directories under `tests/fixtures/device-qualification/**`

**Interfaces:**
- Consumes: schema-v2 targets and canonical build identity from Tasks 2-3.
- Produces:
  - `qualification-candidate-<32 lowercase hex>` candidate IDs
  - `registerQualificationTargetCandidate(candidateId, paths)`
  - `recordQualificationRunCandidate(candidateId, paths)`
  - bundle loader returning `{ record, reportBytes }`
  - CLI `--register-target <candidate-id>` and `--record-run <candidate-id>`

- [ ] **Step 1: Write failing tests for run identity, report binding, invalid-report omission, and immutability**

Create fixture/candidate helpers and add these semantic assertions:

```js
test("valid evidence requires one digest-bound production execution report", () => {
  const bundle = loadEvidenceBundle(path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"));
  assert.equal(bundle.record.runValidity, "valid");
  assert.equal(bundle.record.artifacts.length, 1);
  assert.equal(bundle.record.artifacts[0].id, "execution-report");
  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("invalid audit evidence may omit report only when it is not referenced", () => {
  const bundle = loadEvidenceBundle(path.join(FIXTURES, "evidence-valid/invalid-report-unavailable"));
  assert.equal(bundle.record.runValidity, "invalid");
  assert.deepEqual(bundle.record.artifacts, []);
  assert.equal(bundle.reportBytes, null);
  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("changing a bound report invalidates the bundle", () => {
  const bundle = loadEvidenceBundle(path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"));
  assert.throws(
    () => validateEvidenceBundle({ ...bundle, reportBytes: Buffer.from("{}") }, syntheticContext()),
    /execution report digest/i,
  );
});
```

Also add a temp-directory test that records the same target/run twice and requires the second write to be rejected without altering existing bytes.

- [ ] **Step 2: Run the focused tests and verify they fail**

Run:

```sh
node --test --test-name-pattern="valid evidence requires|invalid audit evidence|changing a bound report|immutable" tools/device-qualification.test.mjs
```

Expected: FAIL because evidence is still single-file JSON and mutation operations do not exist.

- [ ] **Step 3: Add the exact artifact, candidate, and human-prerequisite contracts**

The existing `retroarch-plus-bios` workflow has a human-only `clean_or_deliberately_reset_device` prerequisite but no durable observation that proves it. Preserve the prerequisite declaration, add a required human checkpoint with the same semantic identity, and bump only that workflow's qualification version from `1` to `2` because its evidence contract changed:

```json
{
  "id": "clean_or_deliberately_reset_device",
  "instruction": "Before execution, verify the connected device is clean or has been deliberately reset to the intended qualification baseline.",
  "fact": "The device was clean or deliberately reset before this qualification run.",
  "allowedOutcomes": ["pass", "fail", "unable_to_verify"],
  "required": true
}
```

Add Node tests proving the production catalog exposes this checkpoint, that `retroarch-plus-bios.version === 2`, and that a run missing the required checkpoint cannot validate as `valid`. This checkpoint is a pre-execution prerequisite: qualification mode must require an explicit `pass` before binding a review or real execution. `fail` or `unable_to_verify` ends the session as `invalid + not_observed`; it must not become a product qualification failure.

Add top-level evidence field `artifacts`. Execution report entries use:

```json
{
  "id": "execution-report",
  "kind": "production_execution_report",
  "path": "execution-report.json",
  "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}
```

Candidate IDs must match:

```js
const CANDIDATE_ID_PATTERN = /^qualification-candidate-[0-9a-f]{32}$/;
```

Canonical target-registration candidate shape:

```js
const targetRegistrationCandidate = {
  candidateSchemaVersion: 1,
  candidateId: "qualification-candidate-0123456789abcdef0123456789abcdef",
  kind: "target_registration",
  capturedAt: "2026-08-23T12:00:00Z",
  build: {
    appVersion: "0.1.0",
    gitCommit: "1111111111111111111111111111111111111111",
    materialBuildDigest: `sha256:${"a".repeat(64)}`,
    realExecutionEnabled: true,
    qualificationContract: 1,
  },
  target: {
    profileId: { value: "ayaneo.pocket_s2", source: "production_observation" },
    manufacturer: { value: "AYANEO", source: "production_observation" },
    model: { value: "Pocket S2", source: "production_observation" },
    androidVersion: { value: "15", source: "production_observation" },
    androidApi: { value: 35, source: "production_observation" },
    abiSocClass: { value: "arm64", source: "production_observation" },
    rootState: { value: "non_root", source: "explicit_root_check" },
    connectionType: { value: "usb3", source: "operator_attestation" },
    firmwareBuild: { value: "AYANEO/device/build:15/ABC/123:user/release-keys", source: "production_observation" },
    capabilities: ["apk_install", "shared_storage_write"],
    deferredWorkflows: [],
  },
};
```

The Node tool validates the target and authors its deterministic `id`.

Build valid run candidates from already validated context so target/workflow references cannot drift:

```js
const context = syntheticContext();
const workflow = context.workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
const target = context.targets[0];
const build = {
  appVersion: "0.1.0",
  gitCommit: "1111111111111111111111111111111111111111",
  materialBuildDigest: `sha256:${"a".repeat(64)}`,
  realExecutionEnabled: true,
  qualificationContract: 1,
};
const fingerprint = buildCurrentFingerprint({
  workflow,
  target,
  currentBuild: build,
  runtimeContract: RUNTIME_CONTRACT,
  authoredContentDigests: AUTHORED_DIGESTS,
});
const reportBytes = Buffer.from('{"schemaVersion":1,"status":"succeeded"}\n', "utf8");
const reportSha256 = createHash("sha256").update(reportBytes).digest("hex");
const runCandidate = {
  candidateSchemaVersion: 1,
  candidateId: "qualification-candidate-fedcba9876543210fedcba9876543210",
  kind: "qualification_run",
  capturedAt: "2026-08-23T12:30:00Z",
  build,
  workflowId: workflow.id,
  workflowVersion: workflow.version,
  deviceTargetId: target.id,
  fingerprint,
  runValidity: "valid",
  qualificationOutcome: "passed",
  automatedObservations: [
    { id: "execution-report", outcome: "passed", observedAt: "2026-08-23T12:31:00Z" },
  ],
  humanCheckpoints: [
    {
      checkpointId: "clean_or_deliberately_reset_device",
      outcome: "pass",
      observedAt: "2026-08-23T12:30:30Z",
    },
  ],
  targetWideFailure: null,
  limitations: [],
  artifacts: [
    { id: "execution-report", kind: "production_execution_report", path: "execution-report.json", sha256: reportSha256 },
  ],
};
```

The candidate never supplies `runId`, `recordDigest`, or `fingerprintDigest`; the canonical tool authors those.

- [ ] **Step 4: Implement deterministic run sealing and bundle validation**

Use this non-circular order:

```js
export function qualificationRunId(unsealedRecord) {
  const identity = structuredClone(unsealedRecord);
  delete identity.runId;
  delete identity.recordDigest;
  delete identity.fingerprintDigest;
  return `qualification-run-sha256:${canonicalDigest(identity).slice("sha256:".length)}`;
}

export function sealEvidenceRecord(unsealedRecord) {
  const record = structuredClone(unsealedRecord);
  record.fingerprintDigest = evidenceFingerprintDigest(record.fingerprint);
  record.runId = qualificationRunId(record);
  record.recordDigest = evidenceRecordDigest(record);
  return record;
}
```

Bundle validation rules:

```text
valid run                 -> exactly one execution-report artifact + report bytes + required execution-report observation
invalid run with artifact -> referenced report must exist and digest-match
invalid run without report-> artifacts=[] and no report bytes; qualificationOutcome must be not_observed
any referenced artifact   -> path must be exactly execution-report.json; no traversal or alternate filename
```

- [ ] **Step 5: Implement bounded candidate lookup and pre-mutation source checks**

`--register-target` and `--record-run` accept only a candidate ID, never a path. Resolve:

```js
const CANDIDATE_ROOT = path.join(REPO_ROOT, ".emuchef_runtime/qualification-candidates");

function candidateDirectory(candidateId) {
  if (!CANDIDATE_ID_PATTERN.test(candidateId)) fail("qualification candidate id is invalid");
  return path.join(CANDIDATE_ROOT, candidateId);
}
```

Before any canonical mutation, independently require:

```text
candidate.build.gitCommit === git rev-parse HEAD
git status --porcelain --untracked-files=no is empty
candidate.build.materialBuildDigest === current materialBuildDigest
candidate.build.qualificationContract === QUALIFICATION_CONTRACT_VERSION
candidate.build.realExecutionEnabled === true
```

For run recording, reload the canonical workflow catalog, target registry, authored recipe bytes, and runtime contract at promotion time. Require `candidate.workflowVersion` to equal the current workflow version, require `candidate.deviceTargetId` to resolve to the current registered target, recompute every authored-content SHA-256, rebuild the expected fingerprint with `buildCurrentFingerprint`, and require exact canonical equality with `candidate.fingerprint`. Build the evidence record's embedded `deviceTarget` from the registered target, never from candidate-authored target facts. Then validate required automated observations and human checkpoints against that current workflow before sealing the run.

Do not rely solely on Rust/Tauri having checked these conditions.

- [ ] **Step 6: Implement create-new recording with rollback-safe matrix replacement**

Factor filesystem mutation behind a small injectable operations object so tests can force a late failure without OS permission tricks:

```js
const defaultFsOps = {
  mkdirSync,
  renameSync,
  rmSync,
  writeFileSync,
};
```

For target registration: validate candidate -> compute target ID -> compute resulting registry and matrix entirely in memory -> write temporary registry/matrix -> replace canonical files -> rollback original bytes if the second replacement fails.

For run recording: validate candidate/report -> seal record -> render resulting matrix in memory -> write bundle to a temporary sibling directory -> rename bundle create-new -> replace matrix -> remove the newly created bundle if matrix replacement fails. Never overwrite an existing run directory.

- [ ] **Step 7: Restructure fixtures into evidence bundles and add atomicity tests**

Valid fixture shape:

```text
tests/fixtures/device-qualification/evidence-valid/passing-retroarch-bios/
  evidence.json
  execution-report.json
```

Invalid report-unavailable fixture shape:

```text
tests/fixtures/device-qualification/evidence-valid/invalid-report-unavailable/
  evidence.json
```

Update projection fixture loading to recurse bundle directories and validate each as a unit. Add a fake `fsOps.renameSync` failure on the matrix replacement and assert the target registry/evidence root remains byte-identical to the pre-call state.

- [ ] **Step 8: Run all Node tests and commit**

Run:

```sh
node --test tools/device-qualification.test.mjs
node tools/device-qualification.mjs --write-matrix
node tools/device-qualification.mjs --check
```

Expected: PASS. Then commit:

```sh
git add tools/device-qualification.mjs tools/device-qualification.test.mjs docs/testing/device-qualification tests/fixtures/device-qualification docs/qualification/device-qualification-matrix.md
git commit -m "feat: record immutable qualification bundles"
```

---

### Task 5: Add Clean-Build Qualification Metadata and a Pinned Launcher

**Files:**
- Create: `apps/emuchef-app/src-tauri/src/qualification_build.rs`
- Create: `apps/emuchef-app/scripts/run-device-qualification.mjs`
- Modify: `apps/emuchef-app/src-tauri/build.rs`
- Modify: `apps/emuchef-app/src-tauri/Cargo.toml`
- Modify: `apps/emuchef-app/src-tauri/src/lib.rs`
- Modify: `apps/emuchef-app/package.json`

**Interfaces:**
- Consumes: Node `--build-identity --require-clean` from Task 3.
- Produces: `QualificationBuildIdentity`, `QualificationGateInputs`, `qualification_mode_enabled(inputs)`, compile-time `EMUCHEF_QUALIFICATION_BUILD_IDENTITY`, package script `device-qualification`.

- [ ] **Step 1: Write failing Rust unit tests for all four enablement gates**

In `qualification_build.rs`, define tests around injected gate inputs:

```rust
#[test]
fn qualification_mode_requires_every_gate() {
    let valid = QualificationGateInputs {
        debug_build: true,
        real_execution_enabled: true,
        runtime_opt_in: true,
        embedded_identity: Some(test_identity()),
    };
    assert!(qualification_mode_enabled(&valid));

    for mut invalid in [
        QualificationGateInputs { debug_build: false, ..valid.clone() },
        QualificationGateInputs { real_execution_enabled: false, ..valid.clone() },
        QualificationGateInputs { runtime_opt_in: false, ..valid.clone() },
        QualificationGateInputs { embedded_identity: None, ..valid.clone() },
    ] {
        assert!(!qualification_mode_enabled(&invalid));
        invalid.runtime_opt_in = false;
    }
}
```

Use a concrete helper:

```rust
fn test_identity() -> QualificationBuildIdentity {
    QualificationBuildIdentity {
        app_version: "0.1.0".into(),
        git_commit: "1".repeat(40),
        material_build_digest: format!("sha256:{}", "a".repeat(64)),
        real_execution_enabled: true,
        qualification_contract: 1,
    }
}
```

- [ ] **Step 2: Run the focused Rust test and verify it fails**

Run:

```sh
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode_requires_every_gate
```

Expected: FAIL because `qualification_build.rs` and the types do not exist.

- [ ] **Step 3: Implement build identity parsing and mode-gate evaluation**

Use strict serde names matching the Node JSON:

```rust
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationBuildIdentity {
    pub app_version: String,
    pub git_commit: String,
    pub material_build_digest: String,
    pub real_execution_enabled: bool,
    pub qualification_contract: u32,
}

#[derive(Clone, Debug)]
pub struct QualificationGateInputs {
    pub debug_build: bool,
    pub real_execution_enabled: bool,
    pub runtime_opt_in: bool,
    pub embedded_identity: Option<QualificationBuildIdentity>,
}

pub fn qualification_mode_enabled(inputs: &QualificationGateInputs) -> bool {
    inputs.debug_build
        && inputs.real_execution_enabled
        && inputs.runtime_opt_in
        && inputs.embedded_identity.as_ref().is_some_and(|identity| identity.real_execution_enabled)
}
```

Runtime inputs use `cfg!(debug_assertions)`, `cfg!(feature = "real-execution")`, `std::env::var("EMUCHEF_DEVICE_QUALIFICATION").ok().as_deref() == Some("1")`, and `option_env!("EMUCHEF_QUALIFICATION_BUILD_IDENTITY")`.

- [ ] **Step 4: Make `build.rs` obtain identity only through the canonical Node tool**

Add `serde_json` to `[build-dependencies]` and use this pattern before `tauri_build::build()`:

```rust
println!("cargo:rerun-if-env-changed=EMUCHEF_DEVICE_QUALIFICATION");
if std::env::var("EMUCHEF_DEVICE_QUALIFICATION").ok().as_deref() == Some("1") {
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let tool = manifest_dir.join("../../../tools/device-qualification.mjs");
    let output = std::process::Command::new("node")
        .arg(tool)
        .arg("--build-identity")
        .arg("--require-clean")
        .output()
        .expect("device qualification build identity command must start");
    assert!(output.status.success(), "device qualification requires a clean committed source state");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("device qualification build identity must be valid JSON");
    let encoded = serde_json::to_string(&value).expect("build identity must serialize");
    println!("cargo:rustc-env=EMUCHEF_QUALIFICATION_BUILD_IDENTITY={encoded}");
}
tauri_build::build();
```

No operator-entered build identity/runtime identity variables remain.

- [ ] **Step 5: Add a non-hot-reload launcher**

`run-device-qualification.mjs` must execute these steps with `spawnSync`, failing on the first non-zero status:

```js
run("npm", ["run", "build"]);
run("npm", ["run", "sidecar:dev"]);
run("cargo", [
  "run",
  "--manifest-path",
  "src-tauri/Cargo.toml",
  "--features",
  "real-execution",
], {
  ...process.env,
  EMUCHEF_DEVICE_QUALIFICATION: "1",
});
```

Set `cwd` to `apps/emuchef-app`. Add:

```json
"device-qualification": "node scripts/run-device-qualification.mjs"
```

to `package.json`. Do not use `tauri dev` or Vite hot reload for recordable qualification.

- [ ] **Step 6: Run unit/build checks without enabling physical qualification**

Run:

```sh
cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_build
cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml
cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution
npm --prefix apps/emuchef-app run typecheck
```

Expected: PASS. Do not run `npm --prefix apps/emuchef-app run device-qualification` in this task because the worktree is intentionally uncommitted during implementation.

- [ ] **Step 7: Commit**

```sh
git add apps/emuchef-app/src-tauri/build.rs apps/emuchef-app/src-tauri/Cargo.toml apps/emuchef-app/src-tauri/src/qualification_build.rs apps/emuchef-app/src-tauri/src/lib.rs apps/emuchef-app/scripts/run-device-qualification.mjs apps/emuchef-app/package.json
git commit -m "feat: gate device qualification builds"
```

---

### Task 6: Extend Existing Production Probe and Report Boundaries

**Files:**
- Modify: `crates/emuchef-rust-backend/src/device_probe.rs`
- Modify: backend request/DTO files only where the existing `probeDevice` serialization requires it
- Modify: `apps/emuchef-app/src-tauri/src/commands.rs`
- Modify: `apps/emuchef-app/src-tauri/src/execution.rs`
- Modify: `apps/emuchef-app/src/types.ts`
- Test: existing unit tests in `device_probe.rs`, `commands.rs`, and `execution.rs`

**Interfaces:**
- Consumes: existing production `probeDevice`, `probe_device`, root qualification, and `export_execution_report` boundaries.
- Produces:
  - `DeviceFacts.firmwareBuild: string | null`
  - reusable trusted device-observation helper used by normal command + qualification mode
  - `production_execution_report_bytes(store, execution_handle) -> Result<Vec<u8>, String>` used by normal export + qualification capture.

- [ ] **Step 1: Write a failing backend probe test for Android build fingerprint**

Extend the existing `getprop` parser test with:

```rust
let facts = detected_facts_from_getprop_output(
    "[ro.product.manufacturer]: [AYANEO]\n[ro.build.fingerprint]: [AYANEO/device/build:15/ABC/123:user/release-keys]\n",
    Some("serial".to_string()),
);
assert_eq!(
    facts.firmware_build.as_deref(),
    Some("AYANEO/device/build:15/ABC/123:user/release-keys")
);
```

- [ ] **Step 2: Run the focused backend test and verify it fails**

Run:

```sh
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml device_probe -- --nocapture
```

Expected: FAIL because `DetectedDeviceFacts` has no `firmware_build` field.

- [ ] **Step 3: Extend the existing production probe—not qualification code**

Add:

```rust
pub firmware_build: Option<String>,
```

to `DetectedDeviceFacts`, and parse:

```rust
"ro.build.fingerprint" => {
    facts.firmware_build = present_text(Some(value)).map(ToString::to_string);
}
```

Carry that field through the existing `probeDevice` response and Tauri `public_device_facts` projection. Extend the public TypeScript DTO:

```ts
export interface DeviceFacts {
  deviceHandle: string;
  manufacturer: string | null;
  brand: string | null;
  model: string | null;
  androidVersion: number | null;
  androidApiLevel: number | null;
  firmwareBuild: string | null;
}
```

Do not issue a second qualification-only `adb shell getprop` command.

- [ ] **Step 4: Extract one production report-byte serializer and test equivalence**

Before changing `export_execution_report`, add a test that obtains a terminal execution fixture and asserts the bytes returned by the new internal helper deserialize to the same sanitized JSON written by the existing export path.

Use this interface:

```rust
pub(crate) fn production_execution_report_bytes(
    store: &ExecutionHandleStore,
    execution_handle: &str,
) -> Result<Vec<u8>, String>;
```

The helper must contain the existing report projection/serialization logic. `export_execution_report` continues to own the native save dialog but obtains its bytes only from this helper. Qualification code later calls the same helper directly; it does not reconstruct report JSON.

- [ ] **Step 5: Run backend, Tauri, and frontend type checks**

Run:

```sh
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml device_probe
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml execution
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml commands
npm --prefix apps/emuchef-app run typecheck
```

Expected: PASS.

- [ ] **Step 6: Commit the production-boundary refactor**

```sh
git add crates/emuchef-rust-backend/src/device_probe.rs crates/emuchef-rust-backend/src apps/emuchef-app/src-tauri/src/commands.rs apps/emuchef-app/src-tauri/src/execution.rs apps/emuchef-app/src/types.ts
git commit -m "feat: expose qualification-ready production observations"
```

Before committing, inspect `git diff --name-only` and stage only backend files actually required to carry `firmware_build`; do not stage unrelated backend changes.

---

### Task 7: Add Trusted Candidate Persistence and Bounded Node Invocation

**Files:**
- Create: `apps/emuchef-app/src-tauri/src/qualification_repository.rs`
- Modify: `apps/emuchef-app/src-tauri/src/lib.rs`
- Modify: `apps/emuchef-app/src-tauri/Cargo.toml` only if an already-unused required standard dependency is genuinely needed

**Interfaces:**
- Consumes: build identity from Task 5; Node `--describe`, `--register-target`, and `--record-run` from Tasks 3-4.
- Produces:
  - `QualificationRepository::production()` using the fixed trusted repository root
  - test-only `QualificationRepository::new_for_test(repo_root, runner)`
  - `create_candidate(kind, json, report_bytes) -> candidate_handle`
  - `list_candidates() -> Vec<QualificationCandidateSummary>`
  - `load_candidate(handle) -> StoredQualificationCandidate`
  - `discard_candidate(handle)`
  - `describe() -> RepositoryQualificationDescription`
  - `register_target(handle)` and `record_run(handle)` bounded tool calls.

- [ ] **Step 1: Write failing tests for opaque handles, fixed roots, restart recovery, and path rejection**

Use `tempfile::TempDir` in unit tests and require this handle format:

```rust
const CANDIDATE_HANDLE_PREFIX: &str = "qualification-candidate-";
```

Tests must prove:

```rust
let handle = repository.create_candidate(CandidateKind::TargetRegistration, &json, None).unwrap();
assert!(handle.starts_with(CANDIDATE_HANDLE_PREFIX));
assert!(!handle.contains('/'));
assert!(!handle.contains(".."));

let reopened = QualificationRepository::new_for_test(
    repository.repo_root().to_path_buf(),
    Box::new(FakeQualificationToolRunner::default()),
);
assert_eq!(reopened.list_candidates().unwrap().len(), 1);
assert!(reopened.load_candidate("../../etc/passwd").is_err());
```

- [ ] **Step 2: Run the focused Rust tests and verify they fail**

Run:

```sh
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_repository
```

Expected: FAIL because the repository module does not exist.

- [ ] **Step 3: Implement fixed root derivation and candidate storage**

Production root derives only from the trusted compile-time manifest location:

```rust
pub fn production_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
}
```

Normalize/canonicalize this trusted path internally; never accept a root from React. Candidate storage is exactly:

```text
<repo>/.emuchef_runtime/qualification-candidates/<opaque-handle>/candidate.json
<repo>/.emuchef_runtime/qualification-candidates/<opaque-handle>/execution-report.json   # only when captured
```

Generate handles as:

```rust
format!("qualification-candidate-{}", Uuid::new_v4().simple())
```

Validate every supplied handle with an ASCII prefix + exactly 32 lowercase hex characters before joining it to the candidate root.

- [ ] **Step 4: Add a narrow runner seam and allowlisted tool operations**

Define:

```rust
pub trait QualificationToolRunner: Send + Sync {
    fn run(&self, repo_root: &Path, args: &[String]) -> Result<Vec<u8>, String>;
}
```

Production implementation executes only:

```text
node <fixed repo>/tools/device-qualification.mjs --describe
node <fixed repo>/tools/device-qualification.mjs --register-target <validated opaque handle>
node <fixed repo>/tools/device-qualification.mjs --record-run <validated opaque handle>
```

No method accepts an arbitrary executable, tool path, repository path, candidate path, or additional argument vector from React. Tests inject a fake runner and assert the exact argv.

- [ ] **Step 5: Revalidate candidate bytes during restart/reload**

Store candidate metadata as strict Rust DTOs with `#[serde(deny_unknown_fields)]`, but do not duplicate Node semantic validation. Rust may verify only local integrity/session properties it owns: handle matches directory, JSON parses, report file presence matches locally stored candidate metadata, and build identity can still be compared to the embedded build. Canonical schema/projection validity remains Node-owned.

- [ ] **Step 6: Run tests and commit**

Run:

```sh
cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_repository
cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution
```

Expected: PASS. Then commit:

```sh
git add apps/emuchef-app/src-tauri/src/qualification_repository.rs apps/emuchef-app/src-tauri/src/lib.rs apps/emuchef-app/src-tauri/Cargo.toml
git commit -m "feat: persist qualification candidates safely"
```

---

### Task 8: Add Device-Target Registration Orchestration

**Files:**
- Create: `apps/emuchef-app/src-tauri/src/qualification_mode.rs`
- Modify: `apps/emuchef-app/src-tauri/src/lib.rs`
- Modify: `apps/emuchef-app/src-tauri/src/commands.rs` to expose reusable internal product-observation helpers without changing public authority
- Modify: `apps/emuchef-app/src/types.ts`
- Modify: `apps/emuchef-app/src/api.ts`

**Interfaces:**
- Consumes: production `probe_device`, `match_device`, `get_device_qualification`, `check_device_root`; build/repository layers from Tasks 5/7.
- Produces Tauri commands:
  - `get_device_qualification_mode_status`
  - `create_qualification_target_candidate`
  - `register_qualification_target`
  - `discard_qualification_candidate`

- [ ] **Step 1: Define the frontend DTO contract and write failing Rust serialization tests**

Add these TypeScript interfaces first so names are fixed across Rust/React:

```ts
export type QualificationCheckpointOutcome = "pass" | "fail" | "unable_to_verify";
export type QualificationConnectionType = "usb2" | "usb3";

export interface QualificationBuildIdentity {
  appVersion: string;
  gitCommit: string;
  materialBuildDigest: string;
  realExecutionEnabled: true;
  qualificationContract: number;
}

export interface QualificationWorkflow {
  id: string;
  version: number;
  purpose: string;
  productionRecipes: string[];
  requiredCapabilities: string[];
  prerequisites: string[];
  humanCheckpoints: Array<{
    id: string;
    instruction: string;
    fact: string;
    allowedOutcomes: QualificationCheckpointOutcome[];
    required: boolean;
  }>;
}

export interface QualificationTargetSummary {
  id: string;
  profileId: string;
  manufacturer: string;
  model: string;
  androidVersion: string;
  androidApi: number;
  abiSocClass: string;
  rootState: "non_root" | "rooted";
  connectionType: QualificationConnectionType;
  firmwareBuild: string;
}

export type QualificationFactSource =
  | "production_observation"
  | "explicit_root_check"
  | "operator_attestation";

export interface QualificationFactPreview<T> {
  value: T;
  source: QualificationFactSource;
}

export interface QualificationTargetCandidatePreview {
  candidateHandle: string;
  kind: "target_registration";
  capturedAt: string;
  target: {
    profileId: QualificationFactPreview<string>;
    manufacturer: QualificationFactPreview<string>;
    model: QualificationFactPreview<string>;
    androidVersion: QualificationFactPreview<string>;
    androidApi: QualificationFactPreview<number>;
    abiSocClass: QualificationFactPreview<string>;
    rootState: QualificationFactPreview<"non_root" | "rooted">;
    connectionType: QualificationFactPreview<QualificationConnectionType>;
    firmwareBuild: QualificationFactPreview<string>;
    capabilities: string[];
    deferredWorkflows: string[];
  };
  promotable: boolean;
  nonPromotableReason: string | null;
}

export interface QualificationCandidateSummary {
  candidateHandle: string;
  kind: "target_registration" | "qualification_run";
  capturedAt: string;
  promotable: boolean;
  nonPromotableReason: string | null;
  target?: QualificationTargetCandidatePreview["target"];
  runValidity?: "valid" | "invalid";
  qualificationOutcome?: "passed" | "failed" | "not_observed";
}

export interface QualificationModeStatus {
  enabled: boolean;
  recordable: boolean;
  message: string | null;
  build: QualificationBuildIdentity | null;
  runtimeContract: string | null;
  workflows: QualificationWorkflow[];
  targets: QualificationTargetSummary[];
  resumableCandidates: QualificationCandidateSummary[];
}
```

Rust serialization tests must round-trip the same camelCase field names.

- [ ] **Step 2: Run the focused tests and verify they fail**

Run:

```sh
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
npm --prefix apps/emuchef-app run typecheck
```

Expected: FAIL because the Rust module/API methods do not exist yet.

- [ ] **Step 3: Implement mode status from embedded gates + canonical `--describe`**

`get_device_qualification_mode_status` returns `enabled:false` with empty workflows/targets when the four gates fail and never launches Node in ordinary end-user mode. When enabled, use `QualificationRepository::describe()` and project `runtimeContract` as the separate `QualificationModeStatus.runtimeContract` field; do not fold it into `QualificationBuildIdentity`. For resumable target-registration candidates, include the stored typed target/provenance preview in `QualificationCandidateSummary.target` so an operator can review the exact candidate after restart before registering it.

All mutating commands begin with one shared guard:

```rust
fn require_recordable_mode(state: &QualificationModeState) -> Result<&QualificationBuildIdentity, String> {
    if !state.enabled {
        return Err(safe_qualification_error("qualification_mode_disabled"));
    }
    state.build.as_ref().ok_or_else(|| safe_qualification_error("qualification_build_unavailable"))
}
```

The concrete error helper returns sanitized user-facing JSON/String consistent with existing Tauri command errors; it must not leak host paths or raw process output.

- [ ] **Step 4: Implement target capture from existing production observations**

`create_qualification_target_candidate` accepts only:

```ts
{
  deviceHandle: string;
  devicePlan: string;
  connectionType: "usb2" | "usb3";
}
```

Rust independently resolves/validates:

```text
probeDevice                -> manufacturer, model, Android version/API, firmwareBuild
matchDevice                -> selected devicePlan exists and its profileId
get_device_qualification   -> abiClass, storage, packageManager, current deviceIdentity
check_device_root          -> authoritative root result for the same deviceIdentity
```

Map capabilities exactly:

```rust
if qualification.package_manager == CapabilityAvailabilityDto::Available {
    capabilities.push("apk_install".to_string());
}
if qualification.storage == CapabilityAvailabilityDto::Available {
    capabilities.push("shared_storage_write".to_string());
}
```

Map root exactly:

```text
granted -> rooted
denied -> non_root
unavailable/checkFailed -> reject target candidate as unverified
```

Use `qualification.abiClass` as the initial `abiSocClass` value. Require every identity fact to be present. Build schema-v2 fact wrappers with `production_observation`, root with `explicit_root_check`, and connection with `operator_attestation`. Set `deferredWorkflows: []`.

- [ ] **Step 5: Persist the target candidate and delegate registration to Node**

Rust writes the candidate to the trusted repository store and returns a `QualificationTargetCandidatePreview` containing the exact typed values and provenance that will be submitted to the canonical tool. This is the operator's review surface; React may display it but cannot edit any observed value/source. If the operator wants a different `connectionType`, discard and recapture the candidate rather than mutating stored candidate bytes. `register_qualification_target(candidateHandle)` performs the current-build/source-state guard, then invokes only `--register-target <handle>`.

After successful registration, return the canonical target ID plus an explicit consequence:

```ts
{
  targetId: string;
  requiresCommitAndRebuild: true;
}
```

Do not allow `beginQualificationSession` in that now-dirty checkout/build lifecycle.

- [ ] **Step 6: Add public API wrappers using opaque values only**

Add:

```ts
deviceQualificationModeStatus: () =>
  invoke<QualificationModeStatus>("get_device_qualification_mode_status"),
createQualificationTargetCandidate: (request: {
  deviceHandle: string;
  devicePlan: string;
  connectionType: QualificationConnectionType;
}) => invoke<QualificationTargetCandidatePreview>("create_qualification_target_candidate", { request }),
registerQualificationTarget: (candidateHandle: string) =>
  invoke<{ targetId: string; requiresCommitAndRebuild: true }>("register_qualification_target", { candidateHandle }),
discardQualificationCandidate: (candidateHandle: string) =>
  invoke<void>("discard_qualification_candidate", { candidateHandle }),
```

No path field may appear in these DTOs.

- [ ] **Step 7: Run Rust/API tests and commit**

Run:

```sh
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution
npm --prefix apps/emuchef-app run typecheck
npm --prefix apps/emuchef-app run lint
```

Expected: PASS. Then commit:

```sh
git add apps/emuchef-app/src-tauri/src/qualification_mode.rs apps/emuchef-app/src-tauri/src/lib.rs apps/emuchef-app/src-tauri/src/commands.rs apps/emuchef-app/src/types.ts apps/emuchef-app/src/api.ts
git commit -m "feat: register physical qualification targets"
```

---

### Task 9: Add Qualification Run Sessions, Checkpoints, and Candidate Finalization

**Files:**
- Modify: `apps/emuchef-app/src-tauri/src/qualification_mode.rs`
- Modify: `apps/emuchef-app/src-tauri/src/qualification_repository.rs`
- Modify: `apps/emuchef-app/src-tauri/src/lib.rs`
- Modify: `apps/emuchef-app/src-tauri/src/execution.rs` only for internal accessors required to verify review/execution relationships
- Modify: `apps/emuchef-app/src/types.ts`
- Modify: `apps/emuchef-app/src/api.ts`

**Interfaces:**
- Consumes: registered targets/workflows from canonical `--describe`, production review/execution/report state, candidate repository.
- Produces:
  - `begin_qualification_session`
  - `refresh_qualification_session`
  - `bind_qualification_review`
  - `bind_qualification_execution`
  - `record_qualification_checkpoint`
  - `finalize_qualification_candidate`
  - `record_qualification_run`
  - `QualificationSession::to_persisted() -> PersistedQualificationSession`
  - `QualificationSession::from_persisted(PersistedQualificationSession) -> Result<QualificationSession, String>`
  - `QualificationRepository::save_session(candidate_handle, persisted_session)` and `load_session(candidate_handle)` for restart-safe checkpoint state

- [ ] **Step 1: Write failing state-machine tests for permanent invalidation and typed checkpoints**

Use a pure session core so tests do not require Tauri or hardware:

```rust
#[test]
fn invalidated_session_never_returns_to_valid() {
    let mut session = test_session();
    session.invalidate(QualificationInvalidation::DeviceIdentityChanged);
    assert_eq!(session.run_validity(), RunValidity::Invalid);
    session.observe_matching_device(test_observation());
    assert_eq!(session.run_validity(), RunValidity::Invalid);
}

#[test]
fn checkpoint_ids_and_outcomes_must_come_from_the_workflow_contract() {
    let mut session = checkpoint_session();
    assert!(session.record_checkpoint("device_behavior_verified", CheckpointOutcome::Pass).is_ok());
    assert!(session.record_checkpoint("invented", CheckpointOutcome::Pass).is_err());
    assert!(session.record_checkpoint("device_behavior_verified", CheckpointOutcome::UnableToVerify).is_ok());
    assert_eq!(session.run_validity(), RunValidity::Invalid);
    assert_eq!(session.qualification_outcome(), QualificationOutcome::NotObserved);
}

#[test]
fn recorded_checkpoint_timestamp_survives_persistence_and_reload() {
    let mut session = checkpoint_session();
    session.record_checkpoint_at(
        "device_behavior_verified",
        CheckpointOutcome::Pass,
        "2026-08-23T12:34:56Z",
    ).unwrap();
    let restored = QualificationSession::from_persisted(session.to_persisted()).unwrap();
    assert_eq!(
        restored.recorded_checkpoints()[0].observed_at,
        "2026-08-23T12:34:56Z",
    );
}
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run:

```sh
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode::tests
```

Expected: FAIL because session lifecycle types do not exist.

- [ ] **Step 3: Implement the session state model and exact command DTOs**

Use opaque session handles:

```rust
format!("qualification-session-{}", Uuid::new_v4().simple())
```

`begin_qualification_session` accepts:

```ts
{
  deviceHandle: string;
  devicePlan: string;
  targetId: string;
  workflowId: string;
}
```

Rust must independently verify the current product match maps `devicePlan` to the registered target's `profileId`, and the current observed device facts match every registered material target fact. Return:

```ts
export interface QualificationSessionSnapshot {
  sessionHandle: string;
  targetId: string;
  workflowId: string;
  workflowVersion: number;
  devicePlan: string;
  requiredRecipes: string[];
  humanCheckpoints: QualificationWorkflow["humanCheckpoints"];
  recordedCheckpoints: Array<{
    checkpointId: string;
    outcome: QualificationCheckpointOutcome;
    observedAt: string;
  }>;
  runValidity: "valid" | "invalid";
  qualificationOutcome: "passed" | "failed" | "not_observed";
  invalidReason: string | null;
  candidate: QualificationCandidateSummary | null;
}
```

Starting a session must fail if current HEAD/worktree no longer satisfies the clean embedded-build source binding.

Create the run's opaque candidate directory when the session begins and persist a strict `PersistedQualificationSession` there as session state changes. The persisted form contains only trusted session bindings, permanent invalidation state, bound review/execution handles, and recorded checkpoint records; it is not yet a promotable `candidate.json`. `to_persisted`/`from_persisted` must round-trip those fields exactly. Terminal finalization converts the persisted session into the strict run candidate contract from Task 4.

- [ ] **Step 4: Re-probe identity on refresh and permanently invalidate drift**

`refresh_qualification_session(sessionHandle, deviceHandle)` does not trust React to restate facts. It calls the same production observation helpers used by target registration and compares values to the registered target. At minimum invalidate permanently on:

```text
device unavailable/offline/unauthorized
deviceIdentity changed
manufacturer/model changed
Android API changed
ABI class changed
firmware build changed
root result no longer matches registered root state
```

Return the updated session snapshot; never silently bind a new target.

- [ ] **Step 5: Bind the actual product review and real execution rather than React claims**

`bind_qualification_review(sessionHandle, reviewHandle)` resolves `reviewHandle` from the existing Tauri review store and verifies that its target/device plan and selected recipes correspond to the session target plus exact `workflow.productionRecipes`.

`bind_qualification_execution(sessionHandle, executionHandle)` resolves the existing execution store and requires:

```text
real execution, not simulation
execution.reviewHandle == session.boundReviewHandle
execution target identity == session target
```

No qualification command starts or confirms execution.

- [ ] **Step 6: Implement checkpoint capture and terminal classification**

Use exact enum mappings:

```rust
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointOutcome {
    Pass,
    Fail,
    UnableToVerify,
}
```

Timestamp a checkpoint only when the operator explicitly submits it, and persist the resulting `{ checkpointId, outcome, observedAt }` immediately in the trusted session/candidate state so finalization cannot lose or recreate the observation. On restart, restored candidate/session data must preserve the original checkpoint timestamp and outcome; never timestamp it again during reload/finalization. Missing required checkpoint at finalization -> invalid/not_observed. Required `unable_to_verify` -> invalid/not_observed. Required `fail` with otherwise healthy harness may support valid/failed unless the checkpoint is a declared pre-execution prerequisite.

For `clean_or_deliberately_reset_device`, require an explicit `pass` before `bind_qualification_review` or `bind_qualification_execution` can succeed. `fail` or `unable_to_verify` permanently invalidates the session as not observed, because the production workflow was not run under its required baseline.

For the current required automated observation `execution-report`, classify a bound terminal real execution as:

```text
succeeded | succeeded_with_warnings -> valid + passed
failed                              -> valid + failed
cancelled                           -> invalid + not_observed
nonterminal / missing / unavailable -> invalid + not_observed
```

If a cancelled execution still has a production report, retain it as an artifact on the invalid audit candidate but do not reinterpret operator cancellation as a product failure.

- [ ] **Step 7: Finalize a candidate using the exact production report bytes**

When the session remains valid and the bound execution is terminal, call `production_execution_report_bytes` from Task 6, write those exact bytes into the candidate directory, SHA-256 them, and add the report artifact/automated observation. If report capture fails, permanently invalidate the session and create an invalid/not_observed candidate with `artifacts: []` and without a report file.

The run candidate's fingerprint must use:

```text
embedded QualificationBuildIdentity
workflow version
exact SHA-256 of every workflow.productionRecipes authored file
RUNTIME_CONTRACT
device target fact values
```

Copy the session's persisted human checkpoint records verbatim into `candidate.humanCheckpoints`, including their original `observedAt` timestamps. Do not infer, default, or re-time checkpoint outcomes during finalization.

Do not compute canonical run/fingerprint/record digests in Rust; Node seals them during promotion.

- [ ] **Step 8: Implement explicit run recording and candidate discard**

`record_qualification_run(candidateHandle)` requires a terminal stored candidate, rechecks current build/source binding, and calls only `--record-run <handle>`. Return the canonical run ID from Node. Invalid candidates are allowed if they satisfy the invalid/not_observed contract.

`discard_qualification_candidate` removes only the validated opaque candidate directory under the fixed runtime candidate root.

- [ ] **Step 9: Run orchestration tests and commit**

Run:

```sh
cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_repository
cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution
npm --prefix apps/emuchef-app run typecheck
```

Expected: PASS without a device. Then commit:

```sh
git add apps/emuchef-app/src-tauri/src/qualification_mode.rs apps/emuchef-app/src-tauri/src/qualification_repository.rs apps/emuchef-app/src-tauri/src/lib.rs apps/emuchef-app/src-tauri/src/execution.rs apps/emuchef-app/src/types.ts apps/emuchef-app/src/api.ts
git commit -m "feat: capture qualification run candidates"
```

---

### Task 10: Layer the Qualification Controller Over the Existing React Workflow

**Files:**
- Create: `apps/emuchef-app/src/useDeviceQualificationMode.ts`
- Create: `apps/emuchef-app/src/DeviceQualificationOverlay.tsx`
- Create: `apps/emuchef-app/tests/useDeviceQualificationMode.dom.test.tsx`
- Create: `apps/emuchef-app/tests/DeviceQualificationOverlay.dom.test.tsx`
- Modify: `apps/emuchef-app/src/App.tsx`
- Modify: `apps/emuchef-app/src/styles.css`
- Modify: `apps/emuchef-app/src/api.ts` / `src/types.ts` only if DTO integration requires final adjustments

**Interfaces:**
- Consumes: Task 8-9 Tauri qualification API plus existing `WorkflowState`, existing App recipe/device-plan handlers, existing review/real-execution UI.
- Produces: `useDeviceQualificationMode(...)`, `DeviceQualificationOverlay`, `QualificationIntentLock` consumed by `App`.

- [ ] **Step 1: Write a failing hook test proving normal product state is observed, not replaced**

Create `useDeviceQualificationMode.dom.test.tsx` with a mocked `api` and a small harness. Prove that when the mode is disabled it returns no intent lock and issues no mutation calls:

```tsx
test("disabled qualification mode leaves the normal workflow unconstrained", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue({
    enabled: false,
    recordable: false,
    message: null,
    build: null,
    runtimeContract: null,
    workflows: [],
    targets: [],
    resumableCandidates: [],
  });
  render(<Harness workflow={reviewWorkflow()} />);
  expect(await screen.findByTestId("qualification-active")).toHaveTextContent("false");
  expect(mockApi.beginQualificationSession).not.toHaveBeenCalled();
});
```

Add a second test where an active session exposes exactly `devicePlan` + `requiredRecipes` as the intent lock and does not call `createReview` or `startRealExecution` itself.

- [ ] **Step 2: Run the hook test and verify it fails**

Run:

```sh
npm --prefix apps/emuchef-app exec -- vitest run --config tests/vitest.config.ts tests/useDeviceQualificationMode.dom.test.tsx
```

Expected: FAIL because the hook does not exist.

- [ ] **Step 3: Implement the hook as an observer/controller around `WorkflowState`**

Use this public return shape:

```ts
export interface QualificationIntentLock {
  devicePlan: string;
  selectedRecipes: string[];
}

export interface DeviceQualificationModeController {
  status: QualificationModeStatus | null;
  session: QualificationSessionSnapshot | null;
  targetCandidate: QualificationTargetCandidatePreview | null;
  intentLock: QualificationIntentLock | null;
  busy: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  beginSession: (request: {
    deviceHandle: string;
    devicePlan: string;
    targetId: string;
    workflowId: string;
  }) => Promise<void>;
  createTargetCandidate: (connectionType: QualificationConnectionType) => Promise<void>;
  registerTarget: (candidateHandle: string) => Promise<void>;
  recordCheckpoint: (checkpointId: string, outcome: QualificationCheckpointOutcome) => Promise<void>;
  recordRun: (candidateHandle: string) => Promise<void>;
  discardCandidate: (candidateHandle: string) => Promise<void>;
}
```

`createTargetCandidate` stores the returned `QualificationTargetCandidatePreview` in `targetCandidate`. On status refresh/restart, if a resumable `target_registration` summary carries `target`, reconstruct the same read-only preview from that stored data; never re-probe and silently replace the candidate being reviewed.

Effects observe normal `workflow.review` and `workflow.execution` transitions and call bind/finalize commands only after those production states exist. Deduplicate effects with refs keyed by `reviewHandle`, `executionHandle`, and terminal execution identity so StrictMode cannot double-bind or double-finalize.

- [ ] **Step 4: Write failing overlay tests for explicit operator actions and checkpoint defaults**

In `DeviceQualificationOverlay.dom.test.tsx`, prove:

```tsx
test("declared checkpoints have no default outcome", () => {
  render(<DeviceQualificationOverlay controller={checkpointController()} />);
  expect(screen.getByRole("radio", { name: "Pass" })).not.toBeChecked();
  expect(screen.getByRole("radio", { name: "Fail" })).not.toBeChecked();
  expect(screen.getByRole("radio", { name: "Unable to verify" })).not.toBeChecked();
});

test("recording a run always requires an explicit click", () => {
  const controller = terminalController();
  render(<DeviceQualificationOverlay controller={controller} />);
  expect(controller.recordRun).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Record qualification run" }));
  expect(controller.recordRun).toHaveBeenCalledTimes(1);
});
```

Also test that invalid/not-observed is labeled “Invalid qualification run — not product evidence” and valid/failed is labeled as a product qualification failure. Add a restart-review test proving a resumable target-registration candidate renders the stored value/source provenance, and a session test proving already-recorded checkpoints render their persisted outcome rather than defaulting or being re-timestamped.

- [ ] **Step 5: Implement the persistent overlay without a parallel wizard**

`DeviceQualificationOverlay` renders a development banner/controller above or beside the existing App content. It may show:

```text
mode/build status
registered target + canonical workflow selectors
target-registration capture/review controls
declared checkpoint controls
candidate terminal classification
Register device target / Record qualification run / Discard candidate actions
```

It must not render its own device discovery, configuration, review, confirmation, execution progress, or report screen. Those remain existing App components.

- [ ] **Step 6: Integrate intent locking into `App.tsx`**

Mount the hook inside normal `App`, after the existing startup flow. Do not replace `main.tsx` with another qualification-only route.

When `intentLock` is present:

```ts
const qualificationLocksIntent = qualification.intentLock !== null;
```

Guard every existing device-plan or recipe-selection mutation so the operator cannot diverge from the bound target/workflow during that session. Apply the session's exact `devicePlan` and `selectedRecipes` once at session start through the same workflow reducer/actions the normal UI uses; after that, render those controls disabled with a qualification explanation.

Do not synthesize bindings or clear validation errors. The existing input/configuration/review path remains unchanged.

- [ ] **Step 7: Prove review and real confirmation are still mandatory**

Extend App/overlay DOM tests so activating qualification mode does not call `api.createReview` until the existing Review action is used and never calls `api.startRealExecution` itself. Existing real-execution confirmation tests must continue to pass unchanged.

Run:

```sh
npm --prefix apps/emuchef-app exec -- vitest run --config tests/vitest.config.ts tests/useDeviceQualificationMode.dom.test.tsx tests/DeviceQualificationOverlay.dom.test.tsx tests/useExecution.dom.test.tsx
npm --prefix apps/emuchef-app run typecheck
npm --prefix apps/emuchef-app run lint
```

Expected: PASS.

- [ ] **Step 8: Commit the frontend overlay**

```sh
git add apps/emuchef-app/src/useDeviceQualificationMode.ts apps/emuchef-app/src/DeviceQualificationOverlay.tsx apps/emuchef-app/tests/useDeviceQualificationMode.dom.test.tsx apps/emuchef-app/tests/DeviceQualificationOverlay.dom.test.tsx apps/emuchef-app/src/App.tsx apps/emuchef-app/src/styles.css apps/emuchef-app/src/api.ts apps/emuchef-app/src/types.ts
git commit -m "feat: add device qualification overlay"
```

---

### Task 11: Finish Security Guards, Operator Documentation, Roadmap Truth, and Full Validation

**Files:**
- Modify: `apps/emuchef-app/tests/security-policy.test.mjs`
- Modify: `tools/device-qualification.test.mjs`
- Modify: `docs/manual/device-qualification-operator.md`
- Modify: `docs/qualification/device-qualification-matrix.md` only through generator
- Modify: `docs/product/product-roadmap.md`
- Modify: `Makefile`
- Modify: `.github/workflows/emuchef-execution-feature-matrix.yml`
- Modify: active docs/scripts that still refer to old qualification paths or manual build/runtime env identities

**Interfaces:**
- Consumes: complete Node/Tauri/React harness from Tasks 1-10.
- Produces: repository-level guardrails, truthful operator workflow, CI/local validation, no physical support claim.

- [ ] **Step 1: Add failing security/name-regression tests**

Extend `security-policy.test.mjs` and `tools/device-qualification.test.mjs` to assert the React API contains no qualification filesystem/process fields:

```js
const apiSource = readFileSync(path.join(APP_ROOT, "src/api.ts"), "utf8");
for (const forbidden of [
  "candidatePath",
  "repositoryPath",
  "evidencePath",
  "toolPath",
  "executablePath",
]) {
  assert.equal(apiSource.includes(forbidden), false, forbidden);
}
```

Add an active-path scan that permits historical names only beneath `docs/superpowers/`, `docs/product/product-roadmap.md`, and `.chatgpt/`, while rejecting new phase/slice names in `tools/device-qualification*`, `docs/testing/device-qualification/**`, `tests/fixtures/device-qualification/**`, and new qualification source files.

- [ ] **Step 2: Run the focused guards and verify any stale active references fail**

Run:

```sh
node --test tools/device-qualification.test.mjs
npm --prefix apps/emuchef-app run test:security
```

Expected before cleanup: any remaining active old-path/manual-env reference is surfaced explicitly.

- [ ] **Step 3: Rewrite the operator runbook for the implemented harness**

`docs/manual/device-qualification-operator.md` must describe this exact operator flow:

```text
1. Launch a clean qualification build with `npm --prefix apps/emuchef-app run device-qualification`.
2. If the device is unregistered: connect/probe/match it, choose usb2/usb3, review the captured facts, Register device target, stop, commit the registry/matrix, and rebuild.
3. On the new clean build: choose the registered target and canonical workflow.
4. Complete normal EmuChef inputs, review, and explicit real-execution confirmation.
5. Complete only workflow-declared human checkpoints.
6. Inspect terminal candidate classification.
7. Explicitly Record qualification run, including invalid/not-observed audit runs only when intentionally preserving harness history.
8. Stop and commit the resulting immutable evidence bundle and matrix before another recordable promotion from a fresh build.
9. Run `make device-qualification-check` and repository tests before committing/shipping evidence.
```

Remove obsolete `EMUCHEF_PHASE_6F_BUILD_IDENTITY` / `EMUCHEF_PHASE_6F_RUNTIME_CONTRACT` instructions.

- [ ] **Step 4: Update roadmap state without claiming physical qualification**

Update the relevant roadmap entry to say the production-bound qualification harness is implemented/available for future physical runs, while preserving these truths:

```text
no physical device targets/evidence are added by harness implementation itself
no workflow/device is qualified merely because the harness exists
physical device matrix work remains in progress until real evidence is intentionally recorded
Daijisho and ES-DE remain deferred
```

Do not mark the physical matrix complete.

- [ ] **Step 5: Regenerate and verify the canonical matrix**

Run:

```sh
node tools/device-qualification.mjs --write-matrix
node tools/device-qualification.mjs --check
```

Expected: matrix still truthfully reports no registered physical targets unless the owner separately recorded one outside this implementation plan.

- [ ] **Step 6: Run the full automated validation battery**

Run:

```sh
node --test tools/device-qualification.test.mjs
make device-qualification-check
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --check
cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --all-targets
cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --all-targets --features real-execution
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution
npm --prefix apps/emuchef-app run test
npm --prefix apps/emuchef-app run test:security
npm --prefix apps/emuchef-app run typecheck
npm --prefix apps/emuchef-app run lint
npm --prefix apps/emuchef-app run build
make test
```

Expected: all commands PASS without connected hardware and without adding production physical evidence.

- [ ] **Step 7: Inspect final naming and authority boundaries**

Run repository searches and verify manually:

```sh
git grep -nE 'phase[-_]?6f|Phase6f|PHASE_6F' -- tools apps/emuchef-app/src apps/emuchef-app/src-tauri/src tests/fixtures/device-qualification docs/testing/device-qualification docs/manual/device-qualification-operator.md docs/qualification/device-qualification-matrix.md Makefile .github/workflows/emuchef-execution-feature-matrix.yml
git grep -nE 'candidatePath|repositoryPath|evidencePath|toolPath|executablePath' -- apps/emuchef-app/src
```

Expected: no new active device-qualification implementation identifier uses project-management nomenclature; no React API exposes filesystem/process authority. Legacy unrelated qualification tooling outside this task is not renamed unless directly referenced by the new harness.

- [ ] **Step 8: Commit final integration/docs**

```sh
git add apps/emuchef-app/tests/security-policy.test.mjs tools/device-qualification.test.mjs docs/manual/device-qualification-operator.md docs/qualification/device-qualification-matrix.md docs/product/product-roadmap.md Makefile .github/workflows/emuchef-execution-feature-matrix.yml
git commit -m "docs: finalize device qualification workflow"
```

---

## Final Acceptance Checklist

- [ ] Active qualification foundation uses domain-oriented paths and names.
- [ ] Device targets/evidence use schema version 2 with legal fact provenance.
- [ ] Target ID and run ID are canonical Node-generated digest identities.
- [ ] Build identity contains exact Git audit provenance plus material build-content digest; Git-only/evidence-only commits do not stale evidence.
- [ ] Canonical Node tooling owns all repository schema/digest/projection/mutation rules.
- [ ] Valid runs require the exact digest-bound production report; invalid report-capture failures may be recorded without that artifact only as invalid/not-observed.
- [ ] Tauri qualification mode is gated by debug + real-execution + clean embedded build identity + explicit environment opt-in.
- [ ] New target registration uses production probe/match/qualification/root authority and only typed USB connection attestation.
- [ ] Target registration requires commit/rebuild before workflow qualification.
- [ ] Qualification sessions bind one registered target + one canonical workflow and permanently invalidate identity drift.
- [ ] Existing review and explicit real-execution confirmation remain mandatory.
- [ ] Human checkpoints come only from workflow declarations, persisted outcomes retain their original timestamps, and no checkpoint has an implicit pass.
- [ ] The RetroArch+BIOS clean/reset prerequisite is a required pre-execution checkpoint; only an explicit pass allows review/execution binding.
- [ ] Candidate state survives restart under `.emuchef_runtime/qualification-candidates/` but never affects support projection.
- [ ] React receives only opaque handles/sanitized DTOs and never receives filesystem/process authority.
- [ ] Invalid runs can be retained as audit history but can never become current qualification evidence.
- [ ] Matrix generation remains deterministic and recording is rollback-safe from the caller's perspective.
- [ ] Full automated validation passes with no connected hardware.
- [ ] No physical device/workflow support claim is introduced by implementation alone.
