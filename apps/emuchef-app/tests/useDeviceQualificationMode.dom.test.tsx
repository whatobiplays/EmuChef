import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode, useRef } from "react";
import { beforeEach, expect, test, vi } from "vitest";

const mockApi = vi.hoisted(() => ({
  beginQualificationSession: vi.fn(),
  bindQualificationExecution: vi.fn(),
  bindQualificationReview: vi.fn(),
  createQualificationTargetCandidate: vi.fn(),
  deviceQualificationModeStatus: vi.fn(),
  discardQualificationCandidate: vi.fn(),
  finalizeQualificationCandidate: vi.fn(),
  recordQualificationCheckpoint: vi.fn(),
  recordQualificationRun: vi.fn(),
  refreshQualificationSession: vi.fn(),
  registerQualificationTarget: vi.fn(),
  createReview: vi.fn(),
  startRealExecution: vi.fn(),
}));

vi.mock("../src/api", () => ({ api: mockApi }));

import { useDeviceQualificationMode } from "../src/useDeviceQualificationMode";
import type {
  QualificationModeStatus,
  QualificationSessionSnapshot,
  RealExecutionSnapshot,
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

function activeStatus(): QualificationModeStatus {
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

function reviewWorkflow(): WorkflowState {
  return {
    ...initialWorkflowState,
    step: "review",
    deviceHandle: "device-opaque",
    devicePlan: "plan.bound",
    selectedRecipes: ["recipe.one", "recipe.dependency"],
    review: {
      reviewHandle: "review-opaque",
      setup: { name: "Current setup" },
      target: { label: "Connected Android device" },
      features: [],
      inputs: [],
      notices: [],
      work: { actionCount: 1 },
      canExecute: true,
    },
  };
}

function realTerminalWorkflow(): WorkflowState {
  const snapshot: RealExecutionSnapshot = {
    executionHandle: "execution-opaque",
    reviewHandle: "review-opaque",
    simulated: false,
    verificationScope: "real_device",
    target: { label: "Connected Android device" },
    status: "succeeded",
    startedAt: "2026-08-23T10:00:00Z",
    finishedAt: "2026-08-23T10:01:00Z",
    latestSequence: 1,
    terminal: true,
    recipes: [],
    warnings: [],
    errors: [],
    completion: {
      classification: "success",
      counts: {
        total: 1,
        completed: 1,
        skipped: 0,
        blocked: 0,
        failed: 0,
        cancelled: 0,
        pending: 0,
      },
      warningCount: 0,
      partialChangesPossible: false,
      features: [],
    },
    progress: { currentFeature: null, currentAction: null },
    launchAction: null,
    terminalPolicy: null,
    cancellation: null,
  };
  return {
    ...reviewWorkflow(),
    step: "execution",
    executionGeneration: 4,
    execution: {
      kind: "terminal",
      generation: 4,
      mode: "real",
      snapshot,
      events: [],
      eventCursor: 1,
      cancellationRequested: false,
    },
  };
}

function Harness({ workflow }: { workflow: WorkflowState }) {
  const workflowRef = useRef(workflow);
  workflowRef.current = workflow;
  const controller = useDeviceQualificationMode({ workflow, workflowRef });
  return (
    <>
      <output data-testid="qualification-active">{String(controller.intentLock !== null)}</output>
      <output data-testid="qualification-plan">{controller.intentLock?.devicePlan ?? ""}</output>
      <output data-testid="qualification-recipes">{controller.intentLock?.selectedRecipes.join(",") ?? ""}</output>
      <output data-testid="qualification-candidate">{controller.targetCandidate?.target.model.value ?? ""}</output>
      <output data-testid="qualification-checkpoint">{controller.session?.recordedCheckpoints[0]?.observedAt ?? ""}</output>
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
  mockApi.bindQualificationReview.mockResolvedValue(sessionSnapshot());
  mockApi.bindQualificationExecution.mockResolvedValue(sessionSnapshot());
  mockApi.finalizeQualificationCandidate.mockResolvedValue(sessionSnapshot());
});

test("disabled qualification mode leaves the normal workflow unconstrained", async () => {
  render(<Harness workflow={reviewWorkflow()} />);

  expect((await screen.findByTestId("qualification-active")).textContent).toBe("false");
  expect(mockApi.beginQualificationSession).not.toHaveBeenCalled();
  expect(mockApi.bindQualificationReview).not.toHaveBeenCalled();
  expect(mockApi.bindQualificationExecution).not.toHaveBeenCalled();
});

test("refresh restores the stored target candidate without recapturing it", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue({
    ...activeStatus(),
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
  });

  render(<Harness workflow={reviewWorkflow()} />);

  expect((await screen.findByTestId("qualification-candidate")).textContent).toBe("Stored model");
  expect(mockApi.createQualificationTargetCandidate).not.toHaveBeenCalled();
});

test("refresh restores a persisted run session and checkpoint timestamp without probing", async () => {
  const recordedAt = "2026-08-23T09:30:00Z";
  mockApi.deviceQualificationModeStatus.mockResolvedValue({
    ...activeStatus(),
    resumableSession: sessionSnapshot({
      humanCheckpoints: [{
        id: "clean-reset",
        instruction: "Reset the device before the first reviewed run.",
        fact: "The device is clean before execution.",
        allowedOutcomes: ["pass", "fail", "unable_to_verify"],
        required: true,
      }],
      recordedCheckpoints: [{
        checkpointId: "clean-reset",
        outcome: "pass",
        observedAt: recordedAt,
      }],
    }),
  });

  render(<Harness workflow={initialWorkflowState} />);

  await waitFor(() => {
    expect(screen.getByTestId("qualification-active").textContent).toBe("true");
  });
  expect(screen.getByTestId("qualification-checkpoint").textContent).toBe(recordedAt);
  expect(mockApi.beginQualificationSession).not.toHaveBeenCalled();
  expect(mockApi.refreshQualificationSession).not.toHaveBeenCalled();
});

test("an active session exposes only its bound plan and recipes without starting product work", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  expect((await screen.findByTestId("qualification-active")).textContent).toBe("true");
  expect(screen.getByTestId("qualification-plan").textContent).toBe("plan.bound");
  expect(screen.getByTestId("qualification-recipes").textContent).toBe("recipe.one,recipe.dependency");
  expect(mockApi.createReview).not.toHaveBeenCalled();
  expect(mockApi.startRealExecution).not.toHaveBeenCalled();
});

