import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  EVIDENCE_RECORD_FIELDS,
  FINGERPRINT_FIELDS,
  RUN_VALIDITIES,
  QUALIFICATION_OUTCOMES,
  QUALIFICATION_CONTRACT_VERSION,
  RUNTIME_CONTRACT,
  CHECKPOINT_OUTCOMES,
  TARGET_WIDE_FAILURES,
  buildCurrentFingerprint,
  canonicalDigest,
  canonicalize,
  classifyCompatibility,
  compareBuildIdentity,
  deviceTargetId,
  deriveApplicability,
  deriveDeviceSupportTier,
  deriveWorkflowState,
  evidenceFingerprintDigest,
  evidenceRecordDigest,
  buildMaterialIdentity,
  loadEvidenceDirectory,
  loadEvidenceBundle,
  loadDeviceTargets,
  loadWorkflowCatalog,
  materialBuildDigestFromEntries,
  projectQualificationState,
  recordQualificationRunCandidate,
  registerQualificationTargetCandidate,
  renderQualificationMatrix,
  sealEvidenceRecord,
  selectCurrentEvidence,
  validateEvidenceBundle,
  validateEvidenceRecord,
  validateEvidenceSchemaContract,
  validateDeviceTargets,
  validateWorkflowCatalog,
} from "./device-qualification.mjs";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const FIXTURES = path.join(REPO_ROOT, "tests/fixtures/device-qualification");
const AUTHORED_PROFILES = path.join(REPO_ROOT, "authored/device_profiles");
const SYNTHETIC_POCKET_S2_PROFILE = "ayaneo.pocket_s2";
const SYNTHETIC_AIR_MINI_PROFILE = "ayaneo.pocket_air_mini";

function readJson(relative) {
  return JSON.parse(readFileSync(path.join(FIXTURES, relative), "utf8"));
}

function syntheticContext() {
  return {
    workflowCatalog: loadWorkflowCatalog(path.join(FIXTURES, "definitions-valid/workflow-catalog.json")),
    targets: loadDeviceTargets(
      path.join(FIXTURES, "definitions-valid/device-targets.json"),
      { authoredProfilesDir: AUTHORED_PROFILES },
    ).targets,
  };
}

function recordFor(relative) {
  const resolved = path.join(FIXTURES, relative);
  const evidencePath = statSync(resolved).isDirectory()
    ? path.join(resolved, "evidence.json")
    : resolved;
  return JSON.parse(readFileSync(evidencePath, "utf8"));
}

function sealRecord(record) {
  return sealEvidenceRecord(record);
}

function checkpointGatedRecord() {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  record.runId = `qualification-run-sha256:${"7".repeat(64)}`;
  record.capturedAt = "2026-08-21T02:00:00Z";
  record.workflowId = "checkpoint-gated";
  record.workflowVersion = 1;
  record.artifacts = [
    {
      id: "execution-report",
      kind: "production_execution_report",
      path: "execution-report.json",
      sha256: createHash("sha256").update(REPORT_BYTES).digest("hex"),
    },
  ];
  record.fingerprint.authoredContent = [];
  record.fingerprint.workflowVersion = 1;
  record.fingerprintDigest = evidenceFingerprintDigest(record.fingerprint);
  record.automatedObservations = [
    { id: "execution-report", outcome: "passed", observedAt: "2026-08-21T02:01:00Z" },
  ];
  record.humanCheckpoints = [
    { checkpointId: "device_behavior_verified", outcome: "fail", observedAt: "2026-08-21T02:02:00Z" },
  ];
  record.qualificationOutcome = "failed";
  record.limitations = ["device_behavior_verified failed"];
  return sealRecord(record);
}

const CURRENT_BUILD = {
  appVersion: "0.1.0",
  gitCommit: "1".repeat(40),
  materialBuildDigest: `sha256:${"a".repeat(64)}`,
  realExecutionEnabled: true,
  qualificationContract: QUALIFICATION_CONTRACT_VERSION,
};
const AUTHORED_DIGESTS = {
  "app.retroarch.provision": "a".repeat(64),
  "feature.copy_bios": "b".repeat(64),
  "app.obtainium.install": "c".repeat(64),
  "app.xaniteog.install": "d".repeat(64),
  "feature.copy_roms": "e".repeat(64),
};

