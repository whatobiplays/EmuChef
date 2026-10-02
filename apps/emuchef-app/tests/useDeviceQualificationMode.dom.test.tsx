import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { beforeEach, expect, test, vi } from "vitest";

const mockApi = vi.hoisted(() => ({
  abandonQualificationSession: vi.fn(),
  beginQualificationSession: vi.fn(),
  createQualificationTargetCandidate: vi.fn(),
  deviceQualificationModeStatus: vi.fn(),
  discardQualificationCandidate: vi.fn(),
  recordQualificationCheckpoint: vi.fn(),
  recordQualificationRun: vi.fn(),
  registerQualificationTarget: vi.fn(),
}));

vi.mock("../src/api", () => ({ api: mockApi }));

import { useDeviceQualificationMode } from "../src/useDeviceQualificationMode";
import type {
  QualificationModeStatus,
  QualificationSessionSnapshot,
} from "../src/types";
import { initialWorkflowState, type WorkflowState } from "../src/workflow";

function disabledStatus(): QualificationModeStatus {
  return {
    enabled: false,
    recordable: false,
    message: null,
    build: null,
    runtimeContract: null,
    workflows: [],
    targets: [],
    resumableCandidates: [],
  };
}

function activeStatus(
  overrides: Partial<QualificationModeStatus> = {},
): QualificationModeStatus {
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
    ...overrides,
  };
}