test("review and terminal execution observation is deduplicated under StrictMode", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());

  render(
    <StrictMode>
      <Harness workflow={realTerminalWorkflow()} />
    </StrictMode>,
  );
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  await waitFor(() => {
    expect(mockApi.bindQualificationReview).toHaveBeenCalledTimes(1);
    expect(mockApi.bindQualificationExecution).toHaveBeenCalledTimes(1);
    expect(mockApi.finalizeQualificationCandidate).toHaveBeenCalledTimes(1);
  });
  expect(mockApi.createReview).not.toHaveBeenCalled();
  expect(mockApi.startRealExecution).not.toHaveBeenCalled();
});

test("a failed review bind is retried without duplicate concurrent binds", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  mockApi.bindQualificationReview
    .mockRejectedValueOnce(new Error("temporary review bind failure"))
    .mockResolvedValue(sessionSnapshot());

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  await waitFor(() => expect(mockApi.bindQualificationReview).toHaveBeenCalledTimes(2));
  expect(mockApi.bindQualificationReview).toHaveBeenNthCalledWith(1, "session-opaque", "review-opaque");
  expect(mockApi.bindQualificationReview).toHaveBeenNthCalledWith(2, "session-opaque", "review-opaque");
});

test("execution waits for review binding retry success before binding and finalizing", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  let resolveReview: ((snapshot: QualificationSessionSnapshot) => void) | undefined;
  mockApi.bindQualificationReview
    .mockRejectedValueOnce(new Error("temporary review bind failure"))
    .mockImplementationOnce(() => new Promise((resolve) => {
      resolveReview = resolve;
    }));
  mockApi.bindQualificationExecution.mockResolvedValue(sessionSnapshot());
  mockApi.finalizeQualificationCandidate.mockResolvedValue(sessionSnapshot());

  render(<Harness workflow={realTerminalWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  await waitFor(() => {
    expect(mockApi.bindQualificationReview).toHaveBeenCalledTimes(2);
    expect(resolveReview).toBeTypeOf("function");
  });
  expect(mockApi.bindQualificationExecution).not.toHaveBeenCalled();
  expect(mockApi.finalizeQualificationCandidate).not.toHaveBeenCalled();

  resolveReview!(sessionSnapshot());

  await waitFor(() => {
    expect(mockApi.bindQualificationExecution).toHaveBeenCalledTimes(1);
    expect(mockApi.finalizeQualificationCandidate).toHaveBeenCalledTimes(1);
  });
  expect(mockApi.bindQualificationReview.mock.invocationCallOrder[1])
    .toBeLessThan(mockApi.bindQualificationExecution.mock.invocationCallOrder[0]);
});

test("failed terminal execution binding and finalization retry in order", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  mockApi.bindQualificationReview.mockResolvedValue(sessionSnapshot());
  mockApi.bindQualificationExecution
    .mockRejectedValueOnce(new Error("temporary execution bind failure"))
    .mockResolvedValue(sessionSnapshot());
  mockApi.finalizeQualificationCandidate
    .mockRejectedValueOnce(new Error("temporary finalization failure"))
    .mockResolvedValue(sessionSnapshot());

  render(<Harness workflow={realTerminalWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  await waitFor(() => {
    expect(mockApi.bindQualificationExecution).toHaveBeenCalledTimes(2);
    expect(mockApi.finalizeQualificationCandidate).toHaveBeenCalledTimes(2);
  });
  expect(mockApi.bindQualificationExecution.mock.invocationCallOrder[1])
    .toBeLessThan(mockApi.finalizeQualificationCandidate.mock.invocationCallOrder[0]);
});

test("device availability drift refreshes the bound qualification session", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  mockApi.refreshQualificationSession.mockResolvedValue(sessionSnapshot({
    runValidity: "invalid",
    invalidReason: "device_identity_changed",
  }));

  const { rerender } = render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));
  await screen.findByText("true");

  rerender(<Harness workflow={{
    ...reviewWorkflow(),
    step: "connect",
    deviceHandle: null,
    devicePlan: null,
    facts: null,
    review: null,
  }} />);

  await waitFor(() => expect(mockApi.refreshQualificationSession).toHaveBeenCalledTimes(1));
  expect(mockApi.refreshQualificationSession).toHaveBeenCalledWith("session-opaque", "device-opaque");
});

