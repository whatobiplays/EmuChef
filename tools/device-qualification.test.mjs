import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  assertSupportedExecutionReportSchema,
} from "./device-qualification-execution-report.mjs";
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
  loadValidatedEvidenceRecords,
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
  validateExecutionReport,
  validateDeviceTargets,
  validateWorkflowCatalog,
} from "./device-qualification.mjs";

const REPO_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const APP_ROOT = path.join(REPO_ROOT, "apps/emuchef-app");
const FIXTURES = path.join(REPO_ROOT, "tests/fixtures/device-qualification");
const AUTHORED_PROFILES = path.join(REPO_ROOT, "authored/device_profiles");
const SYNTHETIC_POCKET_S2_PROFILE = "ayaneo.pocket_s2";
const SYNTHETIC_AIR_MINI_PROFILE = "ayaneo.pocket_air_mini";
const ACTIVE_PROJECT_MANAGEMENT_NAME =
  /\b(?:phase|slice)[\s_-]*\d+[a-z0-9-]*\b/i;
const HISTORICAL_PATHS = [
  /^docs\/superpowers(?:\/|$)/,
  /^docs\/product\/product-roadmap\.md$/,
  /^\.chatgpt(?:\/|$)/,
];
const HISTORICAL_TOOL_REFERENCES = [
  "apps/emuchef-app/src/Phase6d6UiSmoke.tsx",
  "apps/emuchef-app/src-tauri/src/phase6d6_ui_smoke.rs",
];

function listFiles(relativeRoot) {
  const root = path.join(REPO_ROOT, relativeRoot);
  if (!existsSync(root)) return [];
  const files = [];
  const visit = (directory, relativeDirectory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const relativePath = path.join(relativeDirectory, entry.name);
      const absolutePath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolutePath, relativePath);
      } else if (entry.isFile()) {
        files.push(relativePath);
      }
    }
  };
  visit(root, relativeRoot);
  return files;
}

function activeQualificationFiles() {
  const files = [
    ...listFiles("tools").filter(
      (relativePath) =>
        /^device-qualification(?:-[a-z0-9-]+)?\.mjs$/.test(path.basename(relativePath)),
    ),
    ...listFiles("docs/testing/device-qualification"),
    ...listFiles("tests/fixtures/device-qualification"),
    ...listFiles("apps/emuchef-app/src").filter((relativePath) =>
      /qualification/i.test(path.basename(relativePath)),
    ),
    ...listFiles("apps/emuchef-app/src-tauri/src").filter((relativePath) =>
      /qualification/i.test(path.basename(relativePath)),
    ),
    ...listFiles("apps/emuchef-app/scripts").filter((relativePath) =>
      /device-qualification/i.test(path.basename(relativePath)),
    ),
    "docs/manual/device-qualification-operator.md",
    "docs/qualification/device-qualification-matrix.md",
    "Makefile",
    ".github/workflows/emuchef-execution-feature-matrix.yml",
  ];
  return [...new Set(files)].filter(
    (relativePath) =>
      !HISTORICAL_PATHS.some((pattern) => pattern.test(relativePath)),
  );
}

function readActiveQualificationSource(relativePath) {
  let source = readFileSync(path.join(REPO_ROOT, relativePath), "utf8");
  if (relativePath === "tools/device-qualification.mjs") {
    for (const historicalReference of HISTORICAL_TOOL_REFERENCES) {
      source = source.replaceAll(historicalReference, "");
    }
  }
  if (
    relativePath === ".github/workflows/emuchef-execution-feature-matrix.yml"
  ) {
    const marker = "      - name: Validate device qualification foundation";
    const start = source.indexOf(marker);
    assert.notEqual(start, -1, relativePath);
    const nextStep = source.indexOf("\n      - name:", start + marker.length);
    source = source.slice(start, nextStep === -1 ? source.length : nextStep);
  }
  return source;
}

function assertActiveQualificationSourceUsesDomainNames(
  relativePath,
  source = readActiveQualificationSource(relativePath),
) {
  assert.doesNotMatch(
    relativePath,
    ACTIVE_PROJECT_MANAGEMENT_NAME,
    relativePath,
  );
  assert.doesNotMatch(source, ACTIVE_PROJECT_MANAGEMENT_NAME, relativePath);
}

function readJson(relative) {
  return JSON.parse(readFileSync(path.join(FIXTURES, relative), "utf8"));
}