function sessionSnapshot(
  overrides: Partial<QualificationSessionSnapshot> = {},
): QualificationSessionSnapshot {
  return {
    sessionHandle: "session-opaque",
    targetId: "device-target-sha256:target",
    workflowId: "workflow.one",
    workflowVersion: 1,
    devicePlan: "plan.bound",
    requiredRecipes: ["recipe.one", "recipe.dependency"],
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

function reviewWorkflow(): WorkflowState {
  return {
    ...initialWorkflowState,
    step: "review",
    deviceHandle: "device-opaque",
    devicePlan: "plan.bound",
    selectedRecipes: ["recipe.one", "recipe.dependency"],
  };
}

function Harness({ workflow }: { workflow: WorkflowState }) {
  const workflowRef = useRef(workflow);
  workflowRef.current = workflow;
  const controller = useDeviceQualificationMode({ workflow, workflowRef });
  return (
    <>
      <output data-testid="qualification-active">{String(controller.intentLock !== null)}</output>
      <output data-testid="qualification-device-selection-locked">
        {controller.deviceSelectionLocked ? "locked" : "unlocked"}
      </output>
      <output data-testid="qualification-plan">{controller.intentLock?.devicePlan ?? ""}</output>
      <output data-testid="qualification-recipes">{controller.intentLock?.selectedRecipes.join(",") ?? ""}</output>
      <output data-testid="qualification-candidate">{controller.targetCandidate?.target.model.value ?? ""}</output>
      <output data-testid="qualification-checkpoint">{controller.session?.recordedCheckpoints[0]?.observedAt ?? ""}</output>
      <output data-testid="qualification-phase">{controller.session?.phase ?? ""}</output>
      <output data-testid="qualification-error">{controller.error ?? ""}</output>
      <button type="button" onClick={() => void controller.refresh()}>Refresh</button>
      <button
        type="button"
        onClick={() => void controller.createTargetCandidate("usb3")}
      >
        Capture target
      </button>
      <button
        type="button"
        onClick={() => void controller.beginSession({
          deviceHandle: "device-opaque",
          devicePlan: workflowRef.current.devicePlan ?? "plan.current",
          targetId: "device-target-sha256:target",
          workflowId: "workflow.one",
        })}
      >
        Begin session
      </button>
      <button
        type="button"
        onClick={() => void controller.recordCheckpoint("device_state_verified", "pass")}
      >
        Record checkpoint
      </button>
      <button type="button" onClick={() => void controller.abandonSession()}>Abandon</button>
      {controller.session?.candidate && (
        <button
          type="button"
          onClick={() => void controller.recordRun(controller.session!.candidate!.candidateHandle)}
        >
          Record session
        </button>
      )}
    </>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mockApi.deviceQualificationModeStatus.mockResolvedValue(disabledStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  mockApi.recordQualificationCheckpoint.mockResolvedValue(sessionSnapshot());
  mockApi.abandonQualificationSession.mockResolvedValue(
    sessionSnapshot({ runValidity: "invalid", recordable: false }),
  );
  mockApi.recordQualificationRun.mockResolvedValue({ runId: "qualification-run-opaque" });
});

test("disabled qualification mode leaves the normal workflow unconstrained", async () => {
  render(<Harness workflow={reviewWorkflow()} />);

  expect((await screen.findByTestId("qualification-active")).textContent).toBe("false");
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("unlocked");
  expect(mockApi.beginQualificationSession).not.toHaveBeenCalled();
  expect(mockApi.recordQualificationCheckpoint).not.toHaveBeenCalled();
  expect(mockApi.abandonQualificationSession).not.toHaveBeenCalled();
});

test("refresh restores the stored target candidate without recapturing it", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus({
    resumableCandidates: [{
      candidateHandle: "target-candidate-opaque",
      kind: "target_registration",
      capturedAt: "2026-08-23T09:00:00Z",
      promotable: true,
      nonPromotableReason: null,
      target: {
        profileId: { value: "profile.one", source: "production_observation" },
        manufacturer: { value: "Ayaneo", source: "production_observation" },
        model: { value: "Stored model", source: "production_observation" },
        androidVersion: { value: "14", source: "production_observation" },
        androidApi: { value: 34, source: "production_observation" },
        abiSocClass: { value: "arm64", source: "production_observation" },
        rootState: { value: "non_root", source: "explicit_root_check" },
        connectionType: { value: "usb3", source: "operator_attestation" },
        firmwareBuild: { value: "firmware-opaque", source: "production_observation" },
        capabilities: ["apk_install"],
        deferredWorkflows: [],
      },
    }],
  }));

  render(<Harness workflow={reviewWorkflow()} />);

  expect((await screen.findByTestId("qualification-candidate")).textContent).toBe("Stored model");
  expect(mockApi.createQualificationTargetCandidate).not.toHaveBeenCalled();
});

test("a restored attempt projects sanitized locks and checkpoints without trusted calls", async () => {
  const recordedAt = "2026-08-23T09:30:00Z";
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus({
    resumableSession: sessionSnapshot({
      phase: "executionActive",
      humanCheckpoints: [{
        id: "device_state_verified",
        instruction: "Confirm the device state.",
        fact: "The device is in the expected state.",
        allowedOutcomes: ["pass", "fail", "unable_to_verify"],
        required: true,
      }],
      recordedCheckpoints: [{
        checkpointId: "device_state_verified",
        outcome: "pass",
        observedAt: recordedAt,
      }],
    }),
  }));

  render(<Harness workflow={reviewWorkflow()} />);

  await waitFor(() => {
    expect(screen.getByTestId("qualification-active").textContent).toBe("true");
  });
  expect(screen.getByTestId("qualification-checkpoint").textContent).toBe(recordedAt);
  expect(screen.getByTestId("qualification-plan").textContent).toBe("plan.bound");
  expect(screen.getByTestId("qualification-recipes").textContent).toBe("recipe.one,recipe.dependency");
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("locked");
  expect(mockApi.beginQualificationSession).not.toHaveBeenCalled();
  expect(mockApi.recordQualificationCheckpoint).not.toHaveBeenCalled();
});

test("an active attempt exposes only its bound plan and recipes without starting product work", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  expect((await screen.findByTestId("qualification-active")).textContent).toBe("true");
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("locked");
  expect(screen.getByTestId("qualification-plan").textContent).toBe("plan.bound");
  expect(screen.getByTestId("qualification-recipes").textContent).toBe("recipe.one,recipe.dependency");
  expect(mockApi.beginQualificationSession).toHaveBeenCalledWith({
    deviceHandle: "device-opaque",
    devicePlan: "plan.bound",
    targetId: "device-target-sha256:target",
    workflowId: "workflow.one",
  });
  expect(mockApi.recordQualificationRun).not.toHaveBeenCalled();
});

