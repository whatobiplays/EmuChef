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
    deviceSelectionLocked: false,
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
    phase: "executionActive",
    runValidity: "valid",
    qualificationOutcome: "not_observed",
    recordable: true,
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
    runCandidates: [],
    intentLock: null,
    deviceSelectionLocked: false,
    busy: false,
    error: null,
    refresh: vi.fn().mockResolvedValue(undefined),
    beginSession: vi.fn().mockResolvedValue(undefined),
    createTargetCandidate: vi.fn().mockResolvedValue(undefined),
    registerTarget: vi.fn().mockResolvedValue(undefined),
    recordCheckpoint: vi.fn().mockResolvedValue(undefined),
    abandonSession: vi.fn().mockResolvedValue(undefined),
    recordRun: vi.fn().mockResolvedValue(undefined),
    discardCandidate: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

test("the overlay is absent unless qualification mode is enabled", () => {
  const { rerender } = render(
    <DeviceQualificationOverlay controller={controller({ status: { ...status(), enabled: false } })} />,
  );
  expect(screen.queryByTestId("device-qualification-overlay")).toBeNull();

  rerender(<DeviceQualificationOverlay controller={controller()} />);
  expect(screen.getByRole("heading", { name: "Device qualification mode" })).toBeTruthy();
});

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

test("recording a run and abandoning an attempt always require an explicit click", () => {
  const current = controller({ session: session() });

  render(<DeviceQualificationOverlay controller={current} />);

  expect(current.recordRun).not.toHaveBeenCalled();
  expect(current.abandonSession).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Record qualification run" }));
  expect(current.recordRun).toHaveBeenCalledTimes(1);
  expect(current.recordRun).toHaveBeenCalledWith("candidate-opaque");

  fireEvent.click(screen.getByRole("button", { name: "Abandon qualification attempt" }));
  expect(current.abandonSession).toHaveBeenCalledTimes(1);
});

test("operator actions are disabled while the controller is busy", () => {
  const current = controller({ session: session(), busy: true });

  render(<DeviceQualificationOverlay controller={current} />);

  for (const name of [
    "Record qualification run",
    "Abandon qualification attempt",
    "Refresh qualification status",
  ]) {
    expect((screen.getByRole("button", { name }) as HTMLButtonElement).disabled).toBe(true);
  }
});

test("terminal-awaiting-evidence is presented before a candidate is materialized", () => {
  const current = controller({
    session: session({
      phase: "terminalAwaitingEvidence",
      candidate: null,
      recordedCheckpoints: [],
      humanCheckpoints: [{
        id: "device_state_verified",
        instruction: "Confirm the device state after the run.",
        fact: "The device is in the expected state.",
        allowedOutcomes: ["pass", "fail", "unable_to_verify"],
        required: true,
      }],
    }),
  });

  render(<DeviceQualificationOverlay controller={current} />);

  expect(screen.getByText("Terminal execution retained — awaiting required evidence")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Record qualification run" })).toBeNull();
  expect(screen.getByRole("radio", { name: "Pass" })).toBeTruthy();
});

test("invalid attempts show the backend explanation and non-recordable state", () => {
  const current = controller({
    session: session({
      phase: "executionPending",
      runValidity: "invalid",
      qualificationOutcome: "not_observed",
      recordable: false,
      invalidReason: "The observed device identity changed during the attempt.",
    }),
  });

  render(<DeviceQualificationOverlay controller={current} />);

  expect(screen.getByText("Invalid qualification run — not product evidence")).toBeTruthy();
  expect(screen.getByText("The observed device identity changed during the attempt.")).toBeTruthy();
  expect(
    screen.getByText("This attempt can no longer be recorded as qualification evidence."),
  ).toBeTruthy();
});

test("a valid attempt never claims an invalidation it was not given", () => {
  const current = controller({ session: session({ runValidity: "valid" }) });

  render(<DeviceQualificationOverlay controller={current} />);

  expect(screen.queryByText("Invalid qualification run — not product evidence")).toBeNull();
  expect(
    screen.queryByText("This attempt can no longer be recorded as qualification evidence."),
  ).toBeNull();
});

test("invalid and failed terminal classifications remain distinct", () => {
  const invalid = controller({
    session: session({
      runValidity: "invalid",
      qualificationOutcome: "not_observed",
      recordable: false,
      invalidReason: "The device is no longer available.",
    }),
  });
  const failed = controller({ session: session({ qualificationOutcome: "failed" }) });

  const { rerender } = render(<DeviceQualificationOverlay controller={invalid} />);
  expect(screen.getByText("Invalid qualification run — not product evidence")).toBeTruthy();

  rerender(<DeviceQualificationOverlay controller={failed} />);
  expect(screen.getByText("Product qualification failure")).toBeTruthy();
});

test("refresh is an explicit operator read of sanitized status", () => {
  const current = controller();

  render(<DeviceQualificationOverlay controller={current} />);

  fireEvent.click(screen.getByRole("button", { name: "Refresh qualification status" }));
  expect(current.refresh).toHaveBeenCalledTimes(1);
});

test("a resumable target candidate renders stored values and provenance", () => {
  const current = controller({ targetCandidate: targetCandidate() });

  render(<DeviceQualificationOverlay controller={current} />);

  expect(screen.getByText("Konkr Pocket Fit")).toBeTruthy();
  expect(screen.getAllByText(/production_observation/).length).toBeGreaterThan(0);
  expect(screen.getByText(/explicit_root_check/)).toBeTruthy();
});

test("persisted qualification-run candidates remain actionable without a session", () => {
  const runCandidates = [
    {
      candidateHandle: "run-one",
      kind: "qualification_run" as const,
      capturedAt: "2026-10-02T10:00:00Z",
      promotable: true,
      nonPromotableReason: null,
      runValidity: "valid" as const,
      qualificationOutcome: "passed" as const,
    },
    {
      candidateHandle: "run-two",
      kind: "qualification_run" as const,
      capturedAt: "2026-10-02T11:00:00Z",
      promotable: false,
      nonPromotableReason: "Qualification source state is not clean.",
      runValidity: "invalid" as const,
      qualificationOutcome: "not_observed" as const,
    },
  ];
  const current = controller({ runCandidates });

  render(<DeviceQualificationOverlay controller={current} />);

  expect(screen.getAllByTestId("qualification-run-candidate").length).toBe(2);
  expect(screen.getAllByRole("button", { name: "Record qualification run" }).length).toBe(2);
  expect((screen.getAllByRole("button", { name: "Record qualification run" })[1] as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getAllByRole("button", { name: "Record qualification run" })[0]);
  expect(current.recordRun).toHaveBeenCalledWith("run-one");
  fireEvent.click(screen.getAllByRole("button", { name: "Discard candidate" })[1]);
  expect(current.discardCandidate).toHaveBeenCalledWith("run-two");
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

test("the overlay never renders repository paths, serials, or raw backend text", () => {
  const current = controller({
    session: session({
      runValidity: "invalid",
      recordable: false,
      invalidReason: "Qualification evidence was invalidated before it could be recorded.",
    }),
    error: "Qualification definitions are unavailable. Rebuild the qualification application.",
  });

  const { container } = render(<DeviceQualificationOverlay controller={current} />);
  const text = container.textContent ?? "";

  expect(/\/Users\/|\/private\/|sensitive-serial|adb output/i.test(text)).toBe(false);
  expect(screen.getByRole("alert").textContent).toBe(
    "Qualification definitions are unavailable. Rebuild the qualification application.",
  );
  expect(text).not.toContain("qualification_repository_unavailable");
});