function syntheticContext() {
  return {
    workflowCatalog: loadWorkflowCatalog(
      path.join(FIXTURES, "definitions-valid/workflow-catalog.json"),
    ),
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

function bindReportBytes(sourceBundle, reportBytes) {
  const record = structuredClone(sourceBundle.record);
  record.artifacts[0].sha256 = createHash("sha256")
    .update(reportBytes)
    .digest("hex");
  return { record: sealRecord(record), reportBytes };
}

function ordinaryExecutionReportV2(bundle) {
  return {
    schema: "emuchef.execution-report",
    schemaVersion: 2,
    app: {
      name: "EmuChef",
      version: bundle.record.fingerprint.emuchefBuild.appVersion,
    },
    runtime: {
      status: "ready",
      protocolVersion: 1,
      catalogVersion: null,
    },
    catalog: null,
    plan: {
      planId: "plan.device-qualification-test",
      planDigest: "a".repeat(64),
    },
    execution: {
      simulated: false,
      verificationScope: "real_device",
      status: "succeeded",
      startedAt: "2026-08-21T02:00:00Z",
      finishedAt: "2026-08-21T02:01:00Z",
      completion: {
        classification: "success",
        counts: {
          total: 3,
          completed: 2,
          skipped: 1,
          blocked: 0,
          failed: 0,
          cancelled: 0,
          pending: 0,
        },
        warningCount: 0,
        features: [
          {
            name: "RetroArch provisioning",
            status: "succeeded",
            counts: { completed: 1 },
          },
          {
            name: "Copy BIOS",
            status: "succeeded",
            counts: { completed: 1, skipped: 1 },
          },
        ],
        partialChangesPossible: false,
      },
      recipes: [
        {
          name: "RetroArch provisioning",
          description: null,
          status: "succeeded",
          steps: [
            {
              name: "Install RetroArch",
              note: null,
              status: "succeeded",
              message: null,
            },
          ],
        },
        {
          name: "Copy BIOS",
          description: null,
          status: "succeeded",
          steps: [
            {
              name: "Copy BIOS files",
              note: null,
              status: "succeeded",
              message: null,
            },
            {
              name: "Optional BIOS check",
              note: null,
              status: "skipped",
              message: null,
            },
          ],
        },
      ],
      warnings: [],
      errors: [],
      target: { label: "Connected Android device" },
      resolvedReleases: [],
    },
  };
}

function ordinaryExecutionBundle(updateReport = () => {}) {
  const sourceBundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  const report = ordinaryExecutionReportV2(sourceBundle);
  updateReport(report);
  const reportBytes = Buffer.from(`${JSON.stringify(report)}\n`, "utf8");
  return bindReportBytes(sourceBundle, reportBytes);
}

function romLibrarySyncBundle({
  profileId = "ayaneo.pocket_s2",
  workflowId = "rom-library-sync",
  updateReport = () => {},
} = {}) {
  const context = syntheticContext();
  const target = structuredClone(
    context.targets.find((candidate) => candidate.profileId.value === "ayaneo.pocket_s2"),
  );
  target.profileId.value = profileId;
  if (profileId === "konkr.pocket_advance") {
    target.manufacturer.value = "ARBOR";
    target.model.value = "GT78-VN";
  }
  target.id = deviceTargetId(target);
  context.targets = [target];

  const sourceBundle = loadEvidenceBundle(
    path.join(FIXTURES, "projection/evidence/failed-newer"),
  );
  const record = structuredClone(sourceBundle.record);
  const workflow = context.workflowCatalog.workflows.find(
    (candidate) => candidate.id === workflowId,
  );
  record.workflowId = workflow.id;
  record.workflowVersion = workflow.version;
  record.fingerprint.workflowVersion = workflow.version;
  record.fingerprint.deviceProfile = target.profileId.value;
  for (const [fingerprintField, targetField] of [
    ["androidApi", "androidApi"],
    ["firmwareBuild", "firmwareBuild"],
    ["abiSocClass", "abiSocClass"],
    ["rootState", "rootState"],
    ["connectionType", "connectionType"],
  ]) {
    record.fingerprint[fingerprintField] = target[targetField].value;
  }
  record.fingerprint.authoredContent = workflow.productionRecipes.map((id) => ({
    id,
    sha256: AUTHORED_DIGESTS[id],
  }));
  record.deviceTarget.id = target.id;
  for (const field of [
    "profileId",
    "manufacturer",
    "model",
    "androidVersion",
    "androidApi",
    "abiSocClass",
    "rootState",
    "connectionType",
    "firmwareBuild",
  ]) {
    record.deviceTarget[field] = structuredClone(target[field]);
  }
  record.runValidity = "valid";
  record.qualificationOutcome = "passed";
  record.automatedObservations = [
    {
      id: "execution-report",
      outcome: "passed",
      observedAt: record.capturedAt,
    },
  ];
  record.humanCheckpoints = [];
  record.targetWideFailure = null;
  record.limitations = [];

  const report = ordinaryExecutionReportV2({ record });
  updateReport(report);
  const reportBytes = Buffer.from(`${JSON.stringify(report)}\n`, "utf8");
  return {
    bundle: bindReportBytes({ record: sealRecord(record) }, reportBytes),
    context,
  };
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
    {
      id: "execution-report",
      outcome: "passed",
      observedAt: "2026-08-21T02:01:00Z",
    },
  ];
  record.humanCheckpoints = [
    {
      checkpointId: "device_behavior_verified",
      outcome: "fail",
      observedAt: "2026-08-21T02:02:00Z",
    },
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
  const workflowCatalog = loadWorkflowCatalog(
    path.join(FIXTURES, "definitions-valid/workflow-catalog.json"),
  );
  const targets = loadDeviceTargets(
    path.join(FIXTURES, "projection/device-targets.json"),
    { authoredProfilesDir: AUTHORED_PROFILES },
  ).targets;
  const records = readdirSync(path.join(FIXTURES, "projection/evidence"), {
    withFileTypes: true,
  })
    .filter((entry) => entry.isDirectory())
    .sort((left, right) => left.name.localeCompare(right.name))
    .map((entry) => {
      const bundle = loadEvidenceBundle(
        path.join(FIXTURES, "projection/evidence", entry.name),
      );
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
  const workflow = context.workflowCatalog.workflows.find(
    (item) => item.id === workflowId,
  );
  const target = context.targets.find((item) => item.id === targetId);
  return deriveWorkflowState({
    workflow,
    target,
    currentFingerprint: currentFingerprint(workflow, target),
    records: context.records,
  });
}

const REPORT_BYTES = Buffer.from(
  `${JSON.stringify(
    ordinaryExecutionReportV2({
      record: { fingerprint: { emuchefBuild: CURRENT_BUILD } },
    }),
  )}\n`,
  "utf8",
);

const ROM_MERGE_PIN_PATH =
  "docs/testing/device-qualification/rom-merge-apk-pin.json";
const ROM_MERGE_EVIDENCE_PATH = "rom-merge-evidence.json";
const ROM_MERGE_EVIDENCE_ID = "rom-merge-evidence";
const ROM_MERGE_EVIDENCE_KIND = "rom_merge_evidence";
const ROM_MERGE_SENTINEL_PATH = "qualification-sentinel.keep";
const ROM_MERGE_SENTINEL_SHA256 = "f".repeat(64);
const ROM_MERGE_TEST_PIN = {
  schemaVersion: 1,
  retroarch: {
    packageId: "com.retroarch.aarch64",
    source: "https://example.invalid/retroarch.apk",
    version: "1.19.1",
    sha256: "e".repeat(64),
  },
};

function copyTrackedFile(repoRoot, relativePath) {
  const destination = path.join(repoRoot, relativePath);
  mkdirSync(path.dirname(destination), { recursive: true });
  cpSync(path.join(REPO_ROOT, relativePath), destination);
}

function removeTempTree(root) {
  rmSync(root, {
    recursive: true,
    force: true,
    maxRetries: 5,
    retryDelay: 100,
  });
}

function createTempQualificationRepo({ deviceTargetsSource } = {}) {
  const repoRoot = mkdtempSync(path.join(tmpdir(), "device-qualification-"));
  mkdirSync(path.join(repoRoot, "docs/testing/device-qualification/evidence"), {
    recursive: true,
  });
  mkdirSync(path.join(repoRoot, "docs/qualification"), { recursive: true });
  mkdirSync(path.join(repoRoot, "authored/recipes"), { recursive: true });
  mkdirSync(path.join(repoRoot, "authored/device_profiles"), {
    recursive: true,
  });
  mkdirSync(path.join(repoRoot, "apps/emuchef-app/src-tauri"), {
    recursive: true,
  });

  copyTrackedFile(repoRoot, "apps/emuchef-app/package.json");
  copyTrackedFile(repoRoot, "apps/emuchef-app/package-lock.json");
  copyTrackedFile(repoRoot, "apps/emuchef-app/src-tauri/Cargo.toml");
  copyTrackedFile(repoRoot, "apps/emuchef-app/src-tauri/Cargo.lock");
  copyTrackedFile(repoRoot, "apps/emuchef-app/src-tauri/tauri.conf.json");
  copyTrackedFile(
    repoRoot,
    "docs/testing/device-qualification/evidence-schema.json",
  );
  copyTrackedFile(
    repoRoot,
    "docs/testing/device-qualification/execution-report.schema.json",
  );
  copyTrackedFile(
    repoRoot,
    "docs/testing/device-qualification/workflow-catalog.json",
  );
  copyTrackedFile(
    repoRoot,
    "docs/testing/device-qualification/evidence/README.md",
  );
  copyTrackedFile(
    repoRoot,
    "docs/qualification/device-qualification-matrix.md",
  );
  cpSync(
    path.join(REPO_ROOT, "authored/device_profiles"),
    path.join(repoRoot, "authored/device_profiles"),
    { recursive: true },
  );

  const workflowCatalog = JSON.parse(
    readFileSync(
      path.join(
        REPO_ROOT,
        "docs/testing/device-qualification/workflow-catalog.json",
      ),
      "utf8",
    ),
  );
  const recipeIds = [
    ...new Set(
      workflowCatalog.workflows.flatMap(
        (workflow) => workflow.productionRecipes,
      ),
    ),
  ];
  for (const recipeId of recipeIds) {
    copyTrackedFile(repoRoot, `authored/recipes/${recipeId}.yaml`);
  }

  const targetsSource =
    deviceTargetsSource ??
    path.join(
      REPO_ROOT,
      "docs/testing/device-qualification/device-targets.json",
    );
  copyTrackedFile(repoRoot, path.relative(REPO_ROOT, targetsSource));
  if (
    targetsSource !==
    path.join(
      REPO_ROOT,
      "docs/testing/device-qualification/device-targets.json",
    )
  ) {
    cpSync(
      targetsSource,
      path.join(
        repoRoot,
        "docs/testing/device-qualification/device-targets.json",
      ),
    );
  }

  execFileSync("git", ["init"], { cwd: repoRoot, stdio: "pipe" });
  execFileSync("git", ["config", "--local", "core.fsmonitor", "false"], {
    cwd: repoRoot,
    stdio: "pipe",
  });
  execFileSync("git", ["config", "--local", "maintenance.auto", "false"], {
    cwd: repoRoot,
    stdio: "pipe",
  });
  execFileSync("git", ["config", "--local", "gc.auto", "0"], {
    cwd: repoRoot,
    stdio: "pipe",
  });
  execFileSync("git", ["config", "user.name", "Codex"], {
    cwd: repoRoot,
    stdio: "pipe",
  });
  execFileSync("git", ["config", "user.email", "codex@example.com"], {
    cwd: repoRoot,
    stdio: "pipe",
  });
  execFileSync("git", ["add", "."], { cwd: repoRoot, stdio: "pipe" });
  execFileSync("git", ["commit", "-m", "initial qualification fixture"], {
    cwd: repoRoot,
    stdio: "pipe",
  });
  return repoRoot;
}

function commitRepoChanges(repoRoot, message) {
  execFileSync("git", ["add", "."], { cwd: repoRoot, stdio: "pipe" });
  execFileSync("git", ["commit", "-m", message], {
    cwd: repoRoot,
    stdio: "pipe",
  });
}

function replaceDeviceTargets(
  repoRoot,
  targets,
  commitMessage = "narrow device targets",
) {
  writeFileSync(
    path.join(
      repoRoot,
      "docs/testing/device-qualification/device-targets.json",
    ),
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
        const bytes = readFileSync(
          path.join(repoRoot, "authored/recipes", `${recipeId}.yaml`),
        );
        return [recipeId, createHash("sha256").update(bytes).digest("hex")];
      }),
  );
}

function targetRegistrationCandidateForRepo(
  repoRoot,
  {
    candidateId = "qualification-candidate-0123456789abcdef0123456789abcdef",
    targetIndex = 0,
  } = {},
) {
  const build = buildMaterialIdentity({ repoRoot, requireClean: false });
  const target = structuredClone(
    readJson("definitions-valid/device-targets.json").targets[targetIndex],
  );
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

function runCandidateForRepo(
  repoRoot,
  {
    candidateId = "qualification-candidate-fedcba9876543210fedcba9876543210",
  } = {},
) {
  const workflowCatalog = loadWorkflowCatalog(
    path.join(
      repoRoot,
      "docs/testing/device-qualification/workflow-catalog.json",
    ),
  );
  const targets = loadDeviceTargets(
    path.join(
      repoRoot,
      "docs/testing/device-qualification/device-targets.json",
    ),
    { authoredProfilesDir: path.join(repoRoot, "authored/device_profiles") },
  ).targets;
  const workflow = workflowCatalog.workflows.find(
    (item) => item.id === "retroarch-plus-bios",
  );
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
      {
        id: "execution-report",
        outcome: "passed",
        observedAt: "2026-08-23T12:31:00Z",
      },
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

function writeCandidateFixture(
  repoRoot,
  candidate,
  reportBytes = null,
  romMergeEvidenceBytes = null,
) {
  const directory = path.join(
    repoRoot,
    ".emuchef_runtime/qualification-candidates",
    candidate.candidateId,
  );
  mkdirSync(directory, { recursive: true });
  const report =
    reportBytes === null
      ? null
      : {
          path: "execution-report.json",
          byteLength: reportBytes.length,
          sha256: createHash("sha256").update(reportBytes).digest("hex"),
        };
  writeFileSync(
    path.join(directory, "candidate.json"),
    `${JSON.stringify(
      {
        candidateHandle: candidate.candidateId,
        kind: candidate.kind,
        capturedAt: candidate.capturedAt ?? null,
        build: candidate.build ?? null,
        payload: candidate,
        report,
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  if (reportBytes !== null) {
    writeFileSync(path.join(directory, "execution-report.json"), reportBytes);
  }
  if (romMergeEvidenceBytes !== null) {
    const candidatePath = path.join(directory, "candidate.json");
    const envelope = JSON.parse(readFileSync(candidatePath, "utf8"));
    envelope.romMergeEvidence = {
      path: ROM_MERGE_EVIDENCE_PATH,
      byteLength: romMergeEvidenceBytes.length,
      sha256: createHash("sha256")
        .update(romMergeEvidenceBytes)
        .digest("hex"),
    };
    writeFileSync(candidatePath, `${JSON.stringify(envelope, null, 2)}\n`, "utf8");
    writeFileSync(
      path.join(directory, ROM_MERGE_EVIDENCE_PATH),
      romMergeEvidenceBytes,
    );
  }
}

function writeRomMergePin(repoRoot, pin = ROM_MERGE_TEST_PIN) {
  const pinPath = path.join(repoRoot, ROM_MERGE_PIN_PATH);
  mkdirSync(path.dirname(pinPath), { recursive: true });
  writeFileSync(pinPath, `${JSON.stringify(pin, null, 2)}\n`, "utf8");
}

function createRomMergePinRoot(pin = ROM_MERGE_TEST_PIN) {
  const repoRoot = mkdtempSync(path.join(tmpdir(), "rom-merge-pin-"));
  writeRomMergePin(repoRoot, pin);
  return repoRoot;
}

function configureKonkrQualificationRepo(
  repoRoot,
  pin = ROM_MERGE_TEST_PIN,
) {
  writeRomMergePin(repoRoot, pin);
  const targetsPath = path.join(
    repoRoot,
    "docs/testing/device-qualification/device-targets.json",
  );
  const registry = JSON.parse(readFileSync(targetsPath, "utf8"));
  const target = structuredClone(registry.targets[0]);
  target.profileId.value = "konkr.pocket_advance";
  target.manufacturer.value = "ARBOR";
  target.model.value = "GT78-VN";
  target.id = deviceTargetId(target);
  registry.targets = [target];
  writeFileSync(targetsPath, `${JSON.stringify(registry, null, 2)}\n`, "utf8");
  commitRepoChanges(repoRoot, "prepare synthetic KONKR qualification target");
  return target;
}

function candidateFromEvidenceRecord(record, candidateId, sessionId) {
  return {
    candidateSchemaVersion: 2,
    candidateId,
    sessionId,
    kind: "qualification_run",
    capturedAt: record.capturedAt,
    build: structuredClone(record.fingerprint.emuchefBuild),
    workflowId: record.workflowId,
    workflowVersion: record.workflowVersion,
    deviceTargetId: record.deviceTarget.id,
    fingerprint: structuredClone(record.fingerprint),
    runValidity: record.runValidity,
    qualificationOutcome: record.qualificationOutcome,
    automatedObservations: structuredClone(record.automatedObservations),
    humanCheckpoints: structuredClone(record.humanCheckpoints),
    targetWideFailure: record.targetWideFailure,
    limitations: structuredClone(record.limitations),
    artifacts: structuredClone(record.artifacts),
  };
}

function candidateForKonkrRepo(repoRoot, candidateId) {
  const workflowCatalog = loadWorkflowCatalog(
    path.join(repoRoot, "docs/testing/device-qualification/workflow-catalog.json"),
  );
  const targets = loadDeviceTargets(
    path.join(repoRoot, "docs/testing/device-qualification/device-targets.json"),
    { authoredProfilesDir: path.join(repoRoot, "authored/device_profiles") },
  ).targets;
  const workflow = workflowCatalog.workflows.find(
    (item) => item.id === "rom-library-sync",
  );
  const target = targets.find(
    (item) => item.profileId.value === "konkr.pocket_advance",
  );
  const build = buildMaterialIdentity({ repoRoot, requireClean: false });
  const fingerprint = buildCurrentFingerprint({
    workflow,
    target,
    currentBuild: build,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests: authoredDigestsForRepo(repoRoot, workflowCatalog),
  });
  return {
    candidateSchemaVersion: 2,
    candidateId,
    sessionId: `qualification-session-${"c".repeat(32)}`,
    kind: "qualification_run",
    capturedAt: "2026-08-23T12:30:00Z",
    build,
    workflowId: workflow.id,
    workflowVersion: workflow.version,
    deviceTargetId: target.id,
    fingerprint,
    runValidity: "valid",
    qualificationOutcome: "passed",
    automatedObservations: workflow.automatedObservations.map((observation) => ({
      id: observation.id,
      outcome: "passed",
      observedAt: "2026-08-23T12:31:00Z",
    })),
    humanCheckpoints: workflow.humanCheckpoints.map((checkpoint) => ({
      checkpointId: checkpoint.id,
      outcome: "pass",
      observedAt: "2026-08-23T12:30:30Z",
    })),
    targetWideFailure: null,
    limitations: [],
    artifacts: [],
  };
}

function romMergeExecutionReport(candidate) {
  const report = ordinaryExecutionReportV2({
    record: { fingerprint: { emuchefBuild: candidate.build } },
  });
  const reviewedPlanId = "plan.rom-merge-reviewed";
  const reviewedPlanDigest = "a".repeat(64);
  const buildIdentity = structuredClone(candidate.build);
  report.plan = { planId: reviewedPlanId, planDigest: reviewedPlanDigest };
  report.runtime.executionRestriction = {
    enabled: true,
    buildIdentity,
    runtimeGeneration: 1,
    strategy: "adb_shell",
    rootShell: false,
    appDataWrite: false,
  };
  report.execution.completion.counts = {
    total: 1,
    completed: 1,
    skipped: 0,
    blocked: 0,
    failed: 0,
    cancelled: 0,
    pending: 0,
  };
  report.execution.completion.features = [
    { name: "ROM Merge", status: "succeeded", counts: { completed: 1 } },
  ];
  report.execution.recipes = [
    {
      recipeId: "feature.copy_roms",
      name: "Copy ROM library",
      description: null,
      status: "succeeded",
      steps: [
        {
          recipeId: "feature.copy_roms",
          stepId: "feature.copy_roms/copy_rom_library",
          name: "Copy ROM library",
          note: null,
          status: "succeeded",
          message: null,
        },
      ],
    },
  ];
  report.qualification = {
    claim: "restricted_unprivileged",
    attemptId: candidate.candidateId,
    executionMode: "real",
    strategy: "adb_unprivileged",
    restriction: "privileged_execution_disabled",
    observedRootState: candidate.fingerprint.rootState,
    effectiveCapabilities: ["shared_storage_write"],
    buildIdentity,
    deviceTargetId: candidate.deviceTargetId,
    deviceProfileId: candidate.fingerprint.deviceProfile,
    reviewedPlanId,
    reviewedPlanDigest,
    requiredOperationIds: ["feature.copy_roms"],
    operationOutcomes: [
      { operationId: "feature.copy_roms", outcome: "executed" },
    ],
  };
  return report;
}

function romMergeEvidenceFor(
  candidate,
  report,
  reportBytes,
  pin = ROM_MERGE_TEST_PIN,
) {
  const sourceEntry = {
    relativePath: "roms/game.rom",
    sha256: "1".repeat(64),
  };
  const sentinelEntry = {
    relativePath: ROM_MERGE_SENTINEL_PATH,
    sha256: ROM_MERGE_SENTINEL_SHA256,
  };
  return {
    schemaVersion: 1,
    candidateHandle: candidate.candidateId,
    sessionId: candidate.sessionId,
    executionReportSha256: createHash("sha256").update(reportBytes).digest("hex"),
    deviceTargetId: candidate.deviceTargetId,
    deviceProfileId: candidate.fingerprint.deviceProfile,
    androidApi: candidate.fingerprint.androidApi,
    firmwareBuild: candidate.fingerprint.firmwareBuild,
    abiSocClass: candidate.fingerprint.abiSocClass,
    connectionType: candidate.fingerprint.connectionType,
    observedRootState: candidate.fingerprint.rootState,
    buildIdentity: structuredClone(candidate.build),
    reviewedPlanId: report.qualification.reviewedPlanId,
    reviewedPlanDigest: report.qualification.reviewedPlanDigest,
    terminalOperation: {
      recipeId: "feature.copy_roms",
      stepId: "feature.copy_roms/copy_rom_library",
      outcome: "succeeded",
    },
    copyPolicy: "merge",
    strategyDigest: canonicalDigest({
      domain: "emuchef.rom-merge.strategy",
      version: 1,
      strategy: report.qualification.strategy,
      restriction: report.qualification.restriction,
      runtimeRestriction: report.runtime.executionRestriction,
      effectiveCapabilities: report.qualification.effectiveCapabilities,
    }),
    sourceSnapshot: {
      sourceIdentity: "source-volume-identity-01",
      entries: [sourceEntry],
    },
    destination: {
      path: "/storage/emulated/0/ROMs",
      setupLocation: "shared-storage",
      volumeIdentity: "destination-volume-identity-01",
      before: [structuredClone(sourceEntry), structuredClone(sentinelEntry)],
      after: [structuredClone(sourceEntry), structuredClone(sentinelEntry)],
    },
    sentinel: {
      relativePath: ROM_MERGE_SENTINEL_PATH,
      expectedSha256: ROM_MERGE_SENTINEL_SHA256,
    },
    retroArch: {
      packageId: pin.retroarch.packageId,
      installedVersion: pin.retroarch.version,
      pinnedSource: pin.retroarch.source,
      pinnedVersion: pin.retroarch.version,
      installedApkSha256: pin.retroarch.sha256,
      pinnedArtifactSha256: pin.retroarch.sha256,
    },
    rootAuthorityUsed: false,
    oemCompilerUsed: false,
    oemStageUsed: false,
  };
}

function romMergeEvidenceBundle({
  candidateId = "qualification-candidate-1234567890abcdef1234567890abcdef",
  sessionId = `qualification-session-${"d".repeat(32)}`,
  updateReport = () => {},
  updateEvidence = () => {},
  pin = ROM_MERGE_TEST_PIN,
} = {}) {
  const { bundle: sourceBundle, context } = romLibrarySyncBundle({
    profileId: "konkr.pocket_advance",
  });
  const candidate = candidateFromEvidenceRecord(
    sourceBundle.record,
    candidateId,
    sessionId,
  );
  const report = romMergeExecutionReport(candidate);
  updateReport(report);
  const reportBytes = Buffer.from(`${JSON.stringify(report)}\n`, "utf8");
  const boundReportBundle = bindReportBytes(sourceBundle, reportBytes);
  candidate.artifacts = structuredClone(boundReportBundle.record.artifacts);
  const evidence = romMergeEvidenceFor(candidate, report, reportBytes, pin);
  updateEvidence(evidence);
  const romMergeEvidenceBytes = Buffer.from(
    `${JSON.stringify(evidence)}\n`,
    "utf8",
  );
  const record = structuredClone(boundReportBundle.record);
  record.artifacts.push({
    id: ROM_MERGE_EVIDENCE_ID,
    kind: ROM_MERGE_EVIDENCE_KIND,
    path: ROM_MERGE_EVIDENCE_PATH,
    sha256: createHash("sha256")
      .update(romMergeEvidenceBytes)
      .digest("hex"),
  });
  return {
    bundle: {
      record: sealRecord(record),
      reportBytes,
      romMergeEvidenceBytes,
      candidate,
    },
    context,
    candidate,
    report,
    evidence,
    romMergeEvidenceBytes,
  };
}

function candidateRomMergeArtifacts(
  repoRoot,
  candidateId,
  pin = ROM_MERGE_TEST_PIN,
  { configureRepo = true } = {},
) {
  if (configureRepo) configureKonkrQualificationRepo(repoRoot, pin);
  const candidate = candidateForKonkrRepo(repoRoot, candidateId);
  const report = romMergeExecutionReport(candidate);
  const reportBytes = Buffer.from(`${JSON.stringify(report)}\n`, "utf8");
  candidate.artifacts = [
    {
      id: "execution-report",
      kind: "production_execution_report",
      path: "execution-report.json",
      sha256: createHash("sha256").update(reportBytes).digest("hex"),
    },
  ];
  const evidence = romMergeEvidenceFor(candidate, report, reportBytes, pin);
  return {
    candidate,
    report,
    reportBytes,
    romMergeEvidenceBytes: Buffer.from(`${JSON.stringify(evidence)}\n`, "utf8"),
  };
}

function installEvidenceBundle(
  repoRoot,
  fixtureRelative,
  { reportBytes } = {},
) {
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
    for (const entry of readdirSync(current, { withFileTypes: true }).sort(
      (left, right) => left.name.localeCompare(right.name),
    )) {
      const nextRelative = relative
        ? path.join(relative, entry.name)
        : entry.name;
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

test("clean build identity rejects untracked checkout material", () => {
  const repoRoot = createTempQualificationRepo();
  const untrackedPath = path.join(
    repoRoot,
    "authored/device_profiles/untracked-build-input.yaml",
  );
  try {
    writeFileSync(untrackedPath, "profile: untracked\n", "utf8");
    assert.throws(
      () => buildMaterialIdentity({ repoRoot, requireClean: true }),
      /clean source worktree/,
    );
  } finally {
    removeTempTree(repoRoot);
  }
});

test("loads a strict version-1 workflow catalog from fixtures", () => {
  const catalog = loadWorkflowCatalog(
    path.join(FIXTURES, "definitions-valid/workflow-catalog.json"),
  );
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
  assert.doesNotThrow(() =>
    validateDeviceTargets(
      { schemaVersion: 2, targets: [target] },
      { authoredProfilesDir: AUTHORED_PROFILES },
    ),
  );

  const illegalRoot = structuredClone(target);
  illegalRoot.rootState.source = "operator_attestation";
  illegalRoot.id = deviceTargetId(illegalRoot);
  assert.throws(
    () =>
      validateDeviceTargets(
        { schemaVersion: 2, targets: [illegalRoot] },
        { authoredProfilesDir: AUTHORED_PROFILES },
      ),
    /rootState.*explicit_root_check/i,
  );
});

test("schema-v2 device targets reject malformed fact wrappers through validation errors", () => {
  const valid = structuredClone(
    readJson("definitions-valid/device-targets.json"),
  );
  const malformed = valid.targets[0];
  malformed.rootState = null;
  assert.throws(
    () =>
      validateDeviceTargets(valid, { authoredProfilesDir: AUTHORED_PROFILES }),
    /device target rootState/i,
  );
});

test("target identity excludes provenance source and policy fields", () => {
  const target = structuredClone(
    readJson("definitions-valid/device-targets.json").targets[0],
  );
  const original = deviceTargetId(target);
  target.capabilities = [];
  target.deferredWorkflows = ["xaniteog-install"];
  target.connectionType.source = "production_observation";
  assert.equal(deviceTargetId(target), original);
  target.connectionType.value =
    target.connectionType.value === "usb3" ? "usb2" : "usb3";
  assert.notEqual(deviceTargetId(target), original);
});

test("schema-v2 evidence records require qualification run ids", () => {
  const context = syntheticContext();
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  assert.match(record.runId, /^qualification-run-sha256:[0-9a-f]{64}$/);

  const oldPrefix = structuredClone(record);
  oldPrefix.runId = `legacy-run-sha256:${"1".repeat(64)}`;
  oldPrefix.recordDigest = evidenceRecordDigest(oldPrefix);
  assert.throws(
    () => validateEvidenceRecord(oldPrefix, context),
    /runId format/i,
  );
});

test("rejects duplicate workflow ids", () => {
  assert.throws(
    () =>
      loadWorkflowCatalog(
        path.join(FIXTURES, "definitions-invalid/duplicate-workflow-id.json"),
      ),
    /duplicate workflow id/,
  );
});

test("rejects device targets whose authored profile does not exist", () => {
  assert.throws(
    () =>
      loadDeviceTargets(
        path.join(FIXTURES, "definitions-invalid/unknown-profile.json"),
        { authoredProfilesDir: AUTHORED_PROFILES },
      ),
    /unknown authored device profile/,
  );
});

test("rejects unknown, missing, and mistyped workflow catalog fields", () => {
  const valid = readJson("definitions-valid/workflow-catalog.json");
  assert.throws(
    () => validateWorkflowCatalog({ ...valid, extra: true }),
    /fields must be exactly/i,
  );
  assert.throws(
    () => validateWorkflowCatalog({ ...valid, workflows: [] }),
    /workflow/i,
  );
  assert.throws(
    () => validateWorkflowCatalog({ ...valid, schemaVersion: 2 }),
    /schemaVersion/i,
  );

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
  assert.throws(
    () =>
      validateDeviceTargets(
        { ...valid, extra: true },
        { authoredProfilesDir: AUTHORED_PROFILES },
      ),
    /fields must be exactly/i,
  );
  assert.throws(
    () =>
      validateDeviceTargets(
        {
          ...valid,
          targets: [
            {
              ...target,
              androidApi: { ...target.androidApi, value: "35" },
              id: deviceTargetId({
                ...target,
                androidApi: { ...target.androidApi, value: "35" },
              }),
            },
          ],
        },
        { authoredProfilesDir: AUTHORED_PROFILES },
      ),
    /androidApi/,
  );
  assert.throws(
    () =>
      validateDeviceTargets(
        {
          ...valid,
          targets: [
            {
              ...target,
              rootState: { ...target.rootState, value: "unknown" },
              id: deviceTargetId({
                ...target,
                rootState: { ...target.rootState, value: "unknown" },
              }),
            },
          ],
        },
        { authoredProfilesDir: AUTHORED_PROFILES },
      ),
    /rootState/,
  );
  assert.throws(
    () =>
      validateDeviceTargets(
        {
          ...valid,
          targets: [
            {
              ...target,
              connectionType: { ...target.connectionType, value: "wifi" },
              id: deviceTargetId({
                ...target,
                connectionType: { ...target.connectionType, value: "wifi" },
              }),
            },
          ],
        },
        { authoredProfilesDir: AUTHORED_PROFILES },
      ),
    /connectionType/,
  );
  assert.throws(
    () =>
      validateDeviceTargets(
        { ...valid, schemaVersion: 1 },
        { authoredProfilesDir: AUTHORED_PROFILES },
      ),
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
    {
      path: "authored/recipes/app.retroarch.provision.yaml",
      sha256: "a".repeat(64),
    },
    {
      path: "crates/emuchef-rust-backend/src/planner.rs",
      sha256: "b".repeat(64),
    },
    {
      path: "docs/testing/device-qualification/execution-report.schema.json",
      sha256: "c".repeat(64),
    },
  ];
  const original = materialBuildDigestFromEntries(base);
  const changedProduct = materialBuildDigestFromEntries([
    base[0],
    { path: base[1].path, sha256: "c".repeat(64) },
  ]);
  assert.notEqual(changedProduct, original);

  const changedReportSchema = materialBuildDigestFromEntries([
    ...base.slice(0, 2),
    { path: base[2].path, sha256: "d".repeat(64) },
  ]);
  assert.notEqual(changedReportSchema, original);

  const withEvidence = materialBuildDigestFromEntries([
    ...base,
    {
      path: "docs/testing/device-qualification/evidence/example/evidence.json",
      sha256: "d".repeat(64),
    },
  ]);
  assert.equal(withEvidence, original);
});

test("temporary qualification repositories disable Git background writers locally", () => {
  const repoRoot = createTempQualificationRepo();
  try {
    for (const [key, expected] of [
      ["core.fsmonitor", "false"],
      ["maintenance.auto", "false"],
      ["gc.auto", "0"],
      ["user.name", "Codex"],
      ["user.email", "codex@example.com"],
    ]) {
      assert.equal(
        execFileSync("git", ["config", "--local", "--get", key], {
          cwd: repoRoot,
          encoding: "utf8",
        }).trim(),
        expected,
        `${key} should be set in the temporary repository's local Git config`,
      );
    }
  } finally {
    removeTempTree(repoRoot);
  }
});

test("material identity refreshes for changed inputs and rejects dirty state", () => {
  const repoRoot = createTempQualificationRepo();
  const toolDestination = path.join(repoRoot, "tools/device-qualification.mjs");
  const packagePath = path.join(repoRoot, "apps/emuchef-app/package.json");

  try {
    mkdirSync(path.dirname(toolDestination), { recursive: true });
    cpSync(
      path.join(REPO_ROOT, "tools/device-qualification.mjs"),
      toolDestination,
    );
    cpSync(
      path.join(REPO_ROOT, "tools/device-qualification-execution-report.mjs"),
      path.join(repoRoot, "tools/device-qualification-execution-report.mjs"),
    );
    const toolPath = realpathSync(toolDestination);
    execFileSync("git", ["add", "tools/device-qualification.mjs", "tools/device-qualification-execution-report.mjs"], {
      cwd: repoRoot,
      stdio: "pipe",
    });
    execFileSync("git", ["commit", "-m", "add canonical identity tool"], {
      cwd: repoRoot,
      stdio: "pipe",
    });

    const runIdentity = (requireClean) =>
      spawnSync(
        process.execPath,
        [
          toolPath,
          "--build-identity",
          ...(requireClean ? ["--require-clean"] : []),
        ],
        { cwd: repoRoot, encoding: "utf8" },
      );

    const cleanResult = runIdentity(true);
    assert.equal(cleanResult.status, 0, cleanResult.stderr);
    const cleanIdentity = JSON.parse(cleanResult.stdout);

    const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));
    packageJson.version = "0.1.1";
    writeFileSync(
      packagePath,
      `${JSON.stringify(packageJson, null, 2)}\n`,
      "utf8",
    );

    const refreshedResult = runIdentity(false);
    assert.equal(refreshedResult.status, 0, refreshedResult.stderr);
    const refreshedIdentity = JSON.parse(refreshedResult.stdout);
    assert.notEqual(
      refreshedIdentity.materialBuildDigest,
      cleanIdentity.materialBuildDigest,
    );

    const dirtyResult = runIdentity(true);
    assert.notEqual(dirtyResult.status, 0);
    assert.match(
      `${dirtyResult.stdout}${dirtyResult.stderr}`,
      /clean source worktree/,
    );
  } finally {
    removeTempTree(repoRoot);
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
    compareBuildIdentity(left, {
      ...evidence,
      materialBuildDigest: `sha256:${"b".repeat(64)}`,
    }),
    "invalidating",
  );
});

