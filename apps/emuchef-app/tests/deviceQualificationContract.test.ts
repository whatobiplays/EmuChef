import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import type {
  QualificationModeStatus,
  QualificationTargetCandidatePreview,
} from "../src/types";

const targetPreview: QualificationTargetCandidatePreview = {
  candidateHandle: "qualification-candidate-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  kind: "target_registration",
  capturedAt: "2026-08-23T12:00:00Z",
  target: {
    profileId: { value: "ayaneo.pocket_s2", source: "production_observation" },
    manufacturer: { value: "AYANEO", source: "production_observation" },
    model: { value: "Pocket S2", source: "production_observation" },
    androidVersion: { value: "15", source: "production_observation" },
    androidApi: { value: 35, source: "production_observation" },
    abiSocClass: { value: "arm64", source: "production_observation" },
    rootState: { value: "non_root", source: "explicit_root_check" },
    connectionType: { value: "usb3", source: "operator_attestation" },
    firmwareBuild: { value: "vendor/build", source: "production_observation" },
    capabilities: ["apk_install", "shared_storage_write"],
    deferredWorkflows: [],
  },
  promotable: true,
  nonPromotableReason: null,
};

const enabledStatus: QualificationModeStatus = {
  enabled: true,
  recordable: true,
  message: null,
  build: {
    appVersion: "0.1.0",
    gitCommit: "1".repeat(40),
    materialBuildDigest: `sha256:${"a".repeat(64)}`,
    realExecutionEnabled: true,
    qualificationContract: 1,
  },
  runtimeContract: "real-execution-v1",
  workflows: [],
  targets: [],
  resumableCandidates: [targetPreview],
};

test("qualification TypeScript DTOs preserve provenance and build identity", () => {
  assert.equal(enabledStatus.build?.realExecutionEnabled, true);
  assert.equal(enabledStatus.resumableCandidates[0].target?.connectionType.value, "usb3");
  assert.equal(
    enabledStatus.resumableCandidates[0].target?.connectionType.source,
    "operator_attestation",
  );
  assert.equal(enabledStatus.runtimeContract, "real-execution-v1");
});

test("qualification API exposes only opaque candidate operations", () => {
  const apiSource = readFileSync(new URL("../src/api.ts", import.meta.url), "utf8");
  for (const command of [
    "get_device_qualification_mode_status",
    "create_qualification_target_candidate",
    "register_qualification_target",
    "discard_qualification_candidate",
  ]) {
    assert.equal(apiSource.includes(command), true, command);
  }
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
