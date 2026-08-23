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

function sessionSnapshot(): QualificationSessionSnapshot {
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
  };
}

function reviewWorkflow(): WorkflowState {
  return {
    ...initialWorkflowState,
    step: "review",
    deviceHandle: "device-opaque",
    devicePlan: "plan.current",
    selectedRecipes: ["recipe.current"],
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
      <button
        type="button"
        onClick={() => void controller.beginSession({
          deviceHandle: "device-opaque",
          devicePlan: "plan.current",
          targetId: "device-target-sha256:target",
          workflowId: "workflow.one",
        })}
      >
        Begin session
      </button>
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