test("production catalog loads and every production recipe exists", () => {
  const catalog = loadWorkflowCatalog(
    path.join(
      REPO_ROOT,
      "docs/testing/device-qualification/workflow-catalog.json",
    ),
  );
  assert.equal(catalog.workflows.length, 4);
  assert.deepEqual(
    catalog.workflows.map((workflow) => workflow.id),
    [
      "retroarch-plus-bios",
      "obtainium-install",
      "xaniteog-install",
      "rom-library-sync",
    ],
  );
  for (const workflow of catalog.workflows) {
    for (const recipeId of workflow.productionRecipes) {
      const recipePath = path.join(
        REPO_ROOT,
        "authored/recipes",
        `${recipeId}.yaml`,
      );
      assert.equal(
        existsSync(recipePath),
        true,
        `${recipeId} must exist in authored/recipes`,
      );
      assert.equal(
        readFileSync(recipePath, "utf8").includes(`id: ${recipeId}`),
        true,
        `${recipeId} must exist in authored/recipes`,
      );
    }
  }
});

test("production catalog binds retroarch prerequisite review to a required human checkpoint", () => {
  const catalog = loadWorkflowCatalog(
    path.join(
      REPO_ROOT,
      "docs/testing/device-qualification/workflow-catalog.json",
    ),
  );
  const retroarch = catalog.workflows.find(
    (workflow) => workflow.id === "retroarch-plus-bios",
  );
  assert.equal(retroarch.version, 2);
  assert.deepEqual(retroarch.prerequisites, [
    "clean_or_deliberately_reset_device",
  ]);
  assert.deepEqual(retroarch.humanCheckpoints, [
    {
      id: "clean_or_deliberately_reset_device",
      instruction:
        "Before execution, verify the connected device is clean or has been deliberately reset to the intended qualification baseline.",
      fact: "The device was clean or deliberately reset before this qualification run.",
      allowedOutcomes: ["pass", "fail", "unable_to_verify"],
      required: true,
    },
  ]);
});

test("production device registry contains the registered Pocket Air Mini contract", () => {
  const registry = loadDeviceTargets(
    path.join(
      REPO_ROOT,
      "docs/testing/device-qualification/device-targets.json",
    ),
    { authoredProfilesDir: AUTHORED_PROFILES },
  );
  assert.equal(registry.schemaVersion, 2);
  assert.equal(registry.targets.length, 1);

  const [target] = registry.targets;
  assert.equal(target.profileId.value, "ayaneo.pocket_air_mini");
  assert.equal(target.profileId.source, "production_observation");
  assert.equal(target.manufacturer.value, "ARBOR");
  assert.equal(target.manufacturer.source, "production_observation");
  assert.equal(target.model.value, "GT78-VN");
  assert.equal(target.model.source, "production_observation");
  assert.equal(target.androidApi.source, "production_observation");
  assert.equal(target.abiSocClass.source, "production_observation");
  assert.equal(target.rootState.value, "non_root");
  assert.equal(target.rootState.source, "explicit_root_check");
  assert.equal(target.connectionType.value, "usb2");
  assert.equal(target.connectionType.source, "operator_attestation");
  assert.deepEqual(target.capabilities, [
    "apk_install",
    "shared_storage_write",
  ]);
  assert.deepEqual(target.deferredWorkflows, []);
  assert.equal(target.id, deviceTargetId(target));
});

test("the evidence schema matches the validator contract", () => {
  const schema = JSON.parse(
    readFileSync(
      path.join(
        REPO_ROOT,
        "docs/testing/device-qualification/evidence-schema.json",
      ),
      "utf8",
    ),
  );
  assert.deepEqual(
    [...schema.required].sort(),
    [...EVIDENCE_RECORD_FIELDS].sort(),
  );
  assert.deepEqual(
    [...schema.properties.runValidity.enum].sort(),
    [...RUN_VALIDITIES].sort(),
  );
  assert.deepEqual(
    [...schema.properties.qualificationOutcome.enum].sort(),
    [...QUALIFICATION_OUTCOMES].sort(),
  );
  assert.deepEqual(
    [...schema.properties.targetWideFailure.oneOf[1].enum].sort(),
    [...TARGET_WIDE_FAILURES.filter((value) => value !== null)].sort(),
  );
  assert.deepEqual(
    [...schema.$defs.fingerprint.required].sort(),
    [...FINGERPRINT_FIELDS].sort(),
  );
  assert.deepEqual(
    [...schema.$defs.humanCheckpoint.properties.outcome.enum].sort(),
    [...CHECKPOINT_OUTCOMES].sort(),
  );
  assert.doesNotThrow(() => validateEvidenceSchemaContract(schema));
});

test("the execution report schema defines a strict v2 envelope", () => {
  const schema = JSON.parse(
    readFileSync(
      path.join(
        REPO_ROOT,
        "docs/testing/device-qualification/execution-report.schema.json",
      ),
      "utf8",
    ),
  );
  const required = [
    "schema",
    "schemaVersion",
    "app",
    "runtime",
    "catalog",
    "plan",
    "execution",
  ];
  assert.equal(schema.$schema, "https://json-schema.org/draft/2020-12/schema");
  assert.equal(schema.additionalProperties, false);
  assert.equal(schema.properties.schema.const, "emuchef.execution-report");
  assert.equal(schema.properties.schemaVersion.const, 2);
  assert.deepEqual(schema.required, required);
  assert.deepEqual(Object.keys(schema.properties).sort(), [
    ...required,
    "qualification",
  ].sort());

  const checkObjectSchemas = (value, label = "schema") => {
    if (value === null || typeof value !== "object") return;
    const declaredTypes = Array.isArray(value.type) ? value.type : [value.type];
    if (declaredTypes.includes("integer")) {
      assert.equal(
        value.maximum,
        Number.MAX_SAFE_INTEGER,
        `${label} integers must fit in a JavaScript safe integer`,
      );
    }
    if (value.type === "object") {
      assert.equal(value.additionalProperties, false, `${label} must reject unknown fields`);
      const required = value.required ?? [];
      assert.ok(Array.isArray(required), `${label} required fields must be an array`);
      assert.equal(
        new Set(required).size,
        required.length,
        `${label} must not repeat required fields`,
      );
      for (const field of required) {
        assert.ok(
          Object.hasOwn(value.properties ?? {}, field),
          `${label} required field ${field} must have a property schema`,
        );
      }
    }
    for (const [key, child] of Object.entries(value)) {
      if (Array.isArray(child)) {
        child.forEach((item, index) => checkObjectSchemas(item, `${label}.${key}[${index}]`));
      } else if (child !== null && typeof child === "object") {
        checkObjectSchemas(child, `${label}.${key}`);
      }
    }
  };
  checkObjectSchemas(schema);
});

test("the execution report parser rejects schema keywords outside its supported subset", () => {
  const schema = JSON.parse(
    readFileSync(
      path.join(
        REPO_ROOT,
        "docs/testing/device-qualification/execution-report.schema.json",
      ),
      "utf8",
    ),
  );
  schema.$defs.runtimeError.properties.actions.contains = {
    const: "recovery action",
  };

  assert.throws(
    () => assertSupportedExecutionReportSchema(schema),
    /unsupported execution report schema keyword contains/i,
  );
});

test("the Rust-produced report fixture validates from its stored bytes", () => {
  const reportBytes = readFileSync(
    path.join(
      REPO_ROOT,
      "docs/testing/device-qualification/fixtures/execution-report-v2-real.json",
    ),
  );
  const report = validateExecutionReport(reportBytes);

  assert.equal(report.schemaVersion, 2);
  assert.equal(report.qualification.claim, "restricted_unprivileged");
  assert.equal(report.execution.status, "succeeded");
});

test("ordinary Rust report exports validate from their serialized bytes", () => {
  const fixtureRoot = path.join(
    REPO_ROOT,
    "docs/testing/device-qualification/fixtures",
  );
  const successBytes = readFileSync(
    path.join(fixtureRoot, "execution-report-v2-ordinary-real-success.json"),
  );
  const failureBytes = readFileSync(
    path.join(fixtureRoot, "execution-report-v2-ordinary-real-failure.json"),
  );
  const simulatedBytes = readFileSync(
    path.join(fixtureRoot, "execution-report-v2-simulated.json"),
  );

  const success = validateExecutionReport(successBytes);
  assert.equal(success.qualification, undefined);
  assert.equal(success.plan.planId, "plan.one");
  assert.equal(success.execution.status, "succeeded_with_warnings");
  assert.equal(success.execution.warnings.length, 1);
  assert.equal(success.execution.completion.warningCount, 1);
  assert.equal(
    success.execution.resolvedReleases[0].redactedFinalUrl,
    "https://downloads.example.test/releases/app.apk",
  );
  assert.doesNotMatch(
    successBytes.toString("utf8"),
    /sensitive-serial|private-warning-message|private-password|token=private-token|private-fragment/,
  );

  const failure = validateExecutionReport(failureBytes);
  assert.equal(failure.qualification, undefined);
  assert.equal(failure.plan.planId, "plan.one");
  assert.equal(failure.execution.status, "failed");
  assert.equal(failure.execution.errors.length, 1);
  assert.doesNotMatch(
    failureBytes.toString("utf8"),
    /private-failure-message|sensitive-serial/,
  );

  const simulated = validateExecutionReport(simulatedBytes);
  assert.equal(simulated.execution.simulated, true);
  assert.equal(simulated.execution.verificationScope, "simulation_only");
  assert.equal(simulated.execution.target, null);
});

test("the Node validator rejects mutations of ordinary Rust report exports", () => {
  const fixtureRoot = path.join(
    REPO_ROOT,
    "docs/testing/device-qualification/fixtures",
  );
  const readReport = (name) =>
    JSON.parse(readFileSync(path.join(fixtureRoot, name), "utf8"));
  const reportBytes = (report) => Buffer.from(JSON.stringify(report), "utf8");

  const warningCountMismatch = readReport(
    "execution-report-v2-ordinary-real-success.json",
  );
  warningCountMismatch.execution.completion.warningCount = 0;
  assert.throws(
    () => validateExecutionReport(reportBytes(warningCountMismatch)),
    /warningCount/,
  );

  const failureClassificationMismatch = readReport(
    "execution-report-v2-ordinary-real-failure.json",
  );
  failureClassificationMismatch.execution.completion.classification = "success";
  assert.throws(
    () => validateExecutionReport(reportBytes(failureClassificationMismatch)),
    /classification/,
  );

  const simulatedTarget = readReport("execution-report-v2-simulated.json");
  simulatedTarget.execution.target = { label: "Connected Android device" };
  assert.throws(
    () => validateExecutionReport(reportBytes(simulatedTarget)),
    /simulated execution report target must be null/,
  );

  const unredactedUrl = readReport(
    "execution-report-v2-ordinary-real-success.json",
  );
  unredactedUrl.execution.resolvedReleases[0].redactedFinalUrl +=
    "?token=private-token";
  assert.throws(
    () => validateExecutionReport(reportBytes(unredactedUrl)),
    /redactedFinalUrl/,
  );
});

test("the Rust-produced report fixture rejects missing required v2 fields", () => {
  const report = JSON.parse(
    readFileSync(
      path.join(
        REPO_ROOT,
        "docs/testing/device-qualification/fixtures/execution-report-v2-real.json",
      ),
      "utf8",
    ),
  );
  delete report.execution.completion.counts;

  assert.throws(
    () => validateExecutionReport(Buffer.from(JSON.stringify(report), "utf8")),
    /execution report completion fields must be exactly/i,
  );
});

test("execution report catalog digests use lowercase SHA-256 values", () => {
  const report = JSON.parse(
    readFileSync(
      path.join(
        REPO_ROOT,
        "docs/testing/device-qualification/fixtures/execution-report-v2-real.json",
      ),
      "utf8",
    ),
  );
  report.catalog.contentDigest.value = "catalog";

  assert.throws(
    () => validateExecutionReport(Buffer.from(JSON.stringify(report), "utf8")),
    /execution report catalog contentDigest value does not match its schema pattern/i,
  );
});

test("execution report runtime error actions must be non-empty strings", () => {
  const bundle = ordinaryExecutionBundle();
  const report = JSON.parse(bundle.reportBytes.toString("utf8"));
  report.runtime = {
    status: "unsupported",
    error: {
      code: "unsupported-runtime",
      message: "This runtime is unsupported.",
      actions: [""],
    },
  };

  assert.throws(
    () => validateExecutionReport(Buffer.from(JSON.stringify(report), "utf8")),
    /execution report runtime error actions must be an array of non-empty strings/i,
  );
});

test("execution report timestamps reject invalid calendar and clock values", () => {
  const bundle = ordinaryExecutionBundle();
  const report = JSON.parse(bundle.reportBytes.toString("utf8"));

  for (const value of ["2026-02-31T00:00:00Z", "2026-01-01T24:00:00Z"]) {
    report.execution.startedAt = value;
    assert.throws(
      () => validateExecutionReport(Buffer.from(JSON.stringify(report), "utf8")),
      /execution report execution startedAt must be an RFC3339 timestamp/i,
    );
  }
});

