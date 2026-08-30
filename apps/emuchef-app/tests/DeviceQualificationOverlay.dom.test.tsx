import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";

import { DeviceQualificationOverlay } from "../src/DeviceQualificationOverlay";
import type { DeviceQualificationModeController } from "../src/useDeviceQualificationMode";
import type {
  QualificationModeStatus,
  QualificationSessionSnapshot,
  QualificationTargetCandidatePreview,
} from "../src/types";

function status(): QualificationModeStatus {
  return {
    enabled: true,
    recordable: true,
    message: null,
    build: {
      appVersion: "0.1.0",
      gitCommit: "commit-opaque",
      materialBuildDigest: "digest-opaque",
      realExecutionEnabled: true,
      qualificationContract: 2,
    },
    runtimeContract: "runtime-contract-2",
    workflows: [],
    targets: [],
    resumableCandidates: [],
  };
}

function session(
  overrides: Partial<QualificationSessionSnapshot> = {},
): QualificationSessionSnapshot {
  return {
    sessionHandle: "session-opaque",
    targetId: "device-target-sha256:target",
    workflowId: "workflow.one",
    workflowVersion: 1,
    devicePlan: "plan.bound",
    requiredRecipes: ["recipe.one"],
    humanCheckpoints: [],
    recordedCheckpoints: [],
    runValidity: "valid",
    qualificationOutcome: "not_observed",
    invalidReason: null,
    candidate: {
      candidateHandle: "candidate-opaque",
      kind: "qualification_run",
      capturedAt: "2026-08-23T10:00:00Z",
      promotable: true,
      nonPromotableReason: null,
      runValidity: "valid",
      qualificationOutcome: "not_observed",
    },
    ...overrides,
  };
}

function targetCandidate(): QualificationTargetCandidatePreview {
  const observed = <T,>(value: T) => ({ value, source: "production_observation" as const });
  return {
    candidateHandle: "target-candidate-opaque",
    kind: "target_registration",
    capturedAt: "2026-08-23T09:00:00Z",
    target: {
      profileId: observed("profile.one"),
      manufacturer: observed("Ayaneo"),
      model: observed("Konkr Pocket Fit"),
      androidVersion: observed("14"),
      androidApi: observed(34),
      abiSocClass: observed("arm64"),
      rootState: { value: "non_root", source: "explicit_root_check" },
      connectionType: { value: "usb3", source: "operator_attestation" },
      firmwareBuild: observed("firmware-opaque"),
      capabilities: ["apk_install"],
      deferredWorkflows: [],
    },
    promotable: true,
    nonPromotableReason: null,
  };
}

function controller(
  overrides: Partial<DeviceQualificationModeController> = {},
): DeviceQualificationModeController {
  return {
    status: status(),
    session: null,
    targetCandidate: null,
    intentLock: null,
    deviceSelectionLocked: false,
    busy: false,
    error: null,
    refresh: vi.fn().mockResolvedValue(undefined),
    beginSession: vi.fn().mockResolvedValue(undefined),
    createTargetCandidate: vi.fn().mockResolvedValue(undefined),
    registerTarget: vi.fn().mockResolvedValue(undefined),
    recordCheckpoint: vi.fn().mockResolvedValue(undefined),
    recordRun: vi.fn().mockResolvedValue(undefined),
    discardCandidate: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

test("declared checkpoints have no default outcome", () => {
  const current = controller({
    session: session({
      humanCheckpoints: [{
        id: "clean-reset",
        instruction: "Reset the device before the first reviewed run.",
        fact: "The device is clean before execution.",
        allowedOutcomes: ["pass", "fail", "unable_to_verify"],
        required: true,
      }],
    }),
  });

  render(<DeviceQualificationOverlay controller={current} />);

  expect((screen.getByRole("radio", { name: "Pass" }) as HTMLInputElement).checked).toBe(false);
  expect((screen.getByRole("radio", { name: "Fail" }) as HTMLInputElement).checked).toBe(false);
  expect((screen.getByRole("radio", { name: "Unable to verify" }) as HTMLInputElement).checked).toBe(false);
  expect(current.recordCheckpoint).not.toHaveBeenCalled();
});

test("recording a run always requires an explicit click", () => {
  const current = controller({ session: session() });

  render(<DeviceQualificationOverlay controller={current} />);

  expect(current.recordRun).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Record qualification run" }));
  expect(current.recordRun).toHaveBeenCalledTimes(1);
  expect(current.recordRun).toHaveBeenCalledWith("candidate-opaque");
});

test("invalid and failed terminal classifications remain distinct", () => {
  const invalid = controller({ session: session({ runValidity: "invalid", qualificationOutcome: "not_observed" }) });
  const failed = controller({ session: session({ qualificationOutcome: "failed" }) });

  const { rerender } = render(<DeviceQualificationOverlay controller={invalid} />);
  expect(screen.getByText("Invalid qualification run — not product evidence")).toBeTruthy();

  rerender(<DeviceQualificationOverlay controller={failed} />);
  expect(screen.getByText("Product qualification failure")).toBeTruthy();
});

test("a resumable target candidate renders stored values and provenance", () => {
  const current = controller({ targetCandidate: targetCandidate() });

  render(<DeviceQualificationOverlay controller={current} />);

  expect(screen.getByText("Konkr Pocket Fit")).toBeTruthy();
  expect(screen.getAllByText(/production_observation/).length).toBeGreaterThan(0);
  expect(screen.getByText(/explicit_root_check/)).toBeTruthy();
});

test("persisted checkpoint outcomes are displayed without a new timestamp", () => {
  const current = controller({
    session: session({
      humanCheckpoints: [{
        id: "clean-reset",
        instruction: "Reset the device before the first reviewed run.",
        fact: "The device is clean before execution.",
        allowedOutcomes: ["pass", "fail", "unable_to_verify"],
        required: true,
      }],
      recordedCheckpoints: [{
        checkpointId: "clean-reset",
        outcome: "fail",
        observedAt: "2026-08-23T09:30:00Z",
      }],
    }),
  });

  render(<DeviceQualificationOverlay controller={current} />);

  expect((screen.getByRole("radio", { name: "Fail" }) as HTMLInputElement).checked).toBe(true);
  expect((screen.getByRole("radio", { name: "Pass" }) as HTMLInputElement).checked).toBe(false);
  expect(screen.getByText(/2026-08-23T09:30:00Z/)).toBeTruthy();
  expect(current.recordCheckpoint).not.toHaveBeenCalled();
});