test("late device facts refresh the session and establish a later drift baseline", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  mockApi.refreshQualificationSession.mockResolvedValue(sessionSnapshot());
  const initial = reviewWorkflow();
  initial.facts = null;
  const lateFacts = {
    deviceHandle: "device-opaque",
    manufacturer: "Example",
    brand: "Example",
    model: "Original model",
    androidVersion: 14,
    androidApiLevel: 34,
    firmwareBuild: "firmware-original",
  };

  const { rerender } = render(<Harness workflow={initial} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));
  await screen.findByText("true");
  expect(mockApi.refreshQualificationSession).not.toHaveBeenCalled();

  rerender(<Harness workflow={{ ...initial, facts: lateFacts }} />);
  await waitFor(() => expect(mockApi.refreshQualificationSession).toHaveBeenCalledTimes(1));
  expect(mockApi.refreshQualificationSession).toHaveBeenLastCalledWith("session-opaque", "device-opaque");

  rerender(<Harness workflow={{
    ...initial,
    facts: { ...lateFacts, model: "Replacement model" },
  }} />);
  await waitFor(() => expect(mockApi.refreshQualificationSession).toHaveBeenCalledTimes(2));
  expect(mockApi.refreshQualificationSession).toHaveBeenLastCalledWith("session-opaque", "device-opaque");

  rerender(<Harness workflow={{
    ...initial,
    devicePlan: "plan.changed",
    facts: { ...lateFacts, model: "Replacement model" },
  }} />);
  await waitFor(() => expect(mockApi.refreshQualificationSession).toHaveBeenCalledTimes(3));
  expect(mockApi.refreshQualificationSession).toHaveBeenLastCalledWith("session-opaque", "device-opaque");
});

test("device identity drift refreshes the bound qualification session", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  mockApi.refreshQualificationSession.mockResolvedValue(sessionSnapshot({
    runValidity: "invalid",
    invalidReason: "device_identity_changed",
  }));
  const initial = reviewWorkflow();
  initial.facts = {
    deviceHandle: "device-opaque",
    manufacturer: "Example",
    brand: "Example",
    model: "Original model",
    androidVersion: 14,
    androidApiLevel: 34,
    firmwareBuild: "firmware-original",
  };

  const { rerender } = render(<Harness workflow={initial} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));
  await screen.findByText("true");

  rerender(<Harness workflow={{
    ...initial,
    facts: { ...initial.facts!, model: "Replacement model" },
  }} />);

  await waitFor(() => expect(mockApi.refreshQualificationSession).toHaveBeenCalledTimes(1));
  expect(mockApi.refreshQualificationSession).toHaveBeenCalledWith("session-opaque", "device-opaque");

  rerender(<Harness workflow={{
    ...initial,
    devicePlan: "plan.changed",
    facts: { ...initial.facts!, model: "Replacement model" },
  }} />);

  await waitFor(() => expect(mockApi.refreshQualificationSession).toHaveBeenCalledTimes(2));
  expect(mockApi.refreshQualificationSession).toHaveBeenLastCalledWith("session-opaque", "device-opaque");
});

test("successful run recording clears the active qualification session", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  mockApi.recordQualificationRun.mockResolvedValue({ runId: "qualification-run-opaque" });

  render(<Harness workflow={reviewWorkflow()} />);
  await screen.findByText("false");
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));
  await screen.findByText("true");

  fireEvent.click(screen.getByRole("button", { name: "Record session" }));

  await waitFor(() => expect(screen.getByTestId("qualification-active").textContent).toBe("false"));
  expect(screen.queryByRole("button", { name: "Record session" })).toBeNull();
  expect(mockApi.recordQualificationRun).toHaveBeenCalledTimes(1);
});