test("valid evidence requires one digest-bound production execution report", () => {
  const bundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  assert.equal(bundle.record.runValidity, "valid");
  assert.equal(bundle.record.artifacts.length, 1);
  assert.equal(bundle.record.artifacts[0].id, "execution-report");
  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("ordinary execution report v2 validates without a restricted claim", () => {
  const bundle = ordinaryExecutionBundle();
  const report = JSON.parse(bundle.reportBytes.toString("utf8"));

  assert.equal(report.qualification, undefined);
  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("simulated ordinary v2 reports are schema-valid but ineligible as valid evidence", () => {
  const bundle = ordinaryExecutionBundle((report) => {
    report.execution.simulated = true;
    report.execution.verificationScope = "simulation_only";
    report.execution.target = null;
  });

  assert.doesNotThrow(() => validateExecutionReport(bundle.reportBytes));
  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /valid canonical execution report requires simulated=false and real_device/i,
  );
});

test("simulated ordinary v2 reports remain valid in audit evidence", () => {
  const sourceBundle = ordinaryExecutionBundle((report) => {
    report.execution.simulated = true;
    report.execution.verificationScope = "simulation_only";
    report.execution.target = null;
  });
  const record = structuredClone(sourceBundle.record);
  record.runValidity = "invalid";
  record.qualificationOutcome = "not_observed";
  record.automatedObservations = [];
  const bundle = bindReportBytes(
    { record: sealRecord(record) },
    sourceBundle.reportBytes,
  );

  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("ordinary execution report v2 rejects unknown nested fields", () => {
  const bundle = ordinaryExecutionBundle((report) => {
    report.execution.completion.unknown = true;
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /execution report completion fields must be exactly/i,
  );
});

test("execution report counts must be safe integers", () => {
  const bundle = ordinaryExecutionBundle((report) => {
    report.execution.completion.counts.total = Number.MAX_SAFE_INTEGER + 1;
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /must be a non-negative integer/i,
  );
});

test("ordinary execution report v2 rejects report-only recipe identities", () => {
  const bundle = ordinaryExecutionBundle((report) => {
    report.execution.recipes[0].recipeId = "app.retroarch.provision";
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /ordinary execution report must omit recipe and step IDs/i,
  );
});

test("canonical execution reports require a reviewed plan identity", () => {
  const bundle = ordinaryExecutionBundle((report) => {
    report.plan.planId = null;
    report.plan.planDigest = null;
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /canonical execution report requires plan identity/i,
  );
});

test("canonical execution status must match the recorded observation and outcome", () => {
  const bundle = ordinaryExecutionBundle((report) => {
    report.execution.status = "failed";
    report.execution.completion.classification = "failed";
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /execution report status does not match/i,
  );
});

test("a target-wide failure can fail qualification after successful execution", () => {
  const sourceBundle = ordinaryExecutionBundle();
  const record = structuredClone(sourceBundle.record);
  record.qualificationOutcome = "failed";
  record.targetWideFailure = "safety_invariant_failed";
  record.limitations = ["safety_invariant_failed"];
  const bundle = bindReportBytes(
    { record: sealRecord(record) },
    sourceBundle.reportBytes,
  );

  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("passing KONKR ROM sync requires supplemental ROM Merge evidence", () => {
  const { bundle, context } = romLibrarySyncBundle({
    profileId: "konkr.pocket_advance",
    updateReport(report) {
      report.execution.target.label = "AYANEO Pocket S2";
    },
  });

  assert.equal(bundle.record.deviceTarget.profileId.value, "konkr.pocket_advance");
  assert.throws(
    () => validateEvidenceBundle(bundle, context),
    /ROM Merge evidence is required for konkr\.pocket_advance/i,
  );
});

test("the ROM sync veto allows unrelated trusted profiles and KONKR workflows", () => {
  const unrelatedProfile = romLibrarySyncBundle();
  assert.doesNotThrow(() =>
    validateEvidenceBundle(unrelatedProfile.bundle, unrelatedProfile.context),
  );

  const unrelatedWorkflow = romLibrarySyncBundle({
    profileId: "konkr.pocket_advance",
    workflowId: "obtainium-install",
  });
  assert.doesNotThrow(() =>
    validateEvidenceBundle(unrelatedWorkflow.bundle, unrelatedWorkflow.context),
  );
});

test("invalid audit evidence may omit report only when it is not referenced", () => {
  const bundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/invalid-report-unavailable"),
  );
  assert.equal(bundle.record.runValidity, "invalid");
  assert.deepEqual(bundle.record.artifacts, []);
  assert.equal(bundle.reportBytes, null);
  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("changing a bound report invalidates the bundle", () => {
  const bundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  assert.throws(
    () =>
      validateEvidenceBundle(
        { ...bundle, reportBytes: Buffer.from("{}") },
        syntheticContext(),
      ),
    /execution report digest/i,
  );
});

test("an active negative fixture preserves a mismatched report digest", () => {
  const bundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-invalid/bad-digest"),
  );

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /execution report digest does not match/i,
  );
});

test("canonical evidence rejects legacy execution report schema v1", () => {
  const source = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  const reportBytes = Buffer.from(
    '{"schema":"emuchef.execution-report","schemaVersion":1,"execution":{"status":"succeeded"}}\n',
    "utf8",
  );

  assert.throws(
    () =>
      validateEvidenceBundle(
        bindReportBytes(source, reportBytes),
        syntheticContext(),
      ),
    /execution report schemaVersion must be 2/i,
  );
});

test("inventory excludes digest-bound v1 history while retaining v2 records", () => {
  const context = syntheticContext();
  const currentBundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  const historicalReportBytes = Buffer.from(
    '{"schema":"emuchef.execution-report","schemaVersion":1,"legacyPayload":true}\n',
    "utf8",
  );
  const historicalRecord = structuredClone(currentBundle.record);
  historicalRecord.capturedAt = "2026-08-22T02:00:00Z";
  historicalRecord.artifacts[0].sha256 = createHash("sha256")
    .update(historicalReportBytes)
    .digest("hex");
  const historicalBundle = {
    record: sealRecord(historicalRecord),
    reportBytes: historicalReportBytes,
  };
  const evidenceDir = mkdtempSync(path.join(tmpdir(), "device-qualification-inventory-"));
  const writeBundle = (name, bundle) => {
    const bundleDir = path.join(evidenceDir, name);
    mkdirSync(bundleDir);
    writeFileSync(
      path.join(bundleDir, "evidence.json"),
      `${JSON.stringify(bundle.record)}\n`,
      "utf8",
    );
    writeFileSync(path.join(bundleDir, "execution-report.json"), bundle.reportBytes);
  };

  try {
    writeBundle("historical-v1", historicalBundle);
    writeBundle("current-v2", currentBundle);
    const records = loadValidatedEvidenceRecords(evidenceDir, {
      fixtureMode: false,
      workflowCatalog: context.workflowCatalog,
      targets: context.targets,
    });

    assert.deepEqual(records.map((record) => record.runId), [currentBundle.record.runId]);
    assert.throws(
      () => validateEvidenceBundle(historicalBundle, context),
      /execution report schemaVersion must be 2/i,
    );
  } finally {
    removeTempTree(evidenceDir);
  }
});

test("inventory does not skip malformed, unbound, or forged v1 history", () => {
  const context = syntheticContext();
  const sourceBundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  const evidenceDir = mkdtempSync(path.join(tmpdir(), "device-qualification-inventory-"));
  const writeBundle = (name, bundle) => {
    const bundleDir = path.join(evidenceDir, name);
    mkdirSync(bundleDir);
    writeFileSync(
      path.join(bundleDir, "evidence.json"),
      `${JSON.stringify(bundle.record)}\n`,
      "utf8",
    );
    writeFileSync(path.join(bundleDir, "execution-report.json"), bundle.reportBytes);
  };
  const inventory = () =>
    loadValidatedEvidenceRecords(evidenceDir, {
      fixtureMode: false,
      workflowCatalog: context.workflowCatalog,
      targets: context.targets,
    });

  try {
    const malformedBytes = Buffer.from("{", "utf8");
    const malformedRecord = structuredClone(sourceBundle.record);
    malformedRecord.capturedAt = "2026-08-22T02:00:00Z";
    malformedRecord.artifacts[0].sha256 = createHash("sha256")
      .update(malformedBytes)
      .digest("hex");
    writeBundle("malformed", {
      record: sealRecord(malformedRecord),
      reportBytes: malformedBytes,
    });
    assert.throws(inventory, /execution report JSON is invalid/i);

    rmSync(evidenceDir, { recursive: true, force: true });
    mkdirSync(evidenceDir, { recursive: true });
    const digestMismatchBytes = Buffer.from(
      '{"schema":"emuchef.execution-report","schemaVersion":1}\n',
      "utf8",
    );
    writeBundle("digest-mismatch", {
      record: sourceBundle.record,
      reportBytes: digestMismatchBytes,
    });
    assert.throws(inventory, /execution report digest does not match/i);

    rmSync(evidenceDir, { recursive: true, force: true });
    mkdirSync(evidenceDir, { recursive: true });
    const legacyBytes = Buffer.from(
      '{"schema":"emuchef.execution-report","schemaVersion":1}\n',
      "utf8",
    );
    const forgedRecord = structuredClone(sourceBundle.record);
    forgedRecord.capturedAt = "2026-08-22T02:00:00Z";
    forgedRecord.artifacts[0].sha256 = createHash("sha256")
      .update(legacyBytes)
      .digest("hex");
    const forgedBundle = { record: sealRecord(forgedRecord), reportBytes: legacyBytes };
    forgedBundle.record.runId = `qualification-run-sha256:${"f".repeat(64)}`;
    forgedBundle.record.recordDigest = evidenceRecordDigest(forgedBundle.record);
    writeBundle("forged", forgedBundle);
    assert.throws(inventory, /runId does not match canonical qualification run identity/i);

    rmSync(evidenceDir, { recursive: true, force: true });
    mkdirSync(evidenceDir, { recursive: true });
    const duplicateHistorical = structuredClone(forgedBundle);
    duplicateHistorical.record = structuredClone(sourceBundle.record);
    duplicateHistorical.record.capturedAt = "2026-08-22T02:00:00Z";
    duplicateHistorical.record.artifacts[0].sha256 = createHash("sha256")
      .update(legacyBytes)
      .digest("hex");
    duplicateHistorical.record = sealRecord(duplicateHistorical.record);
    duplicateHistorical.reportBytes = legacyBytes;
    const duplicateCurrent = structuredClone(sourceBundle);
    duplicateCurrent.record.runId = duplicateHistorical.record.runId;
    duplicateCurrent.record.recordDigest = evidenceRecordDigest(duplicateCurrent.record);
    writeBundle("a-historical", duplicateHistorical);
    writeBundle("b-current", duplicateCurrent);
    assert.throws(inventory, /duplicate run identity/i);
  } finally {
    removeTempTree(evidenceDir);
  }
});

test("canonical evidence rejects malformed execution report JSON", () => {
  const source = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  const reportBytes = Buffer.from("{\n", "utf8");

  assert.throws(
    () =>
      validateEvidenceBundle(
        bindReportBytes(source, reportBytes),
        syntheticContext(),
      ),
    /execution report JSON is invalid/i,
  );
});

test("canonical evidence rejects incomplete execution report v2 envelopes", () => {
  const source = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  const reportBytes = Buffer.from(
    '{"schema":"emuchef.execution-report","schemaVersion":2}\n',
    "utf8",
  );

  assert.throws(
    () =>
      validateEvidenceBundle(
        bindReportBytes(source, reportBytes),
        syntheticContext(),
      ),
    /execution report fields must be exactly/i,
  );
});

function restrictedExecutionBundle(
  updateReport = () => {},
  updateRecord = () => {},
) {
  const sourceBundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  const reviewedPlanId = "konkr-restricted-test";
  const reviewedPlanDigest = "a".repeat(64);
  const report = ordinaryExecutionReportV2(sourceBundle);
  report.plan = {
    planId: reviewedPlanId,
    planDigest: reviewedPlanDigest,
  };
  report.runtime.executionRestriction = {
    enabled: true,
    buildIdentity: structuredClone(sourceBundle.record.fingerprint.emuchefBuild),
    runtimeGeneration: 1,
    strategy: "adb_shell",
    rootShell: false,
    appDataWrite: false,
  };
  report.execution.recipes = [
    {
      recipeId: "app.retroarch.provision",
      name: "RetroArch provisioning",
      description: null,
      status: "succeeded",
      steps: [
        {
          recipeId: "app.retroarch.provision",
          stepId: "step.retroarch.provision",
          name: "Install RetroArch",
          note: null,
          status: "succeeded",
          message: null,
        },
      ],
    },
    {
      recipeId: "feature.copy_bios",
      name: "Copy BIOS",
      description: null,
      status: "succeeded",
      steps: [
        {
          recipeId: "feature.copy_bios",
          stepId: "step.copy.bios",
          name: "Copy BIOS files",
          note: null,
          status: "succeeded",
          message: null,
        },
        {
          recipeId: "feature.copy_bios",
          stepId: "step.optional.bios",
          name: "Optional BIOS check",
          note: null,
          status: "skipped",
          message: null,
        },
      ],
    },
  ];
  report.qualification = {
      claim: "restricted_unprivileged",
      attemptId: "qualification-attempt-0001",
      executionMode: "real",
      strategy: "adb_unprivileged",
      restriction: "privileged_execution_disabled",
      observedRootState: sourceBundle.record.fingerprint.rootState,
      effectiveCapabilities: ["app_install"],
      buildIdentity: structuredClone(sourceBundle.record.fingerprint.emuchefBuild),
      deviceTargetId: sourceBundle.record.deviceTarget.id,
      deviceProfileId: sourceBundle.record.fingerprint.deviceProfile,
      reviewedPlanId,
      reviewedPlanDigest,
      requiredOperationIds: [
        "app.retroarch.provision",
        "feature.copy_bios",
      ],
      operationOutcomes: [
        { operationId: "app.retroarch.provision", outcome: "executed" },
        { operationId: "feature.copy_bios", outcome: "executed" },
      ],
  };
  updateReport(report);
  const reportBytes = Buffer.from(`${JSON.stringify(report)}\n`, "utf8");
  const record = structuredClone(sourceBundle.record);
  updateRecord(record);
  const bound = bindReportBytes({ ...sourceBundle, record: sealRecord(record) }, reportBytes);
  return bound;
}

test("a restricted claim requires a digest-bound real unprivileged ADB attempt", () => {
  const bundle = restrictedExecutionBundle();
  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("a restricted claim requires its verified runtime restriction", () => {
  const invalidReports = [
    ["missing handshake", (report) => delete report.runtime.executionRestriction],
    [
      "disabled restriction",
      (report) => {
        report.runtime.executionRestriction.enabled = false;
      },
    ],
    [
      "different strategy",
      (report) => {
        report.runtime.executionRestriction.strategy = "direct_root";
      },
    ],
    [
      "root shell enabled",
      (report) => {
        report.runtime.executionRestriction.rootShell = true;
      },
    ],
    [
      "app data write enabled",
      (report) => {
        report.runtime.executionRestriction.appDataWrite = true;
      },
    ],
  ];

  for (const [label, updateReport] of invalidReports) {
    assert.throws(
      () =>
        validateEvidenceBundle(
          restrictedExecutionBundle(updateReport),
          syntheticContext(),
        ),
      /restricted qualification requires the verified runtime restriction/i,
      label,
    );
  }
});

test("restricted runtime identity must match the qualification claim", () => {
  const bundle = restrictedExecutionBundle((report) => {
    report.runtime.executionRestriction.buildIdentity.appVersion = "0.2.0";
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /restricted qualification runtime build identity does not match its claim/i,
  );
});

test("ordinary execution reports remain valid without a restricted claim", () => {
  const bundle = ordinaryExecutionBundle();
  const report = JSON.parse(bundle.reportBytes.toString("utf8"));

  assert.equal(report.qualification, undefined);
  assert.doesNotThrow(() => validateEvidenceBundle(bundle, syntheticContext()));
});

test("restricted reports reject missing labels, simulation, and privileged strategies", () => {
  const invalidReports = [
    ["missing claim", (report) => delete report.qualification.claim],
    [
      "incorrect report schema",
      (report) => {
        report.schema = "unknown.execution-report";
      },
    ],
    [
      "incorrect claim",
      (report) => {
        report.qualification.claim = "unrestricted";
      },
    ],
    [
      "missing attempt identity",
      (report) => {
        report.qualification.attemptId = "";
      },
    ],
    [
      "simulation mode",
      (report) => {
        report.qualification.executionMode = "simulation";
      },
    ],
    [
      "simulated execution",
      (report) => {
        report.execution.simulated = true;
      },
    ],
    [
      "simulated build identity",
      (report) => {
        report.qualification.buildIdentity.realExecutionEnabled = false;
      },
    ],
    [
      "direct root strategy",
      (report) => {
        report.qualification.strategy = "direct_root";
      },
    ],
    [
      "OEM strategy",
      (report) => {
        report.qualification.strategy = "oem_assisted";
      },
    ],
    [
      "missing restriction label",
      (report) => {
        delete report.qualification.restriction;
      },
    ],
    [
      "root capability restored",
      (report) => {
        report.qualification.effectiveCapabilities.push("root_shell");
      },
    ],
    [
      "app data write restored",
      (report) => {
        report.qualification.effectiveCapabilities.push("app_data_write");
      },
    ],
  ];

  for (const [label, updateReport] of invalidReports) {
    const bundle = restrictedExecutionBundle(updateReport);
    assert.throws(
      () => validateEvidenceBundle(bundle, syntheticContext()),
      /execution report schema|restricted|qualification|strategy|simulat|capabilit/i,
      label,
    );
  }
});

test("restricted reports bind build, target, profile, and reviewed plan", () => {
  const invalidReports = [
    [
      "build identity",
      (report) => {
        report.qualification.buildIdentity.gitCommit = "f".repeat(40);
      },
    ],
    [
      "report app version",
      (report) => {
        report.app.version = "0.0.0";
      },
    ],
    [
      "device target",
      (report) => {
        report.qualification.deviceTargetId = "device-target-sha256:wrong";
      },
    ],
    [
      "device profile",
      (report) => {
        report.qualification.deviceProfileId = "another-profile";
      },
    ],
    [
      "observed root state",
      (report) => {
        report.qualification.observedRootState = "unknown";
      },
    ],
    [
      "plan identity",
      (report) => {
        report.qualification.reviewedPlanId = "another-plan";
      },
    ],
    [
      "plan digest",
      (report) => {
        report.qualification.reviewedPlanDigest = `sha256:${"b".repeat(64)}`;
      },
    ],
  ];

  for (const [label, updateReport] of invalidReports) {
    const bundle = restrictedExecutionBundle(updateReport);
    assert.throws(
      () => validateEvidenceBundle(bundle, syntheticContext()),
      /restricted|qualification|build|target|profile|plan/i,
      label,
    );
  }
});

test("restricted reports reject executed outcomes without successful recipe steps", () => {
  const bundle = restrictedExecutionBundle((report) => {
    report.execution.recipes[1].steps = [];
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /executed|step|operation/i,
  );
});

test("restricted reports require one terminal outcome for every required operation", () => {
  const invalidReports = [
    [
      "missing outcome",
      (report) => {
        report.qualification.operationOutcomes.pop();
      },
    ],
    [
      "duplicate outcome",
      (report) => {
        report.qualification.operationOutcomes.push({
          operationId: "app.retroarch.provision",
          outcome: "executed",
        });
      },
    ],
    [
      "unexpected operation",
      (report) => {
        report.qualification.operationOutcomes[0].operationId =
          "another-operation";
      },
    ],
    [
      "nonterminal outcome",
      (report) => {
        report.qualification.operationOutcomes[0].outcome = "pending";
      },
    ],
[
"blocked outcome cannot pass",
(report) => {
report.qualification.operationOutcomes[0].outcome = "blocked";
},
],
  ];

  for (const [label, updateReport] of invalidReports) {
    const bundle = restrictedExecutionBundle(updateReport);
    assert.throws(
      () => validateEvidenceBundle(bundle, syntheticContext()),
      /restricted|qualification|operation|outcome/i,
      label,
    );
  }
});

test("restricted reports cannot omit a required workflow recipe and its outcome", () => {
  const bundle = restrictedExecutionBundle((report) => {
    report.qualification.requiredOperationIds.pop();
    report.qualification.operationOutcomes.pop();
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /workflow|required operation/i,
  );
});

test("a restricted report status cannot change without matching its valid qualification outcome", () => {
  const bundle = restrictedExecutionBundle((report) => {
    report.execution.status = "failed";
    report.execution.completion.classification = "failed";
  });

  assert.throws(
    () => validateEvidenceBundle(bundle, syntheticContext()),
    /execution report status|qualification outcome|execution-report observation/i,
  );
});

test("restricted execution status follows native passed and failed outcome semantics", () => {
  const warningBundle = restrictedExecutionBundle((report) => {
    report.execution.status = "succeeded_with_warnings";
    report.execution.completion.classification = "success_with_warnings";
  });
  assert.doesNotThrow(() =>
    validateEvidenceBundle(warningBundle, syntheticContext()),
  );

  const failedExecutionBundle = restrictedExecutionBundle(
    (report) => {
      report.execution.status = "failed";
      report.execution.completion.classification = "failed";
    },
    (record) => {
      record.qualificationOutcome = "failed";
      record.automatedObservations[0].outcome = "failed";
      record.limitations = ["execution-report failed"];
    },
  );
  assert.doesNotThrow(() =>
    validateEvidenceBundle(failedExecutionBundle, syntheticContext()),
  );

  const context = syntheticContext();
  context.workflowCatalog.workflows
    .find((workflow) => workflow.id === "retroarch-plus-bios")
    .humanCheckpoints.push({
      id: "device_behavior_verified",
      instruction: "Verify the device behavior.",
      fact: "The device behavior was verified.",
      allowedOutcomes: ["pass", "fail", "unable_to_verify"],
      required: false,
    });
  const failedCheckpointBundle = restrictedExecutionBundle(
    () => {},
    (record) => {
      record.qualificationOutcome = "failed";
      record.humanCheckpoints.push({
        checkpointId: "device_behavior_verified",
        outcome: "fail",
        observedAt: "2026-11-18T20:00:00Z",
      });
      record.limitations.push("device_behavior_verified failed");
    },
  );
  assert.doesNotThrow(() =>
    validateEvidenceBundle(failedCheckpointBundle, context),
  );
});

test("a changed restricted report is rejected against its stored digest", () => {
  const bundle = restrictedExecutionBundle();
  const changedBytes = Buffer.from(bundle.reportBytes);
  changedBytes[0] ^= 1;
  assert.throws(
    () =>
      validateEvidenceBundle(
        { ...bundle, reportBytes: changedBytes },
        syntheticContext(),
      ),
    /execution report digest/i,
  );
});

test("a retroarch run cannot validate as valid when its required human checkpoint is missing", () => {
  const bundle = loadEvidenceBundle(
    path.join(FIXTURES, "evidence-valid/passing-retroarch-bios"),
  );
  const unsealed = structuredClone(bundle.record);
  delete unsealed.runId;
  delete unsealed.recordDigest;
  delete unsealed.fingerprintDigest;
  unsealed.humanCheckpoints = [];
  const resealed = sealEvidenceRecord(unsealed);
  assert.throws(
    () =>
      validateEvidenceBundle(
        { ...bundle, record: resealed },
        syntheticContext(),
      ),
    /missing required human checkpoint/i,
  );
});

test("Rust-produced target and report-only candidates pass Node recorders", () => {
  const targetRepo = createTempQualificationRepo();
  const runRepo = createTempQualificationRepo();
  const interopRoot = mkdtempSync(path.join(tmpdir(), "candidate-interop-"));
  try {
    const targetCandidate = targetRegistrationCandidateForRepo(targetRepo);
    const runCandidate = runCandidateForRepo(runRepo);
    const reportPath = path.join(interopRoot, "execution-report.json");
    const inputPath = path.join(interopRoot, "input.json");
    const outputPath = path.join(interopRoot, "output.json");
    writeFileSync(reportPath, REPORT_BYTES);
    writeFileSync(
      inputPath,
      JSON.stringify([
        {
          key: "targetRegistration",
          kind: "target_registration",
          repoRoot: targetRepo,
          payload: targetCandidate,
        },
        {
          key: "reportOnlyRun",
          kind: "qualification_run",
          repoRoot: runRepo,
          payload: runCandidate,
          reportPath,
        },
      ]),
    );
  const inheritedTauriConfig = process.env.TAURI_CONFIG
    ? JSON.parse(process.env.TAURI_CONFIG)
    : {};

    execFileSync(
      process.env.CARGO ?? "cargo",
      [
        "test",
        "--manifest-path",
        path.join(REPO_ROOT, "apps/emuchef-app/src-tauri/Cargo.toml"),
        "qualification_repository::tests::rust_candidate_interop_fixture",
        "--",
        "--ignored",
      ],
      {
        cwd: REPO_ROOT,
        env: {
          ...process.env,
        TAURI_CONFIG: JSON.stringify({
          ...inheritedTauriConfig,
        build: {
          ...(inheritedTauriConfig.build ?? {}),
          frontendDist: "https://example.invalid/",
        },
          bundle: {
            ...(inheritedTauriConfig.bundle ?? {}),
            externalBin: [],
          },
        }),
          EMUCHEF_CANDIDATE_INTEROP_INPUT: inputPath,
          EMUCHEF_CANDIDATE_INTEROP_OUTPUT: outputPath,
        },
        stdio: "pipe",
      },
    );

    const handles = JSON.parse(readFileSync(outputPath, "utf8"));
    const recorderFailures = [];
    const targetEnvelopePath = path.join(
      targetRepo,
      ".emuchef_runtime/qualification-candidates",
      handles.targetRegistration,
      "candidate.json",
    );
    const targetEnvelope = JSON.parse(readFileSync(targetEnvelopePath, "utf8"));
    let registeredTarget;
    try {
      registeredTarget = registerQualificationTargetCandidate(
        handles.targetRegistration,
        { repoRoot: targetRepo },
      );
    } catch (error) {
      recorderFailures.push(`target registration: ${error.message}`);
    }

    const runEnvelopePath = path.join(
      runRepo,
      ".emuchef_runtime/qualification-candidates",
      handles.reportOnlyRun,
      "candidate.json",
    );
    const runEnvelope = JSON.parse(readFileSync(runEnvelopePath, "utf8"));
    let record;
    try {
      record = recordQualificationRunCandidate(handles.reportOnlyRun, {
        repoRoot: runRepo,
      });
    } catch (error) {
      recorderFailures.push(`report-only recording: ${error.message}`);
    }
    assert.deepEqual(recorderFailures, []);
    assert.equal(targetEnvelope.payload.candidateSchemaVersion, 1);
    assert.equal(Object.hasOwn(targetEnvelope, "romMergeEvidence"), false);
    assert.equal(runEnvelope.payload.candidateSchemaVersion, 1);
    assert.equal(Object.hasOwn(runEnvelope, "romMergeEvidence"), false);
    assert.equal(registeredTarget.id, deviceTargetId(targetCandidate.target));
    assert.equal(record.qualificationOutcome, "passed");
    assert.deepEqual(
      record.artifacts.map(({ id }) => id),
      ["execution-report"],
    );
  } finally {
    removeTempTree(targetRepo);
    removeTempTree(runRepo);
    removeTempTree(interopRoot);
  }
});

test("report-only recording accepts a legacy null ROM Merge metadata field", () => {
  const repoRoot = createTempQualificationRepo();
  const candidate = runCandidateForRepo(repoRoot, {
    candidateId: "qualification-candidate-11112222333344445555666677778888",
  });
  try {
    writeCandidateFixture(repoRoot, candidate, REPORT_BYTES);
    const candidatePath = path.join(
      repoRoot,
      ".emuchef_runtime/qualification-candidates",
      candidate.candidateId,
      "candidate.json",
    );
    const envelope = JSON.parse(readFileSync(candidatePath, "utf8"));
    envelope.romMergeEvidence = null;
    writeFileSync(candidatePath, `${JSON.stringify(envelope, null, 2)}\n`);

    const record = recordQualificationRunCandidate(candidate.candidateId, {
      repoRoot,
    });
    assert.deepEqual(
      record.artifacts.map(({ id }) => id),
      ["execution-report"],
    );
  } finally {
    removeTempTree(repoRoot);
  }
});

test("a null ROM Merge metadata field does not authorize an unreferenced artifact file", () => {
  const repoRoot = createTempQualificationRepo();
  const candidate = runCandidateForRepo(repoRoot, {
    candidateId: "qualification-candidate-9999aaaabbbbccccddddeeeeffff0000",
  });
  try {
    writeCandidateFixture(repoRoot, candidate, REPORT_BYTES);
    const candidateDirectory = path.join(
      repoRoot,
      ".emuchef_runtime/qualification-candidates",
      candidate.candidateId,
    );
    const candidatePath = path.join(candidateDirectory, "candidate.json");
    const envelope = JSON.parse(readFileSync(candidatePath, "utf8"));
    envelope.romMergeEvidence = null;
    writeFileSync(candidatePath, `${JSON.stringify(envelope, null, 2)}\n`);
    writeFileSync(
      path.join(candidateDirectory, ROM_MERGE_EVIDENCE_PATH),
      "unreferenced evidence\n",
    );

    assert.throws(
      () =>
        recordQualificationRunCandidate(candidate.candidateId, { repoRoot }),
      /ROM Merge evidence file is not declared by candidate metadata/i,
    );
  } finally {
    removeTempTree(repoRoot);
  }
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
    registerQualificationTargetCandidate(
      targetCandidate.candidateId,
      targetPaths,
    );
    const registeredBytes = readFileSync(
      path.join(
        emptyRepo,
        "docs/testing/device-qualification/device-targets.json",
      ),
      "utf8",
    );
    assert.throws(
      () =>
        registerQualificationTargetCandidate(
          targetCandidate.candidateId,
          targetPaths,
        ),
      /already exists|immutable|duplicate/i,
    );
    assert.equal(
      readFileSync(
        path.join(
          emptyRepo,
          "docs/testing/device-qualification/device-targets.json",
        ),
        "utf8",
      ),
      registeredBytes,
    );
  } finally {
    removeTempTree(emptyRepo);
  }

  const seededRepo = createTempQualificationRepo({
    deviceTargetsSource: path.join(
      FIXTURES,
      "definitions-valid/device-targets.json",
    ),
  });
  try {
    const runCandidate = runCandidateForRepo(seededRepo);
    writeCandidateFixture(seededRepo, runCandidate, REPORT_BYTES);
    const recordPaths = {
      repoRoot: seededRepo,
      fsOps: undefined,
    };
    const firstRecord = recordQualificationRunCandidate(
      runCandidate.candidateId,
      recordPaths,
    );
    const evidenceRoot = path.join(
      seededRepo,
      "docs/testing/device-qualification/evidence",
    );
    const matrixPath = path.join(
      seededRepo,
      "docs/qualification/device-qualification-matrix.md",
    );
    const before = snapshotTree(evidenceRoot);
    const beforeMatrix = readFileSync(matrixPath, "utf8");
    assert.ok(firstRecord.runId);
    assert.throws(
      () =>
        recordQualificationRunCandidate(runCandidate.candidateId, recordPaths),
      /already exists|immutable|duplicate/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), before);
    assert.equal(readFileSync(matrixPath, "utf8"), beforeMatrix);
  } finally {
    removeTempTree(seededRepo);
  }
});

test("canonical run promotion rejects candidate-local poison and audit-only markers before mutation", () => {
  const cases = [
    { marker: "session-poison.json", shape: "file" },
    { marker: "session-audit-only", shape: "file" },
    { marker: "session-poison.json", shape: "symlink" },
    { marker: "session-audit-only", shape: "directory" },
  ];
  for (const { marker, shape } of cases) {
    const repoRoot = createTempQualificationRepo({
      deviceTargetsSource: path.join(
        FIXTURES,
        "definitions-valid/device-targets.json",
      ),
    });
    try {
      const candidate = runCandidateForRepo(repoRoot);
      writeCandidateFixture(repoRoot, candidate, REPORT_BYTES);
      const candidateDirectory = path.join(
        repoRoot,
        ".emuchef_runtime/qualification-candidates",
        candidate.candidateId,
      );
      const markerPath = path.join(candidateDirectory, marker);
      if (shape === "file")
        writeFileSync(markerPath, "candidate is non-promotable\n", "utf8");
      if (shape === "symlink") symlinkSync("candidate.json", markerPath);
      if (shape === "directory") mkdirSync(markerPath);

      const evidenceRoot = path.join(
        repoRoot,
        "docs/testing/device-qualification/evidence",
      );
      const matrixPath = path.join(
        repoRoot,
        "docs/qualification/device-qualification-matrix.md",
      );
      const evidenceBefore = snapshotTree(evidenceRoot);
      const matrixBefore = readFileSync(matrixPath, "utf8");

      assert.throws(
        () =>
          recordQualificationRunCandidate(candidate.candidateId, { repoRoot }),
        /non-promotable marker|candidate.*(?:non-promotable|marker|symlink|regular file)/i,
      );

      assert.deepEqual(snapshotTree(evidenceRoot), evidenceBefore);
      assert.equal(readFileSync(matrixPath, "utf8"), matrixBefore);
    } finally {
      removeTempTree(repoRoot);
    }
  }
});

test("the canonical recorder rejects an incomplete provisional run candidate without mutation", () => {
  const repoRoot = createTempQualificationRepo({
    deviceTargetsSource: path.join(
      FIXTURES,
      "definitions-valid/device-targets.json",
    ),
  });
  try {
    const candidate = {
      candidateSchemaVersion: 1,
      candidateId: "qualification-candidate-1234567890abcdef1234567890abcdef",
      kind: "qualification_run",
      capturedAt: "2026-08-23T12:30:00Z",
      build: buildMaterialIdentity({ repoRoot, requireClean: false }),
    };
    writeCandidateFixture(repoRoot, candidate);
    const evidenceRoot = path.join(
      repoRoot,
      "docs/testing/device-qualification/evidence",
    );
    const before = snapshotTree(evidenceRoot);

    assert.throws(
      () =>
        recordQualificationRunCandidate(candidate.candidateId, { repoRoot }),
      /qualification_run candidate fields must be exactly/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), before);
  } finally {
    removeTempTree(repoRoot);
  }
});

test("recording a run refuses a destination reserved by another invocation without deleting it", () => {
  const repoRoot = createTempQualificationRepo({
    deviceTargetsSource: path.join(
      FIXTURES,
      "definitions-valid/device-targets.json",
    ),
  });
  try {
    const runCandidate = runCandidateForRepo(repoRoot, {
      candidateId: "qualification-candidate-11112222333344445555666677778888",
    });
    writeCandidateFixture(repoRoot, runCandidate, REPORT_BYTES);
    const evidenceRoot = path.join(
      repoRoot,
      "docs/testing/device-qualification/evidence",
    );
    const matrixPath = path.join(
      repoRoot,
      "docs/qualification/device-qualification-matrix.md",
    );
    const beforeMatrix = readFileSync(matrixPath, "utf8");
    const raceMarker = "reservation-owner.txt";
    let competitorCreated = false;
    const isCanonicalBundlePath = (targetPath) =>
      targetPath.startsWith(`${evidenceRoot}${path.sep}`) &&
      path.basename(targetPath).startsWith("qualification-run-sha256:") &&
      !path.basename(targetPath).includes(".tmp-");
    const createCompetingBundle = (targetPath) => {
      if (competitorCreated) return;
      competitorCreated = true;
      mkdirSync(targetPath, { recursive: false });
      writeFileSync(
        path.join(targetPath, raceMarker),
        "competing invocation\n",
        "utf8",
      );
      const error = new Error(
        "competing invocation reserved destination first",
      );
      error.code = "EEXIST";
      throw error;
    };
    assert.throws(
      () =>
        recordQualificationRunCandidate(runCandidate.candidateId, {
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
    removeTempTree(repoRoot);
  }
});

test("tampered existing evidence bundle report blocks both promotions before canonical mutation", () => {
  const narrowedTargets = [
    readJson("definitions-valid/device-targets.json").targets[0],
  ];
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
    const targetRegistryPath = path.join(
      targetRepo,
      "docs/testing/device-qualification/device-targets.json",
    );
    const targetMatrixPath = path.join(
      targetRepo,
      "docs/qualification/device-qualification-matrix.md",
    );
    const beforeRegistry = readFileSync(targetRegistryPath, "utf8");
    const beforeMatrix = readFileSync(targetMatrixPath, "utf8");
    assert.throws(
      () =>
        registerQualificationTargetCandidate(targetCandidate.candidateId, {
          repoRoot: targetRepo,
        }),
      /execution report digest/i,
    );
    assert.equal(readFileSync(targetRegistryPath, "utf8"), beforeRegistry);
    assert.equal(readFileSync(targetMatrixPath, "utf8"), beforeMatrix);
  } finally {
    removeTempTree(targetRepo);
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
    const evidenceRoot = path.join(
      runRepo,
      "docs/testing/device-qualification/evidence",
    );
    const matrixPath = path.join(
      runRepo,
      "docs/qualification/device-qualification-matrix.md",
    );
    const beforeEvidence = snapshotTree(evidenceRoot);
    const beforeMatrix = readFileSync(matrixPath, "utf8");
    assert.throws(
      () =>
        recordQualificationRunCandidate(runCandidate.candidateId, {
          repoRoot: runRepo,
        }),
      /execution report digest/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), beforeEvidence);
    assert.equal(readFileSync(matrixPath, "utf8"), beforeMatrix);
  } finally {
    removeTempTree(runRepo);
  }
});

test("target registration restores registry and matrix bytes when matrix replacement fails", () => {
  const repoRoot = createTempQualificationRepo();
  try {
    const targetCandidate = targetRegistrationCandidateForRepo(repoRoot, {
      candidateId: "qualification-candidate-12344321123443211234432112344321",
    });
    writeCandidateFixture(repoRoot, targetCandidate);
    const registryPath = path.join(
      repoRoot,
      "docs/testing/device-qualification/device-targets.json",
    );
    const matrixPath = path.join(
      repoRoot,
      "docs/qualification/device-qualification-matrix.md",
    );
    const beforeRegistry = readFileSync(registryPath, "utf8");
    const beforeMatrix = readFileSync(matrixPath, "utf8");
    let renameCount = 0;
    assert.throws(
      () =>
        registerQualificationTargetCandidate(targetCandidate.candidateId, {
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
    removeTempTree(repoRoot);
  }
});

test("recording a run rolls back the newly created evidence bundle when matrix replacement fails", () => {
  const repoRoot = createTempQualificationRepo({
    deviceTargetsSource: path.join(
      FIXTURES,
      "definitions-valid/device-targets.json",
    ),
  });
  try {
    const runCandidate = runCandidateForRepo(repoRoot, {
      candidateId: "qualification-candidate-00112233445566778899aabbccddeeff",
    });
    writeCandidateFixture(repoRoot, runCandidate, REPORT_BYTES);
    const evidenceRoot = path.join(
      repoRoot,
      "docs/testing/device-qualification/evidence",
    );
    const registryPath = path.join(
      repoRoot,
      "docs/testing/device-qualification/device-targets.json",
    );
    const beforeEvidence = snapshotTree(evidenceRoot);
    const beforeRegistry = readFileSync(registryPath, "utf8");
    let renameCount = 0;
    assert.throws(
      () =>
        recordQualificationRunCandidate(runCandidate.candidateId, {
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
    removeTempTree(repoRoot);
  }
});

test("canonical promotion rejects a symlinked candidate root before reading it", () => {
  const repoRoot = createTempQualificationRepo();
  const outside = mkdtempSync(
    path.join(tmpdir(), "device-qualification-outside-"),
  );
  try {
    rmSync(path.join(repoRoot, ".emuchef_runtime"), {
      recursive: true,
      force: true,
    });
    symlinkSync(outside, path.join(repoRoot, ".emuchef_runtime"));
    const candidate = targetRegistrationCandidateForRepo(repoRoot);
    assert.throws(
      () =>
        registerQualificationTargetCandidate(candidate.candidateId, {
          repoRoot,
        }),
      /symlink|symbolic|candidate root/i,
    );
    assert.deepEqual(readdirSync(outside), []);
  } finally {
    removeTempTree(repoRoot);
    removeTempTree(outside);
  }
});

test("canonical promotion rejects a symlinked candidate file before mutation", () => {
  const repoRoot = createTempQualificationRepo();
  const outside = mkdtempSync(
    path.join(tmpdir(), "device-qualification-outside-"),
  );
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
    const registryPath = path.join(
      repoRoot,
      "docs/testing/device-qualification/device-targets.json",
    );
    const before = readFileSync(registryPath, "utf8");
    assert.throws(
      () =>
        registerQualificationTargetCandidate(candidate.candidateId, {
          repoRoot,
        }),
      /symlink|symbolic|regular|candidate/i,
    );
    assert.equal(readFileSync(registryPath, "utf8"), before);
    assert.ok(existsSync(outsidePath));
  } finally {
    removeTempTree(repoRoot);
    removeTempTree(outside);
  }
});

test("canonical run promotion rejects a symlinked execution report before mutation", () => {
  const repoRoot = createTempQualificationRepo({
    deviceTargetsSource: path.join(
      FIXTURES,
      "definitions-valid/device-targets.json",
    ),
  });
  const outside = mkdtempSync(
    path.join(tmpdir(), "device-qualification-outside-"),
  );
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
    const evidenceRoot = path.join(
      repoRoot,
      "docs/testing/device-qualification/evidence",
    );
    const before = snapshotTree(evidenceRoot);
    assert.throws(
      () =>
        recordQualificationRunCandidate(candidate.candidateId, { repoRoot }),
      /symlink|symbolic|regular|report/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), before);
  } finally {
    removeTempTree(repoRoot);
    removeTempTree(outside);
  }
});

test("validateEvidenceSchemaContract rejects qualification build identity schema drift", () => {
  const schema = JSON.parse(
    readFileSync(
      path.join(
        REPO_ROOT,
        "docs/testing/device-qualification/evidence-schema.json",
      ),
      "utf8",
    ),
  );

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
  driftedConstraint.$defs.qualificationBuildIdentity.properties.gitCommit.pattern =
    "^[0-9a-f]{7}$";
  assert.throws(
    () => validateEvidenceSchemaContract(driftedConstraint),
    /qualificationBuildIdentity/i,
  );
});

test("a passing evidence fixture validates with its stored digests", () => {
  assert.doesNotThrow(() =>
    validateEvidenceRecord(
      recordFor("evidence-valid/passing-retroarch-bios"),
      syntheticContext(),
    ),
  );
});

test("a valid failed evidence fixture validates", () => {
  assert.doesNotThrow(() =>
    validateEvidenceRecord(
      recordFor("evidence-valid/failed-retroarch-bios"),
      syntheticContext(),
    ),
  );
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
  assert.doesNotThrow(() =>
    validateEvidenceRecord(sealRecord(record), syntheticContext()),
  );
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

test("a passed record may omit an optional automated observation", () => {
  const context = syntheticContext();
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  const workflow = context.workflowCatalog.workflows.find(
    (candidate) => candidate.id === record.workflowId,
  );
  assert.ok(workflow, "fixture workflow should be present");
  workflow.automatedObservations.push({
    id: "future-device-audit",
    required: false,
  });

  assert.doesNotThrow(() =>
    validateEvidenceRecord(sealRecord(record), context),
  );
});

test("a failed required automated observation produces a valid failed record", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  record.runId = `qualification-run-sha256:${"8".repeat(64)}`;
  record.capturedAt = "2026-08-21T03:00:00Z";
  record.automatedObservations[0].outcome = "failed";
  record.qualificationOutcome = "failed";
  record.limitations = ["execution-report failed"];
  assert.doesNotThrow(() =>
    validateEvidenceRecord(sealRecord(record), syntheticContext()),
  );
});

test("a failed required human checkpoint produces a valid failed record", () => {
  assert.doesNotThrow(() =>
    validateEvidenceRecord(checkpointGatedRecord(), syntheticContext()),
  );
});

test("a required checkpoint unable_to_verify must be an invalid not_observed record", () => {
  const context = syntheticContext();
  const record = checkpointGatedRecord();
  record.runValidity = "invalid";
  record.qualificationOutcome = "not_observed";
  record.automatedObservations = [];
  record.humanCheckpoints[0].outcome = "unable_to_verify";
  record.limitations = [
    "operator could not verify the required device behavior",
  ];
  assert.doesNotThrow(() =>
    validateEvidenceRecord(sealRecord(record), context),
  );

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
  catalog.workflows.find(
    (workflow) => workflow.id === "checkpoint-gated",
  ).humanCheckpoints[0].required = false;
  const record = checkpointGatedRecord();
  record.humanCheckpoints[0].outcome = "unable_to_verify";
  record.qualificationOutcome = "passed";
  record.limitations = [];
  assert.doesNotThrow(() =>
    validateEvidenceRecord(sealRecord(record), {
      workflowCatalog: catalog,
      targets: context.targets,
    }),
  );
});

test("target-wide failures are restricted, required to be failed, and forbidden on invalid runs", () => {
  const context = syntheticContext();
  const failed = recordFor("evidence-valid/failed-retroarch-bios");
  failed.targetWideFailure = "safety_invariant_failed";
  assert.doesNotThrow(() =>
    validateEvidenceRecord(sealRecord(failed), context),
  );

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
  wrongTarget.deviceTarget.id = targetByProfile(
    context.targets,
    SYNTHETIC_AIR_MINI_PROFILE,
  ).id;
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
  wrongVersion.fingerprintDigest = evidenceFingerprintDigest(
    wrongVersion.fingerprint,
  );
  assert.throws(
    () => validateEvidenceRecord(sealRecord(wrongVersion), context),
    /workflow version/,
  );
});

test("fingerprint validation is strict, deterministic, and digest-bound", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  assert.equal(
    evidenceFingerprintDigest(record.fingerprint),
    record.fingerprintDigest,
  );
  const extra = structuredClone(record.fingerprint);
  extra.extra = true;
  assert.throws(
    () => evidenceFingerprintDigest(extra),
    /fields must be exactly/,
  );
  const tampered = structuredClone(record.fingerprint);
  tampered.emuchefBuild = {
    ...tampered.emuchefBuild,
    materialBuildDigest: `sha256:${"0".repeat(64)}`,
  };
  assert.notEqual(
    evidenceFingerprintDigest(tampered),
    record.fingerprintDigest,
  );
});

test("compatibility classifies identical evidence as compatible", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  const workflow = syntheticContext().workflowCatalog.workflows.find(
    (item) => item.id === "retroarch-plus-bios",
  );
  assert.equal(
    classifyCompatibility({
      workflow,
      currentFingerprint: record.fingerprint,
      evidenceFingerprint: record.fingerprint,
    }),
    "compatible",
  );
});

test("every declared compatibility dimension invalidates on change", () => {
  const context = syntheticContext();
  const workflow = context.workflowCatalog.workflows.find(
    (item) => item.id === "retroarch-plus-bios",
  );
  const base = recordFor("evidence-valid/passing-retroarch-bios").fingerprint;
  const cases = [
    [
      "emuchefBuild",
      {
        ...base.emuchefBuild,
        materialBuildDigest: `sha256:${"f".repeat(64)}`,
      },
    ],
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
    assert.equal(
      classifyCompatibility({
        workflow,
        currentFingerprint: base,
        evidenceFingerprint: changed,
      }),
      "invalidating",
      `${field} must invalidate`,
    );
  }
  const authored = structuredClone(base);
  authored.authoredContent = [
    { id: "app.retroarch.provision", sha256: "f".repeat(64) },
    { id: "feature.copy_bios", sha256: "b".repeat(64) },
  ];
  assert.equal(
    classifyCompatibility({
      workflow,
      currentFingerprint: base,
      evidenceFingerprint: authored,
    }),
    "invalidating",
    "authored_content must invalidate",
  );
});

test("workflow compatibility treats git-sha-only build changes as compatible", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios");
  const workflow = syntheticContext().workflowCatalog.workflows.find(
    (item) => item.id === "retroarch-plus-bios",
  );
  const currentFingerprint = record.fingerprint;
  const movedCommit = structuredClone(record.fingerprint);
  movedCommit.emuchefBuild.gitCommit = "f".repeat(40);
  assert.equal(
    classifyCompatibility({
      workflow,
      currentFingerprint,
      evidenceFingerprint: movedCommit,
    }),
    "compatible",
  );
});

test("compatibility ignores dimensions the workflow does not declare", () => {
  const context = syntheticContext();
  const workflow = context.workflowCatalog.workflows.find(
    (item) => item.id === "retroarch-plus-bios",
  );
  const base = recordFor("evidence-valid/passing-retroarch-bios").fingerprint;
  const changed = structuredClone(base);
  changed.connectionType = "usb2";
  assert.equal(
    classifyCompatibility({
      workflow,
      currentFingerprint: base,
      evidenceFingerprint: changed,
    }),
    "compatible",
    "connectionType is not a declared compatibility dimension",
  );

  const narrowWorkflow = structuredClone(workflow);
  narrowWorkflow.compatibilityDimensions = ["workflow_version"];
  const buildChanged = structuredClone(base);
  buildChanged.emuchefBuild = {
    ...buildChanged.emuchefBuild,
    materialBuildDigest: `sha256:${"0".repeat(64)}`,
  };
  assert.equal(
    classifyCompatibility({
      workflow: narrowWorkflow,
      currentFingerprint: base,
      evidenceFingerprint: buildChanged,
    }),
    "compatible",
    "undeclared build change must not invalidate",
  );
});

test("applicability derives from production intent, capabilities, and deferral", () => {
  const context = syntheticContext();
  const retroarch = context.workflowCatalog.workflows.find(
    (item) => item.id === "retroarch-plus-bios",
  );
  const obtainium = context.workflowCatalog.workflows.find(
    (item) => item.id === "obtainium-install",
  );
  const xaniteog = context.workflowCatalog.workflows.find(
    (item) => item.id === "xaniteog-install",
  );
  const checkpointGated = context.workflowCatalog.workflows.find(
    (item) => item.id === "checkpoint-gated",
  );
  const pocketS2 = targetByProfile(
    context.targets,
    SYNTHETIC_POCKET_S2_PROFILE,
  );
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
  const workflow = context.workflowCatalog.workflows.find(
    (item) => item.id === "retroarch-plus-bios",
  );
  const target = targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE);
  const selected = selectCurrentEvidence({
    workflow,
    target,
    currentFingerprint: currentFingerprint(workflow, target),
    records: context.records,
  });
  assert.equal(
    selected.runId,
    context.records.find(
      (record) =>
        record.workflowId === "retroarch-plus-bios" &&
        record.runValidity === "valid",
    ).runId,
  );
});

test("newer invalid evidence never replaces older valid evidence", () => {
  const context = projectionContext();
  const workflow = context.workflowCatalog.workflows.find(
    (item) => item.id === "xaniteog-install",
  );
  const target = targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE);
  const selected = selectCurrentEvidence({
    workflow,
    target,
    currentFingerprint: currentFingerprint(workflow, target),
    records: context.records,
  });
  assert.equal(
    selected.runId,
    context.records.find(
      (record) =>
        record.workflowId === "xaniteog-install" &&
        record.runValidity === "valid",
    ).runId,
  );
  assert.ok(context.records.some((record) => record.runValidity === "invalid"));
});

test("incompatible historical evidence is never selected as current", () => {
  const context = projectionContext();
  const workflow = context.workflowCatalog.workflows.find(
    (item) => item.id === "obtainium-install",
  );
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
  const workflow = context.workflowCatalog.workflows.find(
    (item) => item.id === "retroarch-plus-bios",
  );
  const target = targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE);
  const fingerprint = currentFingerprint(workflow, target);
  const base = recordFor("evidence-valid/passing-retroarch-bios");
  const first = structuredClone(base);
  first.runId = "qualification-run-sha256:" + "f".repeat(64);
  const second = structuredClone(base);
  second.runId = "qualification-run-sha256:" + "e".repeat(64);
  const records = [first, second];
  const selected = selectCurrentEvidence({
    workflow,
    target,
    currentFingerprint: fingerprint,
    records,
  });
  assert.equal(selected.runId, "qualification-run-sha256:" + "f".repeat(64));
  const reversed = selectCurrentEvidence({
    workflow,
    target,
    currentFingerprint: fingerprint,
    records: [second, first],
  });
  assert.equal(reversed.runId, selected.runId);
});

test("workflow state derivation covers all six states", () => {
  const context = projectionContext();
  assert.equal(
    projectionState(
      context,
      targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id,
      "retroarch-plus-bios",
    ).state,
    "qualified",
  );
  assert.equal(
    projectionState(
      context,
      targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id,
      "rom-library-sync",
    ).state,
    "failed",
  );
  assert.equal(
    projectionState(
      context,
      targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id,
      "obtainium-install",
    ).state,
    "stale",
  );
  assert.equal(
    projectionState(
      context,
      targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id,
      "xaniteog-install",
    ).state,
    "deferred",
  );
  assert.equal(
    projectionState(
      context,
      targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id,
      "rom-library-sync",
    ).state,
    "missing",
  );
  assert.equal(
    projectionState(
      context,
      targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id,
      "retroarch-plus-bios",
    ).state,
    "not_applicable",
  );
});

test("a failed workflow does not erase unrelated qualified evidence", () => {
  const tier = deriveDeviceSupportTier([
    {
      workflowId: "retroarch-plus-bios",
      applicability: "required",
      state: "qualified",
      targetWideFailure: null,
    },
    {
      workflowId: "rom-library-sync",
      applicability: "required",
      state: "failed",
      targetWideFailure: null,
    },
  ]);
  assert.equal(tier, "limited");
});

test("an explicit target-wide safety failure makes the target unqualified", () => {
  const tier = deriveDeviceSupportTier([
    {
      workflowId: "retroarch-plus-bios",
      applicability: "required",
      state: "qualified",
      targetWideFailure: "safety_invariant_failed",
    },
  ]);
  assert.equal(tier, "unqualified");
});

test("device support tier requires every required workflow to be qualified", () => {
  const qualified = deriveDeviceSupportTier([
    {
      workflowId: "retroarch-plus-bios",
      applicability: "required",
      state: "qualified",
      targetWideFailure: null,
    },
    {
      workflowId: "rom-library-sync",
      applicability: "required",
      state: "qualified",
      targetWideFailure: null,
    },
  ]);
  assert.equal(qualified, "qualified");

  const noneQualified = deriveDeviceSupportTier([
    {
      workflowId: "retroarch-plus-bios",
      applicability: "required",
      state: "failed",
      targetWideFailure: null,
    },
    {
      workflowId: "rom-library-sync",
      applicability: "required",
      state: "missing",
      targetWideFailure: null,
    },
  ]);
  assert.equal(noneQualified, "unqualified");

  const noRequired = deriveDeviceSupportTier([
    {
      workflowId: "retroarch-plus-bios",
      applicability: "deferred",
      state: "deferred",
      targetWideFailure: null,
    },
  ]);
  assert.equal(noRequired, "unqualified");
});

test("projected fixture state matches the synthetic qualification history", () => {
  const context = projectionContext();
  const pocketRows = [
    "retroarch-plus-bios",
    "obtainium-install",
    "xaniteog-install",
    "rom-library-sync",
  ].map((workflowId) =>
    projectionState(
      context,
      targetByProfile(context.targets, SYNTHETIC_POCKET_S2_PROFILE).id,
      workflowId,
    ),
  );
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

  const airRows = [
    "retroarch-plus-bios",
    "obtainium-install",
    "xaniteog-install",
    "rom-library-sync",
  ].map((workflowId) =>
    projectionState(
      context,
      targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id,
      workflowId,
    ),
  );
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
  assert.deepEqual(
    context.records.map((record) => JSON.stringify(record)),
    before,
  );
});

test("production evidence loading rejects synthetic fixture paths", () => {
  assert.throws(
    () =>
      loadEvidenceDirectory(path.join(FIXTURES, "projection/evidence"), {
        fixtureMode: false,
      }),
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
  const entries = readdirSync(
    path.join(REPO_ROOT, "docs/testing/device-qualification/evidence"),
  ).sort();
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
    readFileSync(
      path.join(FIXTURES, "matrix/expected-qualified-limited.md"),
      "utf8",
    ),
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
  const catalogOrder = context.workflowCatalog.workflows.map(
    (workflow) => workflow.id,
  );
  for (const entry of projection.targets) {
    const rowOrder = entry.workflows.map((row) => row.workflowId);
    const positions = rowOrder.map((workflowId) =>
      catalogOrder.indexOf(workflowId),
    );
    assert.deepEqual(
      positions,
      [...positions].sort((left, right) => left - right),
    );
  }
});

test("production --check exits zero and detects matrix drift", () => {
  const check = () =>
    execFileSync(
      process.execPath,
      ["tools/device-qualification.mjs", "--check"],
      { cwd: REPO_ROOT, encoding: "utf8", stdio: "pipe" },
    );
  assert.doesNotThrow(check);
  const committed = readFileSync(
    path.join(REPO_ROOT, "docs/qualification/device-qualification-matrix.md"),
    "utf8",
  );
  assert.match(committed, /ayaneo\.pocket_air_mini/);
  assert.match(committed, /ARBOR GT78-VN/);
  assert.match(committed, /Support tier: unqualified/);
  assert.match(committed, /\| retroarch-plus-bios \| missing \|/);
  assert.match(committed, /# Device Qualification Matrix/);
  assert.doesNotMatch(committed, /\bphase[\s_-]*6f\b/i);
  assert.doesNotMatch(committed, /^\| [^|]+ \| qualified \|/im);
});

test("the CLI rejects unknown flags", () => {
  assert.throws(
    () =>
      execFileSync(
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
  assert.match(runbook, /unable_to_verify/);
  assert.match(runbook, /does not\s+itself imply support/i);
  assert.doesNotMatch(
    runbook,
    /EMUCHEF_[A-Z0-9_]*(?:BUILD_IDENTITY|RUNTIME_CONTRACT)/,
  );
  assert.match(
    runbook,
    /npm --prefix apps\/emuchef-app run device-qualification/,
  );
  assert.match(runbook, /Register device target/);
  assert.match(runbook, /canonical workflow/);
  assert.match(runbook, /Record qualification run/);
  assert.match(runbook, /invalid\/not_observed/);
  assert.match(runbook, /fresh build/);
  assert.match(runbook, /make device-qualification-check/);
  assert.match(
    runbook,
    /make device-qualification-check.*repository tests before committing\/shipping evidence/i,
  );
  assert.doesNotMatch(runbook, /first qualified device/i);
});

test("repository validation wires device qualification without claiming completion", () => {
  const makefile = readFileSync(path.join(REPO_ROOT, "Makefile"), "utf8");
  const workflow = readFileSync(
    path.join(
      REPO_ROOT,
      ".github/workflows/emuchef-execution-feature-matrix.yml",
    ),
    "utf8",
  );
  const roadmap = readFileSync(
    path.join(REPO_ROOT, "docs/product/product-roadmap.md"),
    "utf8",
  );

  assert.match(makefile, /device-qualification-check:/);
  assert.match(
    makefile,
    /Validate device qualification definitions, evidence, and matrix/,
  );
  assert.doesNotMatch(makefile, /Validate Phase/);
  assert.match(makefile, /node --test tools\/device-qualification\.test\.mjs/);
  assert.match(makefile, /node tools\/device-qualification\.mjs --check/);
  assert.match(makefile, /test: ensure-deps device-qualification-check/);
  assert.match(workflow, /tools\/device-qualification\*/);
  assert.match(workflow, /docs\/testing\/device-qualification\/\*\*/);
  assert.match(workflow, /node --test tools\/device-qualification\.test\.mjs/);
  assert.match(workflow, /node tools\/device-qualification\.mjs --check/);
  assert.match(workflow, /Validate device qualification foundation/);
  assert.doesNotMatch(workflow, /Validate Phase/);
  assert.match(roadmap, /6F \| Physical-device test matrix \| In progress/);
  assert.match(
    roadmap,
    /Harness\s+implementation by itself adds no physical target or evidence and qualifies no\s+workflow or device/i,
  );
  assert.match(
    roadmap,
    /production-bound device qualification harness is implemented and available/i,
  );
  assert.match(roadmap, /Physical-device matrix work\s+remains in progress/i);
  assert.match(roadmap, /Daijisho and\s+ES-DE remain deferred/i);
  assert.doesNotMatch(
    roadmap,
    /6F \| Physical-device test matrix \| Completed/,
  );
});

test("React qualification API contains no filesystem or process authority fields", () => {
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
});

test("active qualification paths and source use domain-oriented names", () => {
  for (const relativePath of activeQualificationFiles()) {
    assertActiveQualificationSourceUsesDomainNames(relativePath);
  }
});

test("active name guard excludes only its self-test and still rejects production terms", () => {
  const activeFiles = activeQualificationFiles();
  assert.equal(
    activeFiles.includes("tools/device-qualification.test.mjs"),
    false,
  );
  assert.equal(activeFiles.includes("tools/device-qualification.mjs"), true);
  const productionPath = "apps/emuchef-app/src/DeviceQualificationOverlay.tsx";
  assert.equal(activeFiles.includes(productionPath), true);
  assert.equal(activeFiles.includes("Makefile"), true);
  assert.equal(
    activeFiles.includes(
      ".github/workflows/emuchef-execution-feature-matrix.yml",
    ),
    true,
  );
  assert.throws(
    () =>
      assertActiveQualificationSourceUsesDomainNames(
        productionPath,
        `${readActiveQualificationSource(productionPath)}\nconst legacyLabel = "phase-7";\n`,
      ),
    (error) => error?.code === "ERR_ASSERTION",
  );
});

test("canonical qualification tooling uses domain-oriented output labels", () => {
  const toolSource = readFileSync(
    path.join(REPO_ROOT, "tools/device-qualification.mjs"),
    "utf8",
  );
  assert.doesNotMatch(toolSource, /\bphase[\s_-]*6f\b/i);
  assert.doesNotMatch(toolSource, /\bslice[\s_-]*\d+[a-z0-9-]*\b/i);
});

test("ROM Merge evidence passes direct validation against the approved APK pin", () => {
  const fixture = romMergeEvidenceBundle();
  const repoRoot = createRomMergePinRoot();
  try {
    const canonicalBundle = { ...fixture.bundle };
    delete canonicalBundle.candidate;
    assert.doesNotThrow(() =>
      validateEvidenceBundle(canonicalBundle, fixture.context, { repoRoot }),
    );
  } finally {
    removeTempTree(repoRoot);
  }
});

test("the production RetroArch pin disables ROM Merge promotion while null", () => {
  const pin = JSON.parse(
    readFileSync(path.join(REPO_ROOT, ROM_MERGE_PIN_PATH), "utf8"),
  );
  assert.deepEqual(pin, { schemaVersion: 1, retroarch: null });

  const fixture = romMergeEvidenceBundle();
  const repoRoot = createRomMergePinRoot({
    schemaVersion: 1,
    retroarch: null,
  });
  try {
    assert.throws(
      () => validateEvidenceBundle(fixture.bundle, fixture.context, { repoRoot }),
      /ROM Merge promotion is disabled.*pin is null/i,
    );
  } finally {
    removeTempTree(repoRoot);
  }
});

test("a missing RetroArch pin fails closed for KONKR ROM Merge evidence", () => {
  const fixture = romMergeEvidenceBundle();
  const repoRoot = mkdtempSync(path.join(tmpdir(), "rom-merge-missing-pin-"));
  try {
    assert.throws(
      () => validateEvidenceBundle(fixture.bundle, fixture.context, { repoRoot }),
      /ROM Merge APK pin file is missing/i,
    );
  } finally {
    removeTempTree(repoRoot);
  }
});

test("ROM Merge evidence rejects untrusted pin values and stale strategy bindings", () => {
  const invalidCases = [
    ["package id", (evidence) => (evidence.retroArch.packageId = "other.package")],
    ["pinned source", (evidence) => (evidence.retroArch.pinnedSource = "other-source")],
    ["pinned version", (evidence) => (evidence.retroArch.pinnedVersion = "other-version")],
    ["installed version", (evidence) => (evidence.retroArch.installedVersion = "other-version")],
    ["installed APK digest", (evidence) => (evidence.retroArch.installedApkSha256 = "0".repeat(64))],
    ["pinned APK digest", (evidence) => (evidence.retroArch.pinnedArtifactSha256 = "0".repeat(64))],
      ["strategy digest", (evidence) => (evidence.strategyDigest = "sha256:" + "0".repeat(64))],
      ["report digest", (evidence) => (evidence.executionReportSha256 = "0".repeat(64))],
    ["candidate handle", (evidence) => (evidence.candidateHandle = "qualification-candidate-00000000000000000000000000000000")],
    ["session id", (evidence) => (evidence.sessionId = "another-session")],
    ["observed root state", (evidence) => (evidence.observedRootState = "rooted")],
    ["extra field", (evidence) => (evidence.unreviewed = true)],
    ["missing field", (evidence) => delete evidence.sourceSnapshot.sourceIdentity],
  ];
  const repoRoot = createRomMergePinRoot();
  try {
    for (const [label, updateEvidence] of invalidCases) {
      const fixture = romMergeEvidenceBundle({ updateEvidence });
      assert.throws(
        () => validateEvidenceBundle(fixture.bundle, fixture.context, { repoRoot }),
        /ROM Merge evidence|sourceSnapshot|strategy|candidate|session|observedRootState|retroArch/i,
        label,
      );
    }
  } finally {
    removeTempTree(repoRoot);
  }
});

test("ROM Merge evidence binds root state to both candidate and execution report", () => {
  const mismatchedCandidate = romMergeEvidenceBundle({
    updateEvidence(evidence) {
      evidence.observedRootState = "invalid-root-state";
    },
  });
  const repoRoot = createRomMergePinRoot();
  try {
    assert.throws(
      () =>
        validateEvidenceBundle(
          mismatchedCandidate.bundle,
          mismatchedCandidate.context,
          { repoRoot },
        ),
      /observedRootState/i,
    );
  } finally {
    removeTempTree(repoRoot);
  }

  const mismatchedReport = romMergeEvidenceBundle({
    updateReport(report) {
      report.qualification.observedRootState = "invalid-root-state";
    },
  });
  const secondRepoRoot = createRomMergePinRoot();
  try {
    assert.throws(
      () =>
        validateEvidenceBundle(
          mismatchedReport.bundle,
          mismatchedReport.context,
          { repoRoot: secondRepoRoot },
        ),
      /observedRootState/i,
    );
  } finally {
    removeTempTree(secondRepoRoot);
  }
});

test("ROM Merge manifests enforce merge precedence, path, and sentinel rules", () => {
  const invalidCases = [
    ["empty source", (evidence) => (evidence.sourceSnapshot.entries = [])],
    [
      "source missing from after",
      (evidence) => (evidence.destination.after = [evidence.destination.after[1]]),
    ],
    [
      "source hash precedence",
      (evidence) => (evidence.destination.after[0].sha256 = "2".repeat(64)),
    ],
    [
      "delete existing destination entry",
      (evidence) =>
        evidence.destination.before.push({
          relativePath: "keep/existing.rom",
          sha256: "4".repeat(64),
        }),
    ],
    [
      "add unrelated destination entry",
      (evidence) =>
        evidence.destination.after.push({
          relativePath: "unreviewed/extra.rom",
          sha256: "3".repeat(64),
        }),
    ],
    [
      "change unrelated destination entry",
      (evidence) => {
        evidence.destination.before.push({
          relativePath: "keep/existing.rom",
          sha256: "4".repeat(64),
        });
        evidence.destination.after.push({
          relativePath: "keep/existing.rom",
          sha256: "5".repeat(64),
        });
      },
    ],
    [
      "copy sentinel from source",
      (evidence) =>
        evidence.sourceSnapshot.entries.push({
          relativePath: ROM_MERGE_SENTINEL_PATH,
          sha256: ROM_MERGE_SENTINEL_SHA256,
        }),
    ],
    [
      "change sentinel after copy",
      (evidence) => (evidence.destination.after[1].sha256 = "6".repeat(64)),
    ],
    [
      "duplicate normalized path",
      (evidence) => evidence.sourceSnapshot.entries.push(
        structuredClone(evidence.sourceSnapshot.entries[0]),
      ),
    ],
    [
      "non-normalized relative path",
      (evidence) => (evidence.sourceSnapshot.entries[0].relativePath = "roms/../game.rom"),
    ],
  ];
  const repoRoot = createRomMergePinRoot();
  try {
    for (const [label, updateEvidence] of invalidCases) {
      const fixture = romMergeEvidenceBundle({ updateEvidence });
      assert.throws(
        () => validateEvidenceBundle(fixture.bundle, fixture.context, { repoRoot }),
        /ROM Merge evidence|source|destination|sentinel|path|manifest/i,
        label,
      );
    }
  } finally {
    removeTempTree(repoRoot);
  }
});

test("ROM Merge evidence rejects authority flags and unsupported capabilities", () => {
  const invalidEvidence = [
    ["root authority", (evidence) => (evidence.rootAuthorityUsed = true)],
    ["OEM compiler", (evidence) => (evidence.oemCompilerUsed = true)],
    ["OEM staging", (evidence) => (evidence.oemStageUsed = true)],
  ];
  const repoRoot = createRomMergePinRoot();
  try {
    for (const [label, updateEvidence] of invalidEvidence) {
      const fixture = romMergeEvidenceBundle({ updateEvidence });
      assert.throws(
        () => validateEvidenceBundle(fixture.bundle, fixture.context, { repoRoot }),
        /authority|OEM|ROM Merge evidence/i,
        label,
      );
    }

    const privileged = romMergeEvidenceBundle({
      updateReport(report) {
        report.qualification.effectiveCapabilities = ["root_shell"];
      },
    });
    assert.throws(
      () => validateEvidenceBundle(privileged.bundle, privileged.context, { repoRoot }),
      /privileged|capability/i,
    );

    const unsupported = romMergeEvidenceBundle({
      updateReport(report) {
        report.qualification.effectiveCapabilities = ["unsupported_capability"];
      },
    });
    assert.throws(
      () => validateEvidenceBundle(unsupported.bundle, unsupported.context, { repoRoot }),
      /unsupported|capability/i,
    );
  } finally {
    removeTempTree(repoRoot);
  }
});

test("ROM Merge uses separate qualification and runtime strategy bindings", () => {
  const valid = romMergeEvidenceBundle();
  assert.equal(valid.report.qualification.strategy, "adb_unprivileged");
  assert.equal(valid.report.runtime.executionRestriction.strategy, "adb_shell");
  assert.equal(valid.report.qualification.restriction, "privileged_execution_disabled");

  const repoRoot = createRomMergePinRoot();
  try {
    const wrongQualificationStrategy = romMergeEvidenceBundle({
      updateReport(report) {
        report.qualification.strategy = "adb_shell";
      },
    });
    assert.throws(
      () =>
        validateEvidenceBundle(
          wrongQualificationStrategy.bundle,
          wrongQualificationStrategy.context,
          { repoRoot },
        ),
      /qualification strategy/i,
    );

    const wrongRuntimeStrategy = romMergeEvidenceBundle({
      updateReport(report) {
        report.runtime.executionRestriction.strategy = "adb_unprivileged";
      },
    });
    assert.throws(
      () =>
        validateEvidenceBundle(
          wrongRuntimeStrategy.bundle,
          wrongRuntimeStrategy.context,
          { repoRoot },
        ),
      /verified runtime restriction/i,
    );
  } finally {
    removeTempTree(repoRoot);
  }
});

test("ROM Merge evidence requires the successful terminal recipe step and executed operations", () => {
  const invalidReports = [
    ["failed terminal step", (report) => (report.execution.recipes[0].steps[0].status = "failed")],
    ["skipped terminal step", (report) => (report.execution.recipes[0].steps[0].status = "skipped")],
    ["cancelled terminal step", (report) => (report.execution.recipes[0].steps[0].status = "cancelled")],
    ["wrong terminal step id", (report) => (report.execution.recipes[0].steps[0].stepId = "feature.copy_roms/other")],
    ["missing terminal step", (report) => (report.execution.recipes[0].steps = [])],
    ["missing operation outcome", (report) => (report.qualification.operationOutcomes = [])],
    ["skipped operation outcome", (report) => (report.qualification.operationOutcomes[0].outcome = "skipped")],
    ["cancelled operation outcome", (report) => (report.qualification.operationOutcomes[0].outcome = "cancelled")],
    ["failed operation outcome", (report) => (report.qualification.operationOutcomes[0].outcome = "failed")],
    [
      "duplicate operation outcome",
      (report) => report.qualification.operationOutcomes.push(
        structuredClone(report.qualification.operationOutcomes[0]),
      ),
    ],
  ];
  const repoRoot = createRomMergePinRoot();
  try {
    for (const [label, updateReport] of invalidReports) {
      const fixture = romMergeEvidenceBundle({ updateReport });
      assert.throws(
        () => validateEvidenceBundle(fixture.bundle, fixture.context, { repoRoot }),
        /ROM Merge|terminal|step|operation|executed|restricted/i,
        label,
      );
    }
  } finally {
    removeTempTree(repoRoot);
  }
});

test("KONKR ROM Merge candidate bytes reach the canonical bundle unchanged", () => {
  const repoRoot = createTempQualificationRepo();
  const candidateId = "qualification-candidate-1234567890abcdef1234567890abcdef";
  try {
    const fixture = candidateRomMergeArtifacts(repoRoot, candidateId);
    writeCandidateFixture(
      repoRoot,
      fixture.candidate,
      fixture.reportBytes,
      fixture.romMergeEvidenceBytes,
    );

    const record = recordQualificationRunCandidate(candidateId, { repoRoot });
    assert.equal(record.artifacts.length, 2);
    assert.deepEqual(
      record.artifacts.map(({ id, path: artifactPath }) => [id, artifactPath]),
      [
        ["execution-report", "execution-report.json"],
        [ROM_MERGE_EVIDENCE_ID, ROM_MERGE_EVIDENCE_PATH],
      ],
    );

    const evidenceRoot = path.join(
      repoRoot,
      "docs/testing/device-qualification/evidence",
    );
    const canonicalPath = path.join(evidenceRoot, record.runId);
    const canonicalBundle = loadEvidenceBundle(canonicalPath);
    assert.deepEqual(canonicalBundle.reportBytes, fixture.reportBytes);
    assert.deepEqual(
      canonicalBundle.romMergeEvidenceBytes,
      fixture.romMergeEvidenceBytes,
    );
    const workflowCatalog = loadWorkflowCatalog(
      path.join(repoRoot, "docs/testing/device-qualification/workflow-catalog.json"),
    );
    const targets = loadDeviceTargets(
      path.join(repoRoot, "docs/testing/device-qualification/device-targets.json"),
      { authoredProfilesDir: path.join(repoRoot, "authored/device_profiles") },
    ).targets;
    const context = { workflowCatalog, targets };
    assert.doesNotThrow(() =>
      validateEvidenceBundle(canonicalBundle, context, { repoRoot }),
    );
    assert.deepEqual(
      loadValidatedEvidenceRecords(evidenceRoot, {
        fixtureMode: false,
        workflowCatalog,
        targets,
        repoRoot,
      }).map((item) => item.runId),
      [record.runId],
    );
    assert.deepEqual(
      loadEvidenceDirectory(evidenceRoot, {
        fixtureMode: false,
        workflowCatalog,
        targets,
        repoRoot,
      }).map((item) => item.runId),
      [record.runId],
    );
  } finally {
    removeTempTree(repoRoot);
  }
});

test("archived ROM Merge evidence stays readable but stops qualifying after an APK pin change", () => {
  const repoRoot = createTempQualificationRepo();
  const evidenceRoot = path.join(
    repoRoot,
    "docs/testing/device-qualification/evidence",
  );
  const pinA = ROM_MERGE_TEST_PIN;
  const pinB = {
    schemaVersion: 1,
    retroarch: {
      ...ROM_MERGE_TEST_PIN.retroarch,
      source: "https://example.invalid/retroarch-b.apk",
      version: "1.20.0",
      sha256: "d".repeat(64),
    },
  };
  try {
    const candidateAId =
      "qualification-candidate-aaaaaaaa11111111bbbbbbbb22222222";
    const candidateA = candidateRomMergeArtifacts(
      repoRoot,
      candidateAId,
      pinA,
    );
    writeCandidateFixture(
      repoRoot,
      candidateA.candidate,
      candidateA.reportBytes,
      candidateA.romMergeEvidenceBytes,
    );
    const recordA = recordQualificationRunCandidate(candidateAId, { repoRoot });
    const historicalBundlePath = path.join(evidenceRoot, recordA.runId);

    const pinPath = path.join(repoRoot, ROM_MERGE_PIN_PATH);
    writeRomMergePin(repoRoot, pinB);
    commitRepoChanges(repoRoot, "approve the next RetroArch APK pin");

    const historicalBundle = loadEvidenceBundle(historicalBundlePath);
    assert.ok(historicalBundle.romMergeEvidenceBytes instanceof Buffer);
    const workflowCatalog = loadWorkflowCatalog(
      path.join(repoRoot, "docs/testing/device-qualification/workflow-catalog.json"),
    );
    const targets = loadDeviceTargets(
      path.join(repoRoot, "docs/testing/device-qualification/device-targets.json"),
      { authoredProfilesDir: path.join(repoRoot, "authored/device_profiles") },
    ).targets;
    const context = { workflowCatalog, targets };
    assert.deepEqual(
      loadEvidenceDirectory(evidenceRoot, {
        fixtureMode: false,
        workflowCatalog,
        targets,
        repoRoot,
      }).map(({ runId }) => runId),
      [recordA.runId],
    );
    assert.deepEqual(
      loadValidatedEvidenceRecords(evidenceRoot, {
        fixtureMode: false,
        workflowCatalog,
        targets,
        repoRoot,
      }),
      [],
    );
    assert.throws(
      () =>
        validateEvidenceBundle(historicalBundle, context, { repoRoot }),
      /repository-approved APK pin/i,
    );

    assert.throws(
      () =>
        validateEvidenceBundle(historicalBundle, context, {
          repoRoot,
          allowHistoricalRomMergePin: true,
        }),
      /repository-approved APK pin/i,
    );

    const candidateBId =
      "qualification-candidate-cccccccc33333333dddddddd44444444";
    const candidateB = candidateRomMergeArtifacts(
      repoRoot,
      candidateBId,
      pinB,
      { configureRepo: false },
    );
    writeCandidateFixture(
      repoRoot,
      candidateB.candidate,
      candidateB.reportBytes,
      candidateB.romMergeEvidenceBytes,
    );
    const recordB = recordQualificationRunCandidate(candidateBId, { repoRoot });
    assert.deepEqual(
      loadValidatedEvidenceRecords(evidenceRoot, {
        fixtureMode: false,
        workflowCatalog,
        targets,
        repoRoot,
      }).map(({ runId }) => runId),
      [recordB.runId],
    );
    assert.ok(existsSync(pinPath));

  const assertHistoricalEvidenceRemainsReadable = () => {
    assert.deepEqual(
      loadEvidenceDirectory(evidenceRoot, {
        fixtureMode: false,
        workflowCatalog,
        targets,
        repoRoot,
      })
        .map(({ runId }) => runId)
        .sort(),
      [recordA.runId, recordB.runId].sort(),
    );
    assert.deepEqual(
      loadValidatedEvidenceRecords(evidenceRoot, {
        fixtureMode: false,
        workflowCatalog,
        targets,
        repoRoot,
      }),
      [],
    );
  };

  writeRomMergePin(repoRoot, { schemaVersion: 1, retroarch: null });
  commitRepoChanges(repoRoot, "disable current RetroArch APK pin");
  assertHistoricalEvidenceRemainsReadable();

  rmSync(pinPath);
  assertHistoricalEvidenceRemainsReadable();

  writeFileSync(pinPath, "{invalid json\n", "utf8");
  assertHistoricalEvidenceRemainsReadable();
  } finally {
    removeTempTree(repoRoot);
  }
});

test("KONKR passing report without ROM Merge evidence fails before mutation", () => {
  const repoRoot = createTempQualificationRepo();
  const candidateId = "qualification-candidate-abcdefabcdefabcdefabcdefabcdefab";
  try {
    const fixture = candidateRomMergeArtifacts(repoRoot, candidateId);
    writeCandidateFixture(repoRoot, fixture.candidate, fixture.reportBytes);
    const evidenceRoot = path.join(
      repoRoot,
      "docs/testing/device-qualification/evidence",
    );
    const matrixPath = path.join(
      repoRoot,
      "docs/qualification/device-qualification-matrix.md",
    );
    const beforeEvidence = snapshotTree(evidenceRoot);
    const beforeMatrix = readFileSync(matrixPath, "utf8");

    assert.throws(
      () => recordQualificationRunCandidate(candidateId, { repoRoot }),
      /ROM Merge evidence is required for konkr\.pocket_advance/i,
    );
    assert.deepEqual(snapshotTree(evidenceRoot), beforeEvidence);
    assert.equal(readFileSync(matrixPath, "utf8"), beforeMatrix);
  } finally {
    removeTempTree(repoRoot);
  }
});

test("candidate ROM Merge files reject malformed, stale, replaced, duplicate, and symlinked data before mutation", () => {
  const cases = [
    {
      label: "missing sidecar",
      evidenceBytes: "valid",
      update({ directory }) {
        rmSync(path.join(directory, ROM_MERGE_EVIDENCE_PATH));
      },
    },
    {
      label: "stale byte length metadata",
      evidenceBytes: "valid",
      update({ directory }) {
        const candidatePath = path.join(directory, "candidate.json");
        const envelope = JSON.parse(readFileSync(candidatePath, "utf8"));
        envelope.romMergeEvidence.byteLength += 1;
        writeFileSync(candidatePath, JSON.stringify(envelope), "utf8");
      },
    },
    {
      label: "stale digest metadata",
      evidenceBytes: "valid",
      update({ directory }) {
        const candidatePath = path.join(directory, "candidate.json");
        const envelope = JSON.parse(readFileSync(candidatePath, "utf8"));
        envelope.romMergeEvidence.sha256 = "0".repeat(64);
        writeFileSync(candidatePath, JSON.stringify(envelope), "utf8");
      },
    },
    {
      label: "wrong fixed evidence path",
      evidenceBytes: "valid",
      update({ directory }) {
        const candidatePath = path.join(directory, "candidate.json");
        const envelope = JSON.parse(readFileSync(candidatePath, "utf8"));
        envelope.romMergeEvidence.path = "other-evidence.json";
        writeFileSync(candidatePath, JSON.stringify(envelope), "utf8");
      },
    },
    {
      label: "malformed JSON bytes",
      evidenceBytes: Buffer.from("{not-json}\n", "utf8"),
    },
{
label: "evidence exceeds the supported size bound",
evidenceBytes: Buffer.alloc(1024 * 1024 + 1, 0x20),
},
{
label: "evidence is not valid UTF-8",
evidenceBytes: Buffer.from([0xff, 0xfe]),
},
    {
      label: "replaced bytes",
      evidenceBytes: "valid",
      update({ directory }) {
        writeFileSync(
          path.join(directory, ROM_MERGE_EVIDENCE_PATH),
          "replacement bytes\n",
          "utf8",
        );
      },
    },
    {
      label: "symlinked evidence file",
      evidenceBytes: "valid",
      update({ repoRoot, directory }) {
        const sidecarPath = path.join(directory, ROM_MERGE_EVIDENCE_PATH);
        const replacementPath = path.join(repoRoot, "replacement-evidence.json");
        writeFileSync(replacementPath, "replacement bytes\n", "utf8");
        rmSync(sidecarPath);
        symlinkSync(replacementPath, sidecarPath);
      },
    },
    {
      label: "unbound sidecar",
      evidenceBytes: "orphan",
      update({ repoRoot, directory, fixture }) {
        rmSync(path.join(directory, ROM_MERGE_EVIDENCE_PATH), { force: true });
        writeFileSync(
          path.join(directory, ROM_MERGE_EVIDENCE_PATH),
          fixture.romMergeEvidenceBytes,
        );
        const candidatePath = path.join(directory, "candidate.json");
        const envelope = JSON.parse(readFileSync(candidatePath, "utf8"));
        delete envelope.romMergeEvidence;
        writeFileSync(candidatePath, JSON.stringify(envelope), "utf8");
      },
    },
    {
      label: "duplicate artifact reference",
      evidenceBytes: "valid",
  update({ fixture, directory }) {
    const duplicateReference = {
          id: ROM_MERGE_EVIDENCE_ID,
          kind: ROM_MERGE_EVIDENCE_KIND,
          path: ROM_MERGE_EVIDENCE_PATH,
          sha256: createHash("sha256")
            .update(fixture.romMergeEvidenceBytes)
            .digest("hex"),
    };
    fixture.candidate.artifacts.push(duplicateReference);
    const candidatePath = path.join(directory, "candidate.json");
    const envelope = JSON.parse(readFileSync(candidatePath, "utf8"));
    envelope.payload.artifacts.push(duplicateReference);
    writeFileSync(candidatePath, JSON.stringify(envelope), "utf8");
      },
    },
  ];

  for (const [index, item] of cases.entries()) {
    const repoRoot = createTempQualificationRepo();
    const candidateId = `qualification-candidate-${(index + 1)
      .toString(16)
      .padStart(32, "0")}`;
    try {
      const fixture = candidateRomMergeArtifacts(repoRoot, candidateId);
      let evidenceBytes = fixture.romMergeEvidenceBytes;
      if (Buffer.isBuffer(item.evidenceBytes)) evidenceBytes = item.evidenceBytes;
      writeCandidateFixture(
        repoRoot,
        fixture.candidate,
        fixture.reportBytes,
        item.evidenceBytes === "orphan" ? null : evidenceBytes,
      );
      const directory = path.join(
        repoRoot,
        ".emuchef_runtime/qualification-candidates",
        candidateId,
      );
      item.update?.({ repoRoot, directory, fixture });
      const evidenceRoot = path.join(
        repoRoot,
        "docs/testing/device-qualification/evidence",
      );
      const matrixPath = path.join(
        repoRoot,
        "docs/qualification/device-qualification-matrix.md",
      );
      const beforeEvidence = snapshotTree(evidenceRoot);
      const beforeMatrix = readFileSync(matrixPath, "utf8");

      assert.throws(
        () => recordQualificationRunCandidate(candidateId, { repoRoot }),
        /ROM Merge|rom-merge-evidence|artifact|candidate|JSON|symlink|sha256|byte length/i,
        item.label,
      );
      assert.deepEqual(snapshotTree(evidenceRoot), beforeEvidence, item.label);
      assert.equal(readFileSync(matrixPath, "utf8"), beforeMatrix, item.label);
    } finally {
      removeTempTree(repoRoot);
    }
  }
});

test("non-KONKR evidence remains compatible and cannot carry ROM Merge artifacts", () => {
  const ordinary = ordinaryExecutionBundle();
  assert.doesNotThrow(() => validateEvidenceBundle(ordinary, syntheticContext()));

  const { bundle, context } = romLibrarySyncBundle({
    profileId: "ayaneo.pocket_s2",
  });
  assert.doesNotThrow(() => validateEvidenceBundle(bundle, context));

  const unsupported = {
    record: structuredClone(ordinary.record),
    reportBytes: ordinary.reportBytes,
  };
  unsupported.record.artifacts.push({
    id: ROM_MERGE_EVIDENCE_ID,
    kind: ROM_MERGE_EVIDENCE_KIND,
    path: ROM_MERGE_EVIDENCE_PATH,
    sha256: "0".repeat(64),
  });
  unsupported.record = sealRecord(unsupported.record);
  unsupported.romMergeEvidenceBytes = Buffer.from("{}\n", "utf8");
  assert.throws(
    () => validateEvidenceBundle(unsupported, syntheticContext()),
    /not supported for this workflow and device profile/i,
  );
});

test("KONKR support tier is capped at limited while lower tiers remain intact", () => {
  const qualified = [
    {
      workflowId: "rom-library-sync",
      applicability: "required",
      state: "qualified",
      targetWideFailure: null,
    },
  ];
  assert.equal(deriveDeviceSupportTier(qualified), "qualified");
  assert.equal(
    deriveDeviceSupportTier(qualified, "konkr.pocket_advance"),
    "limited",
  );

  const partlyQualified = [
    ...qualified,
    {
      workflowId: "retroarch-plus-bios",
      applicability: "required",
      state: "not_qualified",
      targetWideFailure: null,
    },
  ];
  assert.equal(deriveDeviceSupportTier(partlyQualified), "limited");
  assert.equal(
    deriveDeviceSupportTier(partlyQualified, "konkr.pocket_advance"),
    "limited",
  );
  const unqualified = [
    {
      ...qualified[0],
      state: "not_qualified",
    },
  ];
  assert.equal(
    deriveDeviceSupportTier(unqualified, "konkr.pocket_advance"),
    "unqualified",
  );
});

test("ROM Merge schema validation keeps artifact alternatives and the two-item limit", () => {
  const schema = JSON.parse(
    readFileSync(
      path.join(REPO_ROOT, "docs/testing/device-qualification/evidence-schema.json"),
      "utf8",
    ),
  );
  assert.equal(validateEvidenceSchemaContract(schema), true);

  const missingLimit = structuredClone(schema);
  delete missingLimit.properties.artifacts.maxItems;
  assert.throws(
    () => validateEvidenceSchemaContract(missingLimit),
    /artifacts maxItems/i,
  );

  const alteredAlternative = structuredClone(schema);
  alteredAlternative.$defs.evidenceArtifact.oneOf[1].properties.kind.const =
    "unexpected_artifact";
  assert.throws(
    () => validateEvidenceSchemaContract(alteredAlternative),
    /evidence schema artifact alternatives/i,
  );
});

test("ROM Merge gate leaves workflow v1 and execution report v2 unchanged", () => {
  const workflowCatalog = JSON.parse(
    readFileSync(
      path.join(REPO_ROOT, "docs/testing/device-qualification/workflow-catalog.json"),
      "utf8",
    ),
  );
  const executionReportSchema = JSON.parse(
    readFileSync(
      path.join(REPO_ROOT, "docs/testing/device-qualification/execution-report.schema.json"),
      "utf8",
    ),
  );
  assert.equal(workflowCatalog.schemaVersion, 1);
  assert.equal(executionReportSchema.properties.schemaVersion.const, 2);
});
