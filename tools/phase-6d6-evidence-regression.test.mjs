import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  canonicalDigest,
  evidenceRecordDigest,
  scenarioContractFor,
  validateEvidenceManifest,
  validateUiBackendBinding,
} from "./phase-6d6-evidence.mjs";

const templatePath = fileURLToPath(new URL("../docs/testing/phase-6d6/evidence-template.json", import.meta.url));
const template = JSON.parse(readFileSync(templatePath, "utf8"));
const physicalHarnessPath = fileURLToPath(new URL(
  "../crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs",
  import.meta.url,
));


function cancellationAttempt(outcome, digit) {
  const identity = digit.repeat(64);
  const record = structuredClone(template);
  record.outcome = outcome;
  record.runId = `physical-run-sha256:${identity}`;
  record.evidencePath = `docs/testing/phase-6d6/evidence/cancellation_active-rep1-attempt-${digit}.json`;
  record.tracePath = `docs/testing/phase-6d6/evidence/traces/cancellation_active-rep1-attempt-${digit}.json`;
  record.trace = { events: [`attempt-${digit}`] };
  record.traceDigest = `sha256:${canonicalDigest(record.trace)}`;
  record.scenarioFacts.runScope = `run-scope-sha256:${identity}`;
  record.activeSlotObservation.runId = record.scenarioFacts.runScope;
  record.activeSlotObservation.executionId = `execution-sha256:${identity}`;
  record.activeProcess.runId = record.scenarioFacts.runScope;
  record.activeProcess.operationId = `operation-sha256:${identity}`;
  record.activeProcess.childIdentity = `child-sha256:${identity}`;
  record.sentinel.sentinelId = `sentinel-sha256:${identity}`;
  record.sentinel.nonce = `nonce-sha256:${identity}`;
  record.sentinel.runId = record.runId;
  record.activeCancellation = outcome === "passed" ? {
    requestPhase: "in_flight",
    inFlightObservedAt: "unix:1",
    requestedAt: "unix:2",
    operationFinishedAt: "unix:3",
    requestBeforeFinished: true,
    laterWorkNotAttempted: true,
    operatorEvidence: "operator acknowledged the in-flight checkpoint",
  } : null;
  record.recordDigest = evidenceRecordDigest(record);
  return record;
}

test("low-storage preflight uses the header-aware Available KiB parser", () => {
  const source = readFileSync(physicalHarnessPath, "utf8");
  const parser = source.match(/fn parse_available_kib[\s\S]*?\n}\n\nfn free_space_kib/)?.[0];
  const body = source.match(/fn free_space_kib[\s\S]*?\n}\n/)?.[0];
  assert.ok(parser, "parse_available_kib must remain present");
  assert.ok(body, "free_space_kib must remain present");
  assert.match(parser, /"Available" \| "Avail"/, "the parser must locate either supported available-block header");
  assert.match(body, /parse_available_kib\(&output\)/, "free_space_kib must delegate to the header-aware parser");
  assert.doesNotMatch(body, /\.nth\(2\)/, "free_space_kib must not read the Used KiB column");
});

test("failed attempts remain auditable and do not block a later passing repetition", () => {
  const failed = cancellationAttempt("failed", "1");
  const passed = cancellationAttempt("passed", "2");
  const result = validateEvidenceManifest([failed, passed]);
  assert.ok(!result.missing.includes("cancellation_active:1"));
  assert.throws(
    () => validateEvidenceManifest([passed, cancellationAttempt("passed", "3")]),
    /duplicate passing evidence repetition/,
  );
});

test("supported active scenarios require host-push evidence without changing timeout", () => {
  for (const scenario of [
    "cancellation_active",
    "usb_disconnect_active",
    "device_offline",
    "device_unauthorized",
  ]) {
    assert.equal(scenarioContractFor(scenario).activeProcess.operationClass, "host_push");
  }
});

test("operation timeout requires exact live target-child evidence", () => {
  const contract = scenarioContractFor("operation_timeout");
  assert.deepEqual(contract.activeProcess, {
    required: true,
    operationClass: "device_copy",
    actionMustPrecedeTerminal: true,
    exactRunBinding: true,
  });
});

test("UI smoke binding requires a passing matching physical scenario and issue", () => {
  const subcase = {
    name: "transport",
    backendRunId: `physical-run-sha256:${"4".repeat(64)}`,
    backendTraceDigest: `sha256:${"5".repeat(64)}`,
    backendIssueCode: "device_transport_lost",
  };
  const physical = {
    runId: subcase.backendRunId,
    traceDigest: subcase.backendTraceDigest,
    outcome: "passed",
    scenario: "usb_disconnect_active",
    observedIssueCode: "device_transport_lost",
  };
  assert.doesNotThrow(() => validateUiBackendBinding(subcase, physical));
  assert.throws(
    () => validateUiBackendBinding(subcase, { ...physical, outcome: "failed" }),
    /passing physical backend record/,
  );
  assert.throws(
    () => validateUiBackendBinding(subcase, { ...physical, scenario: "device_unauthorized" }),
    /wrong physical scenario category/,
  );
  assert.throws(
    () => validateUiBackendBinding(subcase, { ...physical, observedIssueCode: "device_offline" }),
    /issue code does not match/,
  );
});