function projectionContext() {
  const workflowCatalog = loadWorkflowCatalog(path.join(FIXTURES, "definitions-valid/workflow-catalog.json"));
  const targets = loadDeviceTargets(
    path.join(FIXTURES, "projection/device-targets.json"),
    { authoredProfilesDir: AUTHORED_PROFILES },
  ).targets;
  const records = readdirSync(path.join(FIXTURES, "projection/evidence"), { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .sort((left, right) => left.name.localeCompare(right.name))
    .map((entry) => {
      const bundle = loadEvidenceBundle(path.join(FIXTURES, "projection/evidence", entry.name));
      validateEvidenceBundle(bundle, { workflowCatalog, targets });
      return bundle.record;
    });
  return {
    workflowCatalog,
    targets,
    records,
  };
}

function currentFingerprint(workflow, target) {
  return buildCurrentFingerprint({
    workflow,
    target,
    currentBuild: CURRENT_BUILD,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests: AUTHORED_DIGESTS,
  });
}

function targetByProfile(targets, profileId) {
  return targets.find((target) => target.profileId.value === profileId);
}

function projectionState(context, targetId, workflowId) {
  const workflow = context.workflowCatalog.workflows.find((item) => item.id === workflowId);
  const target = context.targets.find((item) => item.id === targetId);
  return deriveWorkflowState({
    workflow,
    target,
    currentFingerprint: currentFingerprint(workflow, target),
    records: context.records,
  });
}

const REPORT_BYTES = Buffer.from('{"schemaVersion":1,"status":"succeeded"}\n', "utf8");

function copyTrackedFile(repoRoot, relativePath) {
  const destination = path.join(repoRoot, relativePath);
  mkdirSync(path.dirname(destination), { recursive: true });
  cpSync(path.join(REPO_ROOT, relativePath), destination);
}

function createTempQualificationRepo({ deviceTargetsSource } = {}) {
  const repoRoot = mkdtempSync(path.join(tmpdir(), "device-qualification-"));
  mkdirSync(path.join(repoRoot, "docs/testing/device-qualification/evidence"), { recursive: true });
  mkdirSync(path.join(repoRoot, "docs/qualification"), { recursive: true });
  mkdirSync(path.join(repoRoot, "authored/recipes"), { recursive: true });
  mkdirSync(path.join(repoRoot, "authored/device_profiles"), { recursive: true });
  mkdirSync(path.join(repoRoot, "apps/emuchef-app/src-tauri"), { recursive: true });

  copyTrackedFile(repoRoot, "apps/emuchef-app/package.json");
  copyTrackedFile(repoRoot, "apps/emuchef-app/package-lock.json");
  copyTrackedFile(repoRoot, "apps/emuchef-app/src-tauri/Cargo.toml");
  copyTrackedFile(repoRoot, "apps/emuchef-app/src-tauri/Cargo.lock");
  copyTrackedFile(repoRoot, "apps/emuchef-app/src-tauri/tauri.conf.json");
  copyTrackedFile(repoRoot, "docs/testing/device-qualification/evidence-schema.json");
  copyTrackedFile(repoRoot, "docs/testing/device-qualification/workflow-catalog.json");
  copyTrackedFile(repoRoot, "docs/testing/device-qualification/evidence/README.md");
  copyTrackedFile(repoRoot, "docs/qualification/device-qualification-matrix.md");
  cpSync(path.join(REPO_ROOT, "authored/device_profiles"), path.join(repoRoot, "authored/device_profiles"), { recursive: true });

  const workflowCatalog = JSON.parse(readFileSync(
    path.join(REPO_ROOT, "docs/testing/device-qualification/workflow-catalog.json"),
    "utf8",
  ));
  const recipeIds = [...new Set(workflowCatalog.workflows.flatMap((workflow) => workflow.productionRecipes))];
  for (const recipeId of recipeIds) {
    copyTrackedFile(repoRoot, `authored/recipes/${recipeId}.yaml`);
  }

  const targetsSource = deviceTargetsSource
    ?? path.join(REPO_ROOT, "docs/testing/device-qualification/device-targets.json");
  copyTrackedFile(repoRoot, path.relative(REPO_ROOT, targetsSource));
  if (targetsSource !== path.join(REPO_ROOT, "docs/testing/device-qualification/device-targets.json")) {
    cpSync(
      targetsSource,
      path.join(repoRoot, "docs/testing/device-qualification/device-targets.json"),
    );
  }

  execFileSync("git", ["init"], { cwd: repoRoot, stdio: "pipe" });
  execFileSync("git", ["config", "user.name", "Codex"], { cwd: repoRoot, stdio: "pipe" });
  execFileSync("git", ["config", "user.email", "codex@example.com"], { cwd: repoRoot, stdio: "pipe" });
  execFileSync("git", ["add", "."], { cwd: repoRoot, stdio: "pipe" });
  execFileSync("git", ["commit", "-m", "initial qualification fixture"], { cwd: repoRoot, stdio: "pipe" });
  return repoRoot;
}

function commitRepoChanges(repoRoot, message) {
  execFileSync("git", ["add", "."], { cwd: repoRoot, stdio: "pipe" });
  execFileSync("git", ["commit", "-m", message], { cwd: repoRoot, stdio: "pipe" });
}

function replaceDeviceTargets(repoRoot, targets, commitMessage = "narrow device targets") {
  writeFileSync(
    path.join(repoRoot, "docs/testing/device-qualification/device-targets.json"),
    `${JSON.stringify({ schemaVersion: 2, targets }, null, 2)}\n`,
    "utf8",
  );
  commitRepoChanges(repoRoot, commitMessage);
}

function authoredDigestsForRepo(repoRoot, workflowCatalog) {
  return Object.fromEntries(
    workflowCatalog.workflows
      .flatMap((workflow) => workflow.productionRecipes)
      .filter((value, index, all) => all.indexOf(value) === index)
      .map((recipeId) => {
        const bytes = readFileSync(path.join(repoRoot, "authored/recipes", `${recipeId}.yaml`));
        return [recipeId, createHash("sha256").update(bytes).digest("hex")];
      }),
  );
}

function targetRegistrationCandidateForRepo(repoRoot, {
  candidateId = "qualification-candidate-0123456789abcdef0123456789abcdef",
  targetIndex = 0,
} = {}) {
  const build = buildMaterialIdentity({ repoRoot, requireClean: false });
  const target = structuredClone(readJson("definitions-valid/device-targets.json").targets[targetIndex]);
  delete target.id;
  return {
    candidateSchemaVersion: 1,
    candidateId,
    kind: "target_registration",
    capturedAt: "2026-08-23T12:00:00Z",
    build,
    target,
  };
}

function runCandidateForRepo(repoRoot, {
  candidateId = "qualification-candidate-fedcba9876543210fedcba9876543210",
} = {}) {
  const workflowCatalog = loadWorkflowCatalog(path.join(repoRoot, "docs/testing/device-qualification/workflow-catalog.json"));
  const targets = loadDeviceTargets(
    path.join(repoRoot, "docs/testing/device-qualification/device-targets.json"),
    { authoredProfilesDir: path.join(repoRoot, "authored/device_profiles") },
  ).targets;
  const workflow = workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  const target = targets[0];
  const build = buildMaterialIdentity({ repoRoot, requireClean: false });
  const fingerprint = buildCurrentFingerprint({
    workflow,
    target,
    currentBuild: build,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests: authoredDigestsForRepo(repoRoot, workflowCatalog),
  });
  return {
    candidateSchemaVersion: 1,
    candidateId,
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
      {
        id: "execution-report",
        kind: "production_execution_report",
        path: "execution-report.json",
        sha256: createHash("sha256").update(REPORT_BYTES).digest("hex"),
      },
    ],
  };
}

function writeCandidateFixture(repoRoot, candidate, reportBytes = null) {
  const directory = path.join(repoRoot, ".emuchef_runtime/qualification-candidates", candidate.candidateId);
  mkdirSync(directory, { recursive: true });
  const report = reportBytes === null
    ? null
    : {
      path: "execution-report.json",
      byteLength: reportBytes.length,
      sha256: createHash("sha256").update(reportBytes).digest("hex"),
    };
  writeFileSync(path.join(directory, "candidate.json"), `${JSON.stringify({
    candidateHandle: candidate.candidateId,
    kind: candidate.kind,
    capturedAt: candidate.capturedAt ?? null,
    build: candidate.build ?? null,
    payload: candidate,
    report,
  }, null, 2)}\n`, "utf8");
  if (reportBytes !== null) {
    writeFileSync(path.join(directory, "execution-report.json"), reportBytes);
  }
}

function installEvidenceBundle(repoRoot, fixtureRelative, { reportBytes } = {}) {
  const source = path.join(FIXTURES, fixtureRelative);
  const destination = path.join(
    repoRoot,
    "docs/testing/device-qualification/evidence",
    path.basename(source),
  );
  cpSync(source, destination, { recursive: true });
  if (reportBytes !== undefined) {
    writeFileSync(path.join(destination, "execution-report.json"), reportBytes);
  }
  return destination;
}

function snapshotTree(root) {
  if (!existsSync(root)) {
    return [];
  }
  const entries = [];
  const walk = (current, relative) => {
    for (const entry of readdirSync(current, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name))) {
      const nextRelative = relative ? path.join(relative, entry.name) : entry.name;
      const fullPath = path.join(current, entry.name);
      if (entry.isDirectory()) {
        entries.push({ type: "dir", path: nextRelative });
        walk(fullPath, nextRelative);
      } else {
        entries.push({
          type: "file",
          path: nextRelative,
          content: readFileSync(fullPath, "utf8"),
        });
      }
    }
  };
  walk(root, "");
  return entries;
}

test("loads a strict version-1 workflow catalog from fixtures", () => {
  const catalog = loadWorkflowCatalog(path.join(FIXTURES, "definitions-valid/workflow-catalog.json"));
  assert.equal(catalog.schemaVersion, 1);
  assert.equal(catalog.workflows.length, 5);
  assert.equal(catalog.workflows[0].id, "retroarch-plus-bios");
});

test("loads version-2 device targets and verifies authored profile files", () => {
  const targets = loadDeviceTargets(
    path.join(FIXTURES, "definitions-valid/device-targets.json"),
    { authoredProfilesDir: AUTHORED_PROFILES },
  );
  assert.equal(targets.schemaVersion, 2);
  assert.deepEqual(
    targets.targets.map((target) => target.id),
    targets.targets.map((target) => deviceTargetId(target)),
  );
});