test("candidate capture resolves the selected device from live operator intent", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.createQualificationTargetCandidate.mockResolvedValue({
    candidateHandle: "target-candidate-opaque",
    kind: "target_registration",
    capturedAt: "2026-08-23T09:00:00Z",
    target: {
      profileId: { value: "profile.one", source: "production_observation" },
      manufacturer: { value: "Ayaneo", source: "production_observation" },
      model: { value: "Captured model", source: "production_observation" },
      androidVersion: { value: "14", source: "production_observation" },
      androidApi: { value: 34, source: "production_observation" },
      abiSocClass: { value: "arm64", source: "production_observation" },
      rootState: { value: "non_root", source: "explicit_root_check" },
      connectionType: { value: "usb3", source: "operator_attestation" },
      firmwareBuild: { value: "firmware-opaque", source: "production_observation" },
      capabilities: ["apk_install"],
      deferredWorkflows: [],
    },
    promotable: true,
    nonPromotableReason: null,
  });

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Capture target" }));

  await waitFor(() => {
    expect(mockApi.createQualificationTargetCandidate).toHaveBeenCalledWith({
      deviceHandle: "device-opaque",
      devicePlan: "plan.bound",
      connectionType: "usb3",
    });
  });
  expect((await screen.findByTestId("qualification-candidate")).textContent).toBe("Captured model");
});

test("checkpoint recording forwards only the opaque handle and declared outcome", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus({
    resumableSession: sessionSnapshot(),
  }));

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("true");
  fireEvent.click(screen.getByRole("button", { name: "Record checkpoint" }));

  await waitFor(() => {
    expect(mockApi.recordQualificationCheckpoint).toHaveBeenCalledWith(
      "session-opaque",
      "device_state_verified",
      "pass",
    );
  });
});

test("abandoning an attempt closes it and releases the projected locks", async () => {
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus({ resumableSession: sessionSnapshot() }))
    .mockResolvedValue(activeStatus());

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("true");
  fireEvent.click(screen.getByRole("button", { name: "Abandon" }));

  await waitFor(() => {
    expect(mockApi.abandonQualificationSession).toHaveBeenCalledWith("session-opaque");
    expect(screen.getByTestId("qualification-active").textContent).toBe("false");
  });
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("unlocked");
});

test("a failed operator action surfaces a bounded error without changing locks", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockRejectedValue(new Error("qualification_target_unverified"));

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  await waitFor(() => {
    expect(screen.getByTestId("qualification-error").textContent).toBe("qualification_target_unverified");
  });
  expect(screen.getByTestId("qualification-active").textContent).toBe("false");
});

test("successful run recording clears the active qualification session", async () => {
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus({ resumableSession: sessionSnapshot() }))
    .mockResolvedValue(activeStatus());

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("true");

  fireEvent.click(screen.getByRole("button", { name: "Record session" }));

  await waitFor(() => expect(screen.getByTestId("qualification-active").textContent).toBe("false"));
  expect(screen.queryByRole("button", { name: "Record session" })).toBeNull();
  expect(mockApi.recordQualificationRun).toHaveBeenCalledTimes(1);
});

test("authoritative lifecycle progress appears only through a sanitized status refresh", async () => {
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus({ resumableSession: sessionSnapshot() }))
    .mockResolvedValueOnce(activeStatus({
      resumableSession: sessionSnapshot({
        phase: "terminalAwaitingEvidence",
        candidate: null,
        humanCheckpoints: [{
          id: "device_state_verified",
          instruction: "Confirm the device state.",
          fact: "The device is in the expected state.",
          allowedOutcomes: ["pass", "fail", "unable_to_verify"],
          required: true,
        }],
      }),
    }));

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("true");
  expect(screen.getByTestId("qualification-phase").textContent).toBe("executionActive");

  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

  await waitFor(() => {
    expect(screen.getByTestId("qualification-phase").textContent).toBe("terminalAwaitingEvidence");
  });
  expect(mockApi.beginQualificationSession).not.toHaveBeenCalled();
  expect(mockApi.recordQualificationCheckpoint).not.toHaveBeenCalled();
  expect(mockApi.abandonQualificationSession).not.toHaveBeenCalled();
});
