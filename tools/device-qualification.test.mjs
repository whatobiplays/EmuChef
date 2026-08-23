import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  EVIDENCE_RECORD_FIELDS,
  FINGERPRINT_FIELDS,
  RUN_VALIDITIES,
  QUALIFICATION_OUTCOMES,
  CHECKPOINT_OUTCOMES,
  TARGET_WIDE_FAILURES,
  buildCurrentFingerprint,
  canonicalDigest,
  canonicalize,
  classifyCompatibility,
  deviceTargetId,
  deriveApplicability,
  deriveDeviceSupportTier,
  deriveWorkflowState,
  evidenceFingerprintDigest,
  evidenceRecordDigest,
  loadEvidenceDirectory,
  loadDeviceTargets,
  loadWorkflowCatalog,
  projectQualificationState,
  renderQualificationMatrix,
  selectCurrentEvidence,
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
  return JSON.parse(readFileSync(path.join(FIXTURES, relative), "utf8"));
}

function sealRecord(record) {
  record.recordDigest = evidenceRecordDigest(record);
  return record;
}

function checkpointGatedRecord() {
  const record = recordFor("evidence-valid/passing-retroarch-bios.json");
  record.runId = `qualification-run-sha256:${"7".repeat(64)}`;
  record.capturedAt = "2026-08-21T02:00:00Z";
  record.workflowId = "checkpoint-gated";
  record.fingerprint.authoredContent = [];
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

const CURRENT_BUILD = "2026-08-21-foundation";
const RUNTIME_CONTRACT = "v1";
const AUTHORED_DIGESTS = {
  "app.retroarch.provision": "a".repeat(64),
  "feature.copy_bios": "b".repeat(64),
  "app.obtainium.install": "c".repeat(64),
  "app.xaniteog.install": "d".repeat(64),
  "feature.copy_roms": "e".repeat(64),
};

function projectionContext() {
  return {
    workflowCatalog: loadWorkflowCatalog(path.join(FIXTURES, "definitions-valid/workflow-catalog.json")),
    targets: loadDeviceTargets(
      path.join(FIXTURES, "projection/device-targets.json"),
      { authoredProfilesDir: AUTHORED_PROFILES },
    ).targets,
    records: [
      "qualified.json",
      "failed-newer.json",
      "stale.json",
      "invalid-newer.json",
      "xaniteog-passed-older.json",
    ].map((name) => JSON.parse(readFileSync(path.join(FIXTURES, "projection/evidence", name), "utf8"))),
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
  const record = recordFor("evidence-valid/passing-retroarch-bios.json");
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

test("a passing evidence fixture validates with its stored digests", () => {
  assert.doesNotThrow(() => validateEvidenceRecord(
    recordFor("evidence-valid/passing-retroarch-bios.json"),
    syntheticContext(),
  ));
});

test("a valid failed evidence fixture validates", () => {
  assert.doesNotThrow(() => validateEvidenceRecord(
    recordFor("evidence-valid/failed-retroarch-bios.json"),
    syntheticContext(),
  ));
});

test("rejects a fingerprint digest that does not match structured inputs", () => {
  const record = recordFor("evidence-invalid/bad-digest.json");
  assert.throws(
    () => validateEvidenceRecord(record, syntheticContext()),
    /fingerprintDigest does not match canonical fingerprint/,
  );
});

test("rejects a missing required human-checkpoint result as invalid evidence", () => {
  const record = recordFor("evidence-invalid/missing-required-checkpoint.json");
  assert.throws(
    () => validateEvidenceRecord(record, syntheticContext()),
    /missing required human checkpoint/,
  );
});

test("invalid infrastructure runs cannot claim a product qualification failure", () => {
  const record = recordFor("evidence-invalid/impossible-run-result.json");
  assert.throws(
    () => validateEvidenceRecord(record, syntheticContext()),
    /invalid run must use qualificationOutcome "not_observed"/,
  );
});

test("an invalid infrastructure run with not_observed remains valid historical evidence", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios.json");
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
  const validNotObserved = recordFor("evidence-valid/passing-retroarch-bios.json");
  validNotObserved.qualificationOutcome = "not_observed";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(validNotObserved), context),
    /valid run must use qualificationOutcome/,
  );
  for (const outcome of ["passed", "failed"]) {
    const record = recordFor("evidence-valid/passing-retroarch-bios.json");
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
  const record = recordFor("evidence-valid/passing-retroarch-bios.json");
  record.automatedObservations = [];
  assert.throws(
    () => validateEvidenceRecord(sealRecord(record), syntheticContext()),
    /missing required automated observation/,
  );
});

test("a failed required automated observation produces a valid failed record", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios.json");
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
  const failed = recordFor("evidence-valid/failed-retroarch-bios.json");
  failed.targetWideFailure = "safety_invariant_failed";
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(failed), context));

  const unknown = recordFor("evidence-valid/failed-retroarch-bios.json");
  unknown.targetWideFailure = "arbitrary_failure";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(unknown), context),
    /targetWideFailure/,
  );

  const passed = recordFor("evidence-valid/passing-retroarch-bios.json");
  passed.targetWideFailure = "safety_invariant_failed";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(passed), context),
    /targetWideFailure|failed/,
  );
});