test("schema-v2 device targets require legal per-fact provenance and deterministic ids", () => {
  const observed = (value) => ({ value, source: "production_observation" });
  const rooted = (value) => ({ value, source: "explicit_root_check" });
  const attested = (value) => ({ value, source: "operator_attestation" });
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

test("schema-v2 device targets reject malformed fact wrappers through validation errors", () => {
  const valid = structuredClone(readJson("definitions-valid/device-targets.json"));
  const malformed = valid.targets[0];
  malformed.rootState = null;
  assert.throws(
    () => validateDeviceTargets(valid, { authoredProfilesDir: AUTHORED_PROFILES }),
    /device target rootState/i,
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

test("schema-v2 evidence records require qualification run ids", () => {
  const context = syntheticContext();
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  assert.match(record.runId, /^qualification-run-sha256:[0-9a-f]{64}$/);

  const oldPrefix = structuredClone(record);
  oldPrefix.runId = `phase-6f-run-sha256:${"1".repeat(64)}`;
  oldPrefix.recordDigest = evidenceRecordDigest(oldPrefix);
  assert.throws(
    () => validateEvidenceRecord(oldPrefix, context),
    /runId format/i,
  );
});

test("rejects duplicate workflow ids", () => {
  assert.throws(
    () => loadWorkflowCatalog(path.join(FIXTURES, "definitions-invalid/duplicate-workflow-id.json")),
    /duplicate workflow id/,
  );
});

test("rejects device targets whose authored profile does not exist", () => {
  assert.throws(
    () => loadDeviceTargets(
      path.join(FIXTURES, "definitions-invalid/unknown-profile.json"),
      { authoredProfilesDir: AUTHORED_PROFILES },
    ),
    /unknown authored device profile/,
  );
});

test("rejects unknown, missing, and mistyped workflow catalog fields", () => {
  const valid = readJson("definitions-valid/workflow-catalog.json");
  assert.throws(() => validateWorkflowCatalog({ ...valid, extra: true }), /fields must be exactly/i);
  assert.throws(() => validateWorkflowCatalog({ ...valid, workflows: [] }), /workflow/i);
  assert.throws(() => validateWorkflowCatalog({ ...valid, schemaVersion: 2 }), /schemaVersion/i);

  const workflow = valid.workflows[0];
  const badPurpose = {
    ...valid,
    workflows: [{ ...workflow, purpose: "" }],
  };
  assert.throws(() => validateWorkflowCatalog(badPurpose), /purpose/i);
  const badVersion = {
    ...valid,
    workflows: [{ ...workflow, version: 0 }],
  };
  assert.throws(() => validateWorkflowCatalog(badVersion), /version/i);
});

test("rejects unknown, missing, and mistyped device target fields", () => {
  const valid = readJson("definitions-valid/device-targets.json");
  const target = valid.targets[0];
  assert.throws(() => validateDeviceTargets({ ...valid, extra: true }, { authoredProfilesDir: AUTHORED_PROFILES }), /fields must be exactly/i);
  assert.throws(
    () => validateDeviceTargets({
      ...valid,
      targets: [{
        ...target,
        androidApi: { ...target.androidApi, value: "35" },
        id: deviceTargetId({
          ...target,
          androidApi: { ...target.androidApi, value: "35" },
        }),
      }],
    }, { authoredProfilesDir: AUTHORED_PROFILES }),
    /androidApi/,
  );
  assert.throws(
    () => validateDeviceTargets({
      ...valid,
      targets: [{
        ...target,
        rootState: { ...target.rootState, value: "unknown" },
        id: deviceTargetId({
          ...target,
          rootState: { ...target.rootState, value: "unknown" },
        }),
      }],
    }, { authoredProfilesDir: AUTHORED_PROFILES }),
    /rootState/,
  );
  assert.throws(
    () => validateDeviceTargets({
      ...valid,
      targets: [{
        ...target,
        connectionType: { ...target.connectionType, value: "wifi" },
        id: deviceTargetId({
          ...target,
          connectionType: { ...target.connectionType, value: "wifi" },
        }),
      }],
    }, { authoredProfilesDir: AUTHORED_PROFILES }),
    /connectionType/,
  );
  assert.throws(
    () => validateDeviceTargets({ ...valid, schemaVersion: 1 }, { authoredProfilesDir: AUTHORED_PROFILES }),
    /schemaVersion/i,
  );
});

test("canonicalization sorts object keys recursively and keeps array order", () => {
  const canonical = canonicalize({ b: 1, a: [2, { d: 4, c: 3 }] });
  assert.deepEqual(canonical, { a: [2, { c: 3, d: 4 }], b: 1 });
  const first = canonicalDigest({ b: 1, a: 2 });
  const second = canonicalDigest({ a: 2, b: 1 });
  assert.equal(first, second);
  assert.match(first, /^sha256:[0-9a-f]{64}$/);
});

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

test("material identity refreshes for changed inputs and rejects dirty state", () => {
  const repoRoot = createTempQualificationRepo();
  const toolDestination = path.join(repoRoot, "tools/device-qualification.mjs");
  const packagePath = path.join(repoRoot, "apps/emuchef-app/package.json");

  try {
    mkdirSync(path.dirname(toolDestination), { recursive: true });
    cpSync(path.join(REPO_ROOT, "tools/device-qualification.mjs"), toolDestination);
    const toolPath = realpathSync(toolDestination);
    execFileSync("git", ["add", "tools/device-qualification.mjs"], { cwd: repoRoot, stdio: "pipe" });
    execFileSync("git", ["commit", "-m", "add canonical identity tool"], { cwd: repoRoot, stdio: "pipe" });

    const runIdentity = (requireClean) => spawnSync(
      process.execPath,
      [toolPath, "--build-identity", ...(requireClean ? ["--require-clean"] : [])],
      { cwd: repoRoot, encoding: "utf8" },
    );

    const cleanResult = runIdentity(true);
    assert.equal(cleanResult.status, 0, cleanResult.stderr);
    const cleanIdentity = JSON.parse(cleanResult.stdout);

    const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));
    packageJson.version = "0.1.1";
    writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`, "utf8");

    const refreshedResult = runIdentity(false);
    assert.equal(refreshedResult.status, 0, refreshedResult.stderr);
    const refreshedIdentity = JSON.parse(refreshedResult.stdout);
    assert.notEqual(refreshedIdentity.materialBuildDigest, cleanIdentity.materialBuildDigest);

    const dirtyResult = runIdentity(true);
    assert.notEqual(dirtyResult.status, 0);
    assert.match(`${dirtyResult.stdout}${dirtyResult.stderr}`, /clean tracked worktree/);
  } finally {
    rmSync(repoRoot, { recursive: true, force: true });
  }
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

test("production catalog loads and every production recipe exists", () => {
  const catalog = loadWorkflowCatalog(path.join(REPO_ROOT, "docs/testing/device-qualification/workflow-catalog.json"));
  assert.equal(catalog.workflows.length, 4);
  assert.deepEqual(
    catalog.workflows.map((workflow) => workflow.id),
    ["retroarch-plus-bios", "obtainium-install", "xaniteog-install", "rom-library-sync"],
  );
  for (const workflow of catalog.workflows) {
    for (const recipeId of workflow.productionRecipes) {
      const recipePath = path.join(REPO_ROOT, "authored/recipes", `${recipeId}.yaml`);
      assert.equal(existsSync(recipePath), true, `${recipeId} must exist in authored/recipes`);
      assert.equal(
        readFileSync(recipePath, "utf8").includes(`id: ${recipeId}`),
        true,
        `${recipeId} must exist in authored/recipes`,
      );
    }
  }
});

test("production catalog binds retroarch prerequisite review to a required human checkpoint", () => {
  const catalog = loadWorkflowCatalog(path.join(REPO_ROOT, "docs/testing/device-qualification/workflow-catalog.json"));
  const retroarch = catalog.workflows.find((workflow) => workflow.id === "retroarch-plus-bios");
  assert.equal(retroarch.version, 2);
  assert.deepEqual(retroarch.prerequisites, ["clean_or_deliberately_reset_device"]);
  assert.deepEqual(retroarch.humanCheckpoints, [
    {
      id: "clean_or_deliberately_reset_device",
      instruction: "Before execution, verify the connected device is clean or has been deliberately reset to the intended qualification baseline.",
      fact: "The device was clean or deliberately reset before this qualification run.",
      allowedOutcomes: ["pass", "fail", "unable_to_verify"],
      required: true,
    },
  ]);
});

test("production device registry starts with no targets", () => {
  const targets = loadDeviceTargets(
    path.join(REPO_ROOT, "docs/testing/device-qualification/device-targets.json"),
    { authoredProfilesDir: AUTHORED_PROFILES },
  );
  assert.equal(targets.schemaVersion, 2);
  assert.deepEqual(targets.targets, []);
});

test("the evidence schema matches the validator contract", () => {
  const schema = JSON.parse(readFileSync(
    path.join(REPO_ROOT, "docs/testing/device-qualification/evidence-schema.json"),
    "utf8",
  ));
  assert.deepEqual([...schema.required].sort(), [...EVIDENCE_RECORD_FIELDS].sort());
  assert.deepEqual([...schema.properties.runValidity.enum].sort(), [...RUN_VALIDITIES].sort());
  assert.deepEqual(
    [...schema.properties.qualificationOutcome.enum].sort(),
    [...QUALIFICATION_OUTCOMES].sort(),
  );
  assert.deepEqual(
    [...schema.properties.targetWideFailure.oneOf[1].enum].sort(),
    [...TARGET_WIDE_FAILURES.filter((value) => value !== null)].sort(),
  );
  assert.deepEqual([...schema.$defs.fingerprint.required].sort(), [...FINGERPRINT_FIELDS].sort());
  assert.deepEqual(
    [...schema.$defs.humanCheckpoint.properties.outcome.enum].sort(),
    [...CHECKPOINT_OUTCOMES].sort(),
  );
  assert.doesNotThrow(() => validateEvidenceSchemaContract(schema));
});

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

test("a retroarch run cannot validate as valid when its required human checkpoint is missing", () => {
  const bundle = loadEvidenceBundle(path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"));
  const unsealed = structuredClone(bundle.record);
  delete unsealed.runId;
  delete unsealed.recordDigest;
  delete unsealed.fingerprintDigest;
  unsealed.humanCheckpoints = [];
  const resealed = sealEvidenceRecord(unsealed);
  assert.throws(
    () => validateEvidenceBundle({ ...bundle, record: resealed }, syntheticContext()),
    /missing required human checkpoint/i,
  );
});

test("recording the same target or run twice rejects the second write without mutating canonical bytes", () => {
  const emptyRepo = createTempQualificationRepo();
  try {
    const targetCandidate = targetRegistrationCandidateForRepo(emptyRepo);
    writeCandidateFixture(emptyRepo, targetCandidate);
    const targetPaths = {
      repoRoot: emptyRepo,
      fsOps: undefined,
    };
    registerQualificationTargetCandidate(targetCandidate.candidateId, targetPaths);
    const registeredBytes = readFileSync(
      path.join(emptyRepo, "docs/testing/device-qualification/device-targets.json"),
      "utf8",
    );
    assert.throws(
      () => registerQualificationTargetCandidate(targetCandidate.candidateId, targetPaths),
      /already exists|immutable|duplicate/i,
    );
    assert.equal(
      readFileSync(path.join(emptyRepo, "docs/testing/device-qualification/device-targets.json"), "utf8"),
      registeredBytes,
    );
  } finally {
    rmSync(emptyRepo, { recursive: true, force: true });
  }

  const seededRepo = createTempQualificationRepo({
    deviceTargetsSource: path.join(FIXTURES, "definitions-valid/device-targets.json"),
  });
  try {
    const runCandidate = runCandidateForRepo(seededRepo);
    writeCandidateFixture(seededRepo, runCandidate, REPORT_BYTES);
    const recordPaths = {
      repoRoot: seededRepo,
      fsOps: undefined,
    };
    const firstRecord = recordQualificationRunCandidate(runCandidate.candidateId, recordPaths);
    const evidenceRoot = path.join(seededRepo, "docs/testing/device-qualification/evidence");
    const before = snapshotTree(evidenceRoot);
    assert.ok(firstRecord.runId);
    assert.throws(
      () => recordQualificationRunCandidate(runCandidate.candidateId, recordPaths),
      /already exists|immutable|duplicate/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), before);
  } finally {
    rmSync(seededRepo, { recursive: true, force: true });
  }
});

test("recording a run refuses a destination reserved by another invocation without deleting it", () => {
  const repoRoot = createTempQualificationRepo({
    deviceTargetsSource: path.join(FIXTURES, "definitions-valid/device-targets.json"),
  });
  try {
    const runCandidate = runCandidateForRepo(repoRoot, {
      candidateId: "qualification-candidate-11112222333344445555666677778888",
    });
    writeCandidateFixture(repoRoot, runCandidate, REPORT_BYTES);
    const evidenceRoot = path.join(repoRoot, "docs/testing/device-qualification/evidence");
    const matrixPath = path.join(repoRoot, "docs/qualification/device-qualification-matrix.md");
    const beforeMatrix = readFileSync(matrixPath, "utf8");
    const raceMarker = "reservation-owner.txt";
    let competitorCreated = false;
    const isCanonicalBundlePath = (targetPath) => (
      targetPath.startsWith(`${evidenceRoot}${path.sep}`)
      && path.basename(targetPath).startsWith("qualification-run-sha256:")
      && !path.basename(targetPath).includes(".tmp-")
    );
    const createCompetingBundle = (targetPath) => {
      if (competitorCreated) return;
      competitorCreated = true;
      mkdirSync(targetPath, { recursive: false });
      writeFileSync(path.join(targetPath, raceMarker), "competing invocation\n", "utf8");
      const error = new Error("competing invocation reserved destination first");
      error.code = "EEXIST";
      throw error;
    };
    assert.throws(
      () => recordQualificationRunCandidate(runCandidate.candidateId, {
        repoRoot,
        fsOps: {
          mkdirSync(targetPath, options) {
            if (isCanonicalBundlePath(targetPath)) {
              createCompetingBundle(targetPath);
            }
            mkdirSync(targetPath, options);
          },
          renameSync(source, destination) {
            if (isCanonicalBundlePath(destination)) {
              createCompetingBundle(destination);
            }
            rmSync(destination, { recursive: true, force: true });
            cpSync(source, destination, { recursive: true });
            rmSync(source, { recursive: true, force: true });
          },
          rmSync,
          writeFileSync,
        },
      }),
      /reserved destination first|already exists|immutable/i,
    );
    const after = snapshotTree(evidenceRoot);
    assert.ok(after.some((entry) => entry.path.endsWith(`/${raceMarker}`)));
    assert.equal(readFileSync(matrixPath, "utf8"), beforeMatrix);
  } finally {
    rmSync(repoRoot, { recursive: true, force: true });
  }
});

test("tampered existing evidence bundle report blocks both promotions before canonical mutation", () => {
  const narrowedTargets = [readJson("definitions-valid/device-targets.json").targets[0]];
  const targetRepo = createTempQualificationRepo();
  try {
    replaceDeviceTargets(targetRepo, narrowedTargets, "narrow target repo");
    installEvidenceBundle(targetRepo, "evidence-valid/passing-retroarch-bios", {
      reportBytes: Buffer.from("{}\n", "utf8"),
    });
    const targetCandidate = targetRegistrationCandidateForRepo(targetRepo, {
      candidateId: "qualification-candidate-9999aaaabbbbccccddddeeeeffff0000",
      targetIndex: 1,
    });
    writeCandidateFixture(targetRepo, targetCandidate);
    const targetRegistryPath = path.join(targetRepo, "docs/testing/device-qualification/device-targets.json");
    const targetMatrixPath = path.join(targetRepo, "docs/qualification/device-qualification-matrix.md");
    const beforeRegistry = readFileSync(targetRegistryPath, "utf8");
    const beforeMatrix = readFileSync(targetMatrixPath, "utf8");
    assert.throws(
      () => registerQualificationTargetCandidate(targetCandidate.candidateId, { repoRoot: targetRepo }),
      /execution report digest/i,
    );
    assert.equal(readFileSync(targetRegistryPath, "utf8"), beforeRegistry);
    assert.equal(readFileSync(targetMatrixPath, "utf8"), beforeMatrix);
  } finally {
    rmSync(targetRepo, { recursive: true, force: true });
  }

  const runRepo = createTempQualificationRepo();
  try {
    replaceDeviceTargets(runRepo, narrowedTargets, "narrow run repo");
    installEvidenceBundle(runRepo, "evidence-valid/passing-retroarch-bios", {
      reportBytes: Buffer.from("{}\n", "utf8"),
    });
    const runCandidate = runCandidateForRepo(runRepo, {
      candidateId: "qualification-candidate-0000ffffeeeeddddccccbbbbaaaa9999",
    });
    writeCandidateFixture(runRepo, runCandidate, REPORT_BYTES);
    const evidenceRoot = path.join(runRepo, "docs/testing/device-qualification/evidence");
    const matrixPath = path.join(runRepo, "docs/qualification/device-qualification-matrix.md");
    const beforeEvidence = snapshotTree(evidenceRoot);
    const beforeMatrix = readFileSync(matrixPath, "utf8");
    assert.throws(
      () => recordQualificationRunCandidate(runCandidate.candidateId, { repoRoot: runRepo }),
      /execution report digest/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), beforeEvidence);
    assert.equal(readFileSync(matrixPath, "utf8"), beforeMatrix);
  } finally {
    rmSync(runRepo, { recursive: true, force: true });
  }
});

test("target registration restores registry and matrix bytes when matrix replacement fails", () => {
  const repoRoot = createTempQualificationRepo();
  try {
    const targetCandidate = targetRegistrationCandidateForRepo(repoRoot, {
      candidateId: "qualification-candidate-12344321123443211234432112344321",
    });
    writeCandidateFixture(repoRoot, targetCandidate);
    const registryPath = path.join(repoRoot, "docs/testing/device-qualification/device-targets.json");
    const matrixPath = path.join(repoRoot, "docs/qualification/device-qualification-matrix.md");
    const beforeRegistry = readFileSync(registryPath, "utf8");
    const beforeMatrix = readFileSync(matrixPath, "utf8");
    let renameCount = 0;
    assert.throws(
      () => registerQualificationTargetCandidate(targetCandidate.candidateId, {
        repoRoot,
        fsOps: {
          mkdirSync,
          renameSync(source, destination) {
            renameCount += 1;
            if (renameCount === 2) {
              throw new Error("forced matrix replacement failure");
            }
            rmSync(destination, { recursive: true, force: true });
            cpSync(source, destination, { recursive: true });
            rmSync(source, { recursive: true, force: true });
          },
          rmSync,
          writeFileSync,
        },
      }),
      /forced matrix replacement failure/i,
    );
    assert.equal(readFileSync(registryPath, "utf8"), beforeRegistry);
    assert.equal(readFileSync(matrixPath, "utf8"), beforeMatrix);
  } finally {
    rmSync(repoRoot, { recursive: true, force: true });
  }
});

test("recording a run rolls back the newly created evidence bundle when matrix replacement fails", () => {
  const repoRoot = createTempQualificationRepo({
    deviceTargetsSource: path.join(FIXTURES, "definitions-valid/device-targets.json"),
  });
  try {
    const runCandidate = runCandidateForRepo(repoRoot, {
      candidateId: "qualification-candidate-00112233445566778899aabbccddeeff",
    });
    writeCandidateFixture(repoRoot, runCandidate, REPORT_BYTES);
    const evidenceRoot = path.join(repoRoot, "docs/testing/device-qualification/evidence");
    const registryPath = path.join(repoRoot, "docs/testing/device-qualification/device-targets.json");
    const beforeEvidence = snapshotTree(evidenceRoot);
    const beforeRegistry = readFileSync(registryPath, "utf8");
    let renameCount = 0;
    assert.throws(
      () => recordQualificationRunCandidate(runCandidate.candidateId, {
        repoRoot,
        fsOps: {
          mkdirSync,
          renameSync(source, destination) {
            renameCount += 1;
            if (renameCount === 1) {
              throw new Error("forced matrix replacement failure");
            }
            rmSync(destination, { recursive: true, force: true });
            cpSync(source, destination, { recursive: true });
            rmSync(source, { recursive: true, force: true });
          },
          rmSync,
          writeFileSync,
        },
      }),
      /forced matrix replacement failure/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), beforeEvidence);
    assert.equal(readFileSync(registryPath, "utf8"), beforeRegistry);
  } finally {
    rmSync(repoRoot, { recursive: true, force: true });
  }
});

test("canonical promotion rejects a symlinked candidate root before reading it", () => {
  const repoRoot = createTempQualificationRepo();
  const outside = mkdtempSync(path.join(tmpdir(), "device-qualification-outside-"));
  try {
    rmSync(path.join(repoRoot, ".emuchef_runtime"), { recursive: true, force: true });
    symlinkSync(outside, path.join(repoRoot, ".emuchef_runtime"));
    const candidate = targetRegistrationCandidateForRepo(repoRoot);
    assert.throws(
      () => registerQualificationTargetCandidate(candidate.candidateId, { repoRoot }),
      /symlink|symbolic|candidate root/i,
    );
    assert.deepEqual(readdirSync(outside), []);
  } finally {
    rmSync(repoRoot, { recursive: true, force: true });
    rmSync(outside, { recursive: true, force: true });
  }
});

test("canonical promotion rejects a symlinked candidate file before mutation", () => {
  const repoRoot = createTempQualificationRepo();
  const outside = mkdtempSync(path.join(tmpdir(), "device-qualification-outside-"));
  try {
    const candidate = targetRegistrationCandidateForRepo(repoRoot, {
      candidateId: "qualification-candidate-12344321123443211234432112344321",
    });
    writeCandidateFixture(repoRoot, candidate);
    const candidatePath = path.join(
      repoRoot,
      ".emuchef_runtime/qualification-candidates",
      candidate.candidateId,
      "candidate.json",
    );
    const outsidePath = path.join(outside, "candidate.json");
    cpSync(candidatePath, outsidePath);
    rmSync(candidatePath);
    symlinkSync(outsidePath, candidatePath);
    const registryPath = path.join(repoRoot, "docs/testing/device-qualification/device-targets.json");
    const before = readFileSync(registryPath, "utf8");
    assert.throws(
      () => registerQualificationTargetCandidate(candidate.candidateId, { repoRoot }),
      /symlink|symbolic|regular|candidate/i,
    );
    assert.equal(readFileSync(registryPath, "utf8"), before);
    assert.ok(existsSync(outsidePath));
  } finally {
    rmSync(repoRoot, { recursive: true, force: true });
    rmSync(outside, { recursive: true, force: true });
  }
});

test("canonical run promotion rejects a symlinked execution report before mutation", () => {
  const repoRoot = createTempQualificationRepo({
    deviceTargetsSource: path.join(FIXTURES, "definitions-valid/device-targets.json"),
  });
  const outside = mkdtempSync(path.join(tmpdir(), "device-qualification-outside-"));
  try {
    const candidate = runCandidateForRepo(repoRoot, {
      candidateId: "qualification-candidate-fedcba9876543210fedcba9876543210",
    });
    writeCandidateFixture(repoRoot, candidate, REPORT_BYTES);
    const reportPath = path.join(
      repoRoot,
      ".emuchef_runtime/qualification-candidates",
      candidate.candidateId,
      "execution-report.json",
    );
    const outsidePath = path.join(outside, "execution-report.json");
    cpSync(reportPath, outsidePath);
    rmSync(reportPath);
    symlinkSync(outsidePath, reportPath);
    const evidenceRoot = path.join(repoRoot, "docs/testing/device-qualification/evidence");
    const before = snapshotTree(evidenceRoot);
    assert.throws(
      () => recordQualificationRunCandidate(candidate.candidateId, { repoRoot }),
      /symlink|symbolic|regular|report/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), before);
  } finally {
    rmSync(repoRoot, { recursive: true, force: true });
    rmSync(outside, { recursive: true, force: true });
  }
});

test("validateEvidenceSchemaContract rejects qualification build identity schema drift", () => {
  const schema = JSON.parse(readFileSync(
    path.join(REPO_ROOT, "docs/testing/device-qualification/evidence-schema.json"),
    "utf8",
  ));

  const missingRequired = structuredClone(schema);
  missingRequired.$defs.qualificationBuildIdentity.required = [
    "appVersion",
    "gitCommit",
    "materialBuildDigest",
    "realExecutionEnabled",
  ];
  assert.throws(
    () => validateEvidenceSchemaContract(missingRequired),
    /qualificationBuildIdentity/i,
  );

  const nonStrict = structuredClone(schema);
  nonStrict.$defs.qualificationBuildIdentity.additionalProperties = true;
  assert.throws(
    () => validateEvidenceSchemaContract(nonStrict),
    /qualificationBuildIdentity/i,
  );

  const driftedConstraint = structuredClone(schema);
  driftedConstraint.$defs.qualificationBuildIdentity.properties.gitCommit.pattern = "^[0-9a-f]{7}$";
  assert.throws(
    () => validateEvidenceSchemaContract(driftedConstraint),
    /qualificationBuildIdentity/i,
  );
});

test("a passing evidence fixture validates with its stored digests", () => {
  assert.doesNotThrow(() => validateEvidenceRecord(
    recordFor("evidence-valid/passing-retroarch-bios"),
    syntheticContext(),
  ));
});

test("a valid failed evidence fixture validates", () => {
  assert.doesNotThrow(() => validateEvidenceRecord(
    recordFor("evidence-valid/failed-retroarch-bios"),
    syntheticContext(),
  ));
});

test("rejects a fingerprint digest that does not match structured inputs", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  record.fingerprintDigest = `sha256:${"0".repeat(64)}`;
  assert.throws(
    () => validateEvidenceRecord(record, syntheticContext()),
    /fingerprintDigest does not match canonical fingerprint/,
  );
});

test("rejects a missing required human-checkpoint result as invalid evidence", () => {
  const record = recordFor("evidence-invalid/missing-required-checkpoint");
  assert.throws(
    () => validateEvidenceRecord(record, syntheticContext()),
    /missing required human checkpoint/,
  );
});

test("invalid infrastructure runs cannot claim a product qualification failure", () => {
  const record = recordFor("evidence-invalid/impossible-run-result");
  assert.throws(
    () => validateEvidenceRecord(record, syntheticContext()),
    /invalid run must use qualificationOutcome "not_observed"/,
  );
});

test("an invalid infrastructure run with not_observed remains valid historical evidence", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  record.runId = `qualification-run-sha256:${"6".repeat(64)}`;
  record.capturedAt = "2026-08-21T01:00:00Z";
  record.runValidity = "invalid";
  record.qualificationOutcome = "not_observed";
  record.automatedObservations = [];
  record.humanCheckpoints = [];
  record.limitations = ["harness failed before product observation"];
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(record), syntheticContext()));
});

test("valid runs forbid not_observed and invalid runs forbid passed and failed", () => {
  const context = syntheticContext();
  const validNotObserved = recordFor("evidence-valid/passing-retroarch-bios");
  validNotObserved.qualificationOutcome = "not_observed";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(validNotObserved), context),
    /valid run must use qualificationOutcome/,
  );
  for (const outcome of ["passed", "failed"]) {
    const record = recordFor("evidence-valid/passing-retroarch-bios");
    record.runValidity = "invalid";
    record.qualificationOutcome = outcome;
    record.automatedObservations = [];
    record.humanCheckpoints = [];
    record.limitations = ["harness failure"];
    assert.throws(
      () => validateEvidenceRecord(sealRecord(record), context),
      /invalid run must use qualificationOutcome "not_observed"/,
    );
  }
});

test("a passed record requires every required automated observation", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  record.automatedObservations = [];
  assert.throws(
    () => validateEvidenceRecord(sealRecord(record), syntheticContext()),
    /missing required automated observation/,
  );
});

test("a failed required automated observation produces a valid failed record", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  record.runId = `qualification-run-sha256:${"8".repeat(64)}`;
  record.capturedAt = "2026-08-21T03:00:00Z";
  record.automatedObservations[0].outcome = "failed";
  record.qualificationOutcome = "failed";
  record.limitations = ["execution-report failed"];
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(record), syntheticContext()));
});

test("a failed required human checkpoint produces a valid failed record", () => {
  assert.doesNotThrow(() => validateEvidenceRecord(checkpointGatedRecord(), syntheticContext()));
});

test("a required checkpoint unable_to_verify must be an invalid not_observed record", () => {
  const context = syntheticContext();
  const record = checkpointGatedRecord();
  record.runValidity = "invalid";
  record.qualificationOutcome = "not_observed";
  record.automatedObservations = [];
  record.humanCheckpoints[0].outcome = "unable_to_verify";
  record.limitations = ["operator could not verify the required device behavior"];
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(record), context));

  const validVariant = checkpointGatedRecord();
  validVariant.humanCheckpoints[0].outcome = "unable_to_verify";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(validVariant), context),
    /required checkpoint unable_to_verify requires runValidity "invalid" and qualificationOutcome "not_observed"/,
  );
});

test("an optional checkpoint unable_to_verify does not block a passed record", () => {
  const context = syntheticContext();
  const catalog = structuredClone(context.workflowCatalog);
  catalog.workflows.find((workflow) => workflow.id === "checkpoint-gated")
    .humanCheckpoints[0].required = false;
  const record = checkpointGatedRecord();
  record.humanCheckpoints[0].outcome = "unable_to_verify";
  record.qualificationOutcome = "passed";
  record.limitations = [];
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(record), {
    workflowCatalog: catalog,
    targets: context.targets,
  }));
});

test("target-wide failures are restricted, required to be failed, and forbidden on invalid runs", () => {
  const context = syntheticContext();
  const failed = recordFor("evidence-valid/failed-retroarch-bios");
  failed.targetWideFailure = "safety_invariant_failed";
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(failed), context));

  const unknown = recordFor("evidence-valid/failed-retroarch-bios");
  unknown.targetWideFailure = "arbitrary_failure";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(unknown), context),
    /targetWideFailure/,
  );

  const passed = recordFor("evidence-valid/passing-retroarch-bios");
  passed.targetWideFailure = "safety_invariant_failed";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(passed), context),
    /targetWideFailure|failed/,
  );
});

test("a valid failed record must contain a failed observation, failed checkpoint, or target-wide failure", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  record.runId = `qualification-run-sha256:${"9".repeat(64)}`;
  record.capturedAt = "2026-08-21T04:00:00Z";
  record.qualificationOutcome = "failed";
  record.limitations = ["product behavior was observed as failed"];
  assert.throws(
    () => validateEvidenceRecord(sealRecord(record), syntheticContext()),
    /failed observation, failed checkpoint, or target-wide failure/,
  );
});

test("evidence must bind to a registered target, workflow, and workflow version", () => {
  const context = syntheticContext();
  const wrongTarget = recordFor("evidence-valid/passing-retroarch-bios");
  wrongTarget.deviceTarget.id = targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id;
  assert.throws(
    () => validateEvidenceRecord(sealRecord(wrongTarget), context),
    /unknown device target|registered target/,
  );

  const wrongWorkflow = recordFor("evidence-valid/passing-retroarch-bios");
  wrongWorkflow.workflowId = "does-not-exist";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(wrongWorkflow), context),
    /unknown workflow/,
  );

  const wrongVersion = recordFor("evidence-valid/passing-retroarch-bios");
  wrongVersion.workflowVersion = 3;
  wrongVersion.fingerprint.workflowVersion = 3;
  wrongVersion.fingerprintDigest = evidenceFingerprintDigest(wrongVersion.fingerprint);
  assert.throws(
    () => validateEvidenceRecord(sealRecord(wrongVersion), context),
    /workflow version/,
  );
});

test("fingerprint validation is strict, deterministic, and digest-bound", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  assert.equal(evidenceFingerprintDigest(record.fingerprint), record.fingerprintDigest);
  const extra = structuredClone(record.fingerprint);
  extra.extra = true;
  assert.throws(() => evidenceFingerprintDigest(extra), /fields must be exactly/);
  const tampered = structuredClone(record.fingerprint);
  tampered.emuchefBuild = { ...tampered.emuchefBuild, materialBuildDigest: `sha256:${"0".repeat(64)}` };
  assert.notEqual(evidenceFingerprintDigest(tampered), record.fingerprintDigest);
});

test("compatibility classifies identical evidence as compatible", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  const workflow = syntheticContext().workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  assert.equal(classifyCompatibility({
    workflow,
    currentFingerprint: record.fingerprint,
    evidenceFingerprint: record.fingerprint,
  }), "compatible");
});

test("every declared compatibility dimension invalidates on change", () => {
  const context = syntheticContext();
  const workflow = context.workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  const base = recordFor("evidence-valid/passing-retroarch-bios").fingerprint;
  const cases = [
    ["emuchefBuild", {
      ...base.emuchefBuild,
      materialBuildDigest: `sha256:${"f".repeat(64)}`,
    }],
    ["workflowVersion", 3],
    ["runtimeContract", "v2"],
    ["deviceProfile", "ayaneo.pocket_air_mini"],
    ["androidApi", 34],
    ["firmwareBuild", "other/build"],
    ["abiSocClass", "arm64-mtk"],
    ["rootState", "rooted"],
  ];
  for (const [field, value] of cases) {
    const changed = structuredClone(base);
    changed[field] = value;
    assert.equal(classifyCompatibility({
      workflow,
      currentFingerprint: base,
      evidenceFingerprint: changed,
    }), "invalidating", `${field} must invalidate`);
  }
  const authored = structuredClone(base);
  authored.authoredContent = [
    { id: "app.retroarch.provision", sha256: "f".repeat(64) },
    { id: "feature.copy_bios", sha256: "b".repeat(64) },
  ];
  assert.equal(classifyCompatibility({
    workflow,
    currentFingerprint: base,
    evidenceFingerprint: authored,
  }), "invalidating", "authored_content must invalidate");
});

test("workflow compatibility treats git-sha-only build changes as compatible", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  const workflow = syntheticContext().workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  const currentFingerprint = record.fingerprint;
  const movedCommit = structuredClone(record.fingerprint);
  movedCommit.emuchefBuild.gitCommit = "f".repeat(40);
  assert.equal(classifyCompatibility({
    workflow,
    currentFingerprint,
    evidenceFingerprint: movedCommit,
  }), "compatible");
});

test("compatibility ignores dimensions the workflow does not declare", () => {
  const context = syntheticContext();
  const workflow = context.workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  const base = recordFor("evidence-valid/passing-retroarch-bios").fingerprint;
  const changed = structuredClone(base);
  changed.connectionType = "usb2";
  assert.equal(classifyCompatibility({
    workflow,
    currentFingerprint: base,
    evidenceFingerprint: changed,
  }), "compatible", "connectionType is not a declared compatibility dimension");

  const narrowWorkflow = structuredClone(workflow);
  narrowWorkflow.compatibilityDimensions = ["workflow_version"];
  const buildChanged = structuredClone(base);
  buildChanged.emuchefBuild = {
    ...buildChanged.emuchefBuild,
    materialBuildDigest: `sha256:${"0".repeat(64)}`,
  };
  assert.equal(classifyCompatibility({
    workflow: narrowWorkflow,
    currentFingerprint: base,
    evidenceFingerprint: buildChanged,
  }), "compatible", "undeclared build change must not invalidate");
});

test("applicability derives from production intent, capabilities, and deferral", () => {
  const context = syntheticContext();
  const retroarch = context.workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  const obtainium = context.workflowCatalog.workflows.find((item) => item.id === "obtainium-install");
  const xaniteog = context.workflowCatalog.workflows.find((item) => item.id === "xaniteog-install");
  const checkpointGated = context.workflowCatalog.workflows.find((item) => item.id === "checkpoint-gated");
  const pocketS2 = targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE);
  const airMini = targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE);

  assert.deepEqual(deriveApplicability(retroarch, pocketS2), {
    state: "required",
    reason: "production_intent_and_capabilities",
  });
  assert.deepEqual(deriveApplicability(retroarch, airMini), {
    state: "not_applicable",
    reason: "missing_capabilities:apk_install",
  });
  assert.deepEqual(deriveApplicability(obtainium, airMini), {
    state: "not_applicable",
    reason: "missing_capabilities:apk_install",
  });
  assert.deepEqual(deriveApplicability(xaniteog, airMini), {
    state: "deferred",
    reason: "explicitly_deferred",
  });
  assert.deepEqual(deriveApplicability(checkpointGated, pocketS2), {
    state: "not_applicable",
    reason: "missing_capabilities:fixture_checkpoint_capability",
  });
});

test("current evidence selection picks the newest compatible valid record", () => {
  const context = projectionContext();
  const workflow = context.workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  const target = targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE);
  const selected = selectCurrentEvidence({
    workflow,
    target,
    currentFingerprint: currentFingerprint(workflow, target),
    records: context.records,
  });
  assert.equal(
    selected.runId,
    context.records.find((record) => record.workflowId === "retroarch-plus-bios" && record.runValidity === "valid").runId,
  );
});

test("newer invalid evidence never replaces older valid evidence", () => {
  const context = projectionContext();
  const workflow = context.workflowCatalog.workflows.find((item) => item.id === "xaniteog-install");
  const target = targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE);
  const selected = selectCurrentEvidence({
    workflow,
    target,
    currentFingerprint: currentFingerprint(workflow, target),
    records: context.records,
  });
  assert.equal(
    selected.runId,
    context.records.find((record) => record.workflowId === "xaniteog-install" && record.runValidity === "valid").runId,
  );
  assert.ok(context.records.some((record) => record.runValidity === "invalid"));
});

test("incompatible historical evidence is never selected as current", () => {
  const context = projectionContext();
  const workflow = context.workflowCatalog.workflows.find((item) => item.id === "obtainium-install");
  const target = targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE);
  const selected = selectCurrentEvidence({
    workflow,
    target,
    currentFingerprint: currentFingerprint(workflow, target),
    records: context.records,
  });
  assert.equal(selected, null);
});

test("current evidence selection is deterministic on capturedAt then runId", () => {
  const context = syntheticContext();
  const workflow = context.workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  const target = targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE);
  const fingerprint = currentFingerprint(workflow, target);
  const base = recordFor("evidence-valid/passing-retroarch-bios");
  const first = structuredClone(base);
  first.runId = "qualification-run-sha256:" + "f".repeat(64);
  const second = structuredClone(base);
  second.runId = "qualification-run-sha256:" + "e".repeat(64);
  const records = [first, second];
  const selected = selectCurrentEvidence({ workflow, target, currentFingerprint: fingerprint, records });
  assert.equal(selected.runId, "qualification-run-sha256:" + "f".repeat(64));
  const reversed = selectCurrentEvidence({ workflow, target, currentFingerprint: fingerprint, records: [second, first] });
  assert.equal(reversed.runId, selected.runId);
});

test("workflow state derivation covers all six states", () => {
  const context = projectionContext();
  assert.equal(projectionState(context, targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id, "retroarch-plus-bios").state, "qualified");
  assert.equal(projectionState(context, targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id, "rom-library-sync").state, "failed");
  assert.equal(projectionState(context, targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id, "obtainium-install").state, "stale");
  assert.equal(projectionState(context, targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id, "xaniteog-install").state, "deferred");
  assert.equal(projectionState(context, targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id, "rom-library-sync").state, "missing");
  assert.equal(projectionState(context, targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id, "retroarch-plus-bios").state, "not_applicable");
});

test("a failed workflow does not erase unrelated qualified evidence", () => {
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

test("device support tier requires every required workflow to be qualified", () => {
  const qualified = deriveDeviceSupportTier([
    { workflowId: "retroarch-plus-bios", applicability: "required", state: "qualified", targetWideFailure: null },
    { workflowId: "rom-library-sync", applicability: "required", state: "qualified", targetWideFailure: null },
  ]);
  assert.equal(qualified, "qualified");

  const noneQualified = deriveDeviceSupportTier([
    { workflowId: "retroarch-plus-bios", applicability: "required", state: "failed", targetWideFailure: null },
    { workflowId: "rom-library-sync", applicability: "required", state: "missing", targetWideFailure: null },
  ]);
  assert.equal(noneQualified, "unqualified");

  const noRequired = deriveDeviceSupportTier([
    { workflowId: "retroarch-plus-bios", applicability: "deferred", state: "deferred", targetWideFailure: null },
  ]);
  assert.equal(noRequired, "unqualified");
});

test("projected fixture state matches the synthetic qualification history", () => {
  const context = projectionContext();
  const pocketRows = ["retroarch-plus-bios", "obtainium-install", "xaniteog-install", "rom-library-sync"]
    .map((workflowId) => projectionState(context, targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id, workflowId));
  assert.deepEqual(
    pocketRows.map((row) => [row.workflowId, row.state]),
    [
      ["retroarch-plus-bios", "qualified"],
      ["obtainium-install", "stale"],
      ["xaniteog-install", "qualified"],
      ["rom-library-sync", "failed"],
    ],
  );
  assert.equal(deriveDeviceSupportTier(pocketRows), "limited");

  const airRows = ["retroarch-plus-bios", "obtainium-install", "xaniteog-install", "rom-library-sync"]
    .map((workflowId) => projectionState(context, targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id, workflowId));
  assert.equal(deriveDeviceSupportTier(airRows), "unqualified");
});

test("historical evidence is not mutated by projection", () => {
  const context = projectionContext();
  const before = context.records.map((record) => JSON.stringify(record));
  for (const workflow of context.workflowCatalog.workflows) {
    for (const target of context.targets) {
      deriveWorkflowState({
        workflow,
        target,
        currentFingerprint: currentFingerprint(workflow, target),
        records: context.records,
      });
    }
  }
  assert.deepEqual(context.records.map((record) => JSON.stringify(record)), before);
});

test("production evidence loading rejects synthetic fixture paths", () => {
  assert.throws(
    () => loadEvidenceDirectory(
      path.join(FIXTURES, "projection/evidence"),
      { fixtureMode: false },
    ),
    /synthetic fixture path cannot be used as production evidence/,
  );
});

test("fixture evidence loading accepts synthetic evidence in fixture mode", () => {
  const records = loadEvidenceDirectory(
    path.join(FIXTURES, "projection/evidence"),
    { fixtureMode: true },
  );
  assert.equal(records.length, 5);
});

test("the production evidence directory contains no physical JSON records", () => {
  const entries = readdirSync(path.join(REPO_ROOT, "docs/testing/device-qualification/evidence")).sort();
  assert.deepEqual(entries, ["README.md"]);
});

test("matrix rendering is byte deterministic", () => {
  const context = projectionContext();
  const projection = projectQualificationState({
    workflowCatalog: context.workflowCatalog,
    targets: context.targets,
    records: context.records,
    currentBuild: CURRENT_BUILD,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests: AUTHORED_DIGESTS,
  });
  const first = renderQualificationMatrix(projection);
  const second = renderQualificationMatrix(projection);
  assert.equal(first, second);
  assert.equal(
    first,
    readFileSync(path.join(FIXTURES, "matrix/expected-qualified-limited.md"), "utf8"),
  );
});

test("projection preserves device target and workflow catalog order", () => {
  const context = projectionContext();
  const projection = projectQualificationState({
    workflowCatalog: context.workflowCatalog,
    targets: context.targets,
    records: context.records,
    currentBuild: CURRENT_BUILD,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests: AUTHORED_DIGESTS,
  });
  assert.deepEqual(
    projection.targets.map((entry) => entry.target.id),
    [
      targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id,
      targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id,
    ],
  );
  const catalogOrder = context.workflowCatalog.workflows.map((workflow) => workflow.id);
  for (const entry of projection.targets) {
    const rowOrder = entry.workflows.map((row) => row.workflowId);
    const positions = rowOrder.map((workflowId) => catalogOrder.indexOf(workflowId));
    assert.deepEqual(positions, [...positions].sort((left, right) => left - right));
  }
});

test("production --check exits zero and detects matrix drift", () => {
  const check = () => execFileSync(
    process.execPath,
    ["tools/device-qualification.mjs", "--check"],
    { cwd: REPO_ROOT, encoding: "utf8", stdio: "pipe" },
  );
  assert.doesNotThrow(check);
  const committed = readFileSync(
    path.join(REPO_ROOT, "docs/qualification/device-qualification-matrix.md"),
    "utf8",
  );
  assert.match(committed, /No physical-device qualification targets have been registered yet/);
  assert.doesNotMatch(committed, /^## /m);
});

test("the CLI rejects unknown flags", () => {
  assert.throws(
    () => execFileSync(
      process.execPath,
    ["tools/device-qualification.mjs", "--bogus"],
      { cwd: REPO_ROOT, encoding: "utf8", stdio: "pipe" },
    ),
    /usage|unknown/i,
  );
});

test("operator runbook documents the evidence boundary without claiming physical qualification", () => {
  const runbook = readFileSync(
    path.join(REPO_ROOT, "docs/manual/device-qualification-operator.md"),
    "utf8",
  );
  assert.match(runbook, /production EmuChef remains the system under test/i);
  assert.match(runbook, /node tools\/device-qualification\.mjs --check/);
  assert.match(runbook, /node tools\/device-qualification\.mjs --write-matrix/);
  assert.match(runbook, /node tools\/device-qualification\.mjs --build-identity/);
  assert.match(runbook, /node tools\/device-qualification\.mjs --describe/);
  assert.match(runbook, /unable_to_verify/);
  assert.match(runbook, /does not\s+itself imply support/i);
  assert.doesNotMatch(runbook, /EMUCHEF_PHASE_6F_BUILD_IDENTITY/);
  assert.doesNotMatch(runbook, /EMUCHEF_PHASE_6F_RUNTIME_CONTRACT/);
  assert.match(runbook, /rerun `--check` and repository tests before committing evidence/i);
  assert.doesNotMatch(runbook, /first qualified device/i);
});

test("repository validation wires phase 6f without claiming completion", () => {
  const makefile = readFileSync(path.join(REPO_ROOT, "Makefile"), "utf8");
  const workflow = readFileSync(
    path.join(REPO_ROOT, ".github/workflows/emuchef-execution-feature-matrix.yml"),
    "utf8",
  );
  const roadmap = readFileSync(path.join(REPO_ROOT, "docs/product/product-roadmap.md"), "utf8");

  assert.match(makefile, /device-qualification-check:/);
  assert.match(makefile, /node --test tools\/device-qualification\.test\.mjs/);
  assert.match(makefile, /node tools\/device-qualification\.mjs --check/);
  assert.match(makefile, /test: ensure-deps device-qualification-check/);
  assert.match(workflow, /tools\/device-qualification\*/);
  assert.match(workflow, /docs\/testing\/device-qualification\/\*\*/);
  assert.match(workflow, /node --test tools\/device-qualification\.test\.mjs/);
  assert.match(workflow, /node tools\/device-qualification\.mjs --check/);
  assert.match(roadmap, /6F \| Physical-device test matrix \| In progress/);
  assert.match(roadmap, /no physical-device qualification evidence has been added/i);
  assert.doesNotMatch(roadmap, /6F \| Physical-device test matrix \| Completed/);
});

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