test("a valid failed record must contain a failed observation, failed checkpoint, or target-wide failure", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios.json");
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
  const wrongTarget = recordFor("evidence-valid/passing-retroarch-bios.json");
  wrongTarget.deviceTarget.id = targetByProfile(context.targets, SYNTHETIC_AIR_MINI_PROFILE).id;
  assert.throws(
    () => validateEvidenceRecord(sealRecord(wrongTarget), context),
    /unknown device target|registered target/,
  );

  const wrongWorkflow = recordFor("evidence-valid/passing-retroarch-bios.json");
  wrongWorkflow.workflowId = "does-not-exist";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(wrongWorkflow), context),
    /unknown workflow/,
  );

  const wrongVersion = recordFor("evidence-valid/passing-retroarch-bios.json");
  wrongVersion.workflowVersion = 2;
  wrongVersion.fingerprint.workflowVersion = 2;
  wrongVersion.fingerprintDigest = evidenceFingerprintDigest(wrongVersion.fingerprint);
  assert.throws(
    () => validateEvidenceRecord(sealRecord(wrongVersion), context),
    /workflow version/,
  );
});

test("fingerprint validation is strict, deterministic, and digest-bound", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios.json");
  assert.equal(evidenceFingerprintDigest(record.fingerprint), record.fingerprintDigest);
  const extra = structuredClone(record.fingerprint);
  extra.extra = true;
  assert.throws(() => evidenceFingerprintDigest(extra), /fields must be exactly/);
  const tampered = structuredClone(record.fingerprint);
  tampered.emuchefBuild = "other-build";
  assert.notEqual(evidenceFingerprintDigest(tampered), record.fingerprintDigest);
});

test("compatibility classifies identical evidence as compatible", () => {
  const record = recordFor("evidence-valid/passing-retroarch-bios.json");
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
  const base = recordFor("evidence-valid/passing-retroarch-bios.json").fingerprint;
  const cases = [
    ["emuchefBuild", "other-build"],
    ["workflowVersion", 2],
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

test("compatibility ignores dimensions the workflow does not declare", () => {
  const context = syntheticContext();
  const workflow = context.workflowCatalog.workflows.find((item) => item.id === "retroarch-plus-bios");
  const base = recordFor("evidence-valid/passing-retroarch-bios.json").fingerprint;
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
  buildChanged.emuchefBuild = "other-build";
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
  assert.equal(selected.runId, "qualification-run-sha256:" + "a".repeat(64));
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
  assert.equal(selected.runId, "qualification-run-sha256:" + "e".repeat(64));
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
  const base = recordFor("evidence-valid/passing-retroarch-bios.json");
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
  assert.match(runbook, /unable_to_verify/);
  assert.match(runbook, /does not\s+itself imply support/i);
  assert.match(runbook, /EMUCHEF_PHASE_6F_BUILD_IDENTITY/);
  assert.match(runbook, /EMUCHEF_PHASE_6F_RUNTIME_CONTRACT/);
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
