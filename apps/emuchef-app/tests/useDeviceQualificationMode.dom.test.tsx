import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  ExecutionEvent,
  QualificationModeStatus,
  QualificationSessionSnapshot,
  QualificationTargetCandidatePreview,
  RealExecutionSnapshot,
} from "../src/types";
import { initialWorkflowState, workflowReducer, type WorkflowState } from "../src/workflow";

function disabledStatus(): QualificationModeStatus {
  return {
    enabled: false,
    recordable: false,
    deviceSelectionLocked: false,
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

function realExecutionSnapshot(
  overrides: Partial<RealExecutionSnapshot> = {},
): RealExecutionSnapshot {
  return {
    executionHandle: "execution-opaque",
    reviewHandle: "review-opaque",
    simulated: false,
    verificationScope: "real_device",
    target: { label: "Connected Android device" },
    status: "running",
    startedAt: "2026-10-01T12:00:00Z",
    finishedAt: null,
    latestSequence: 1,
    terminal: false,
    recipes: [],
    warnings: [],
    errors: [],
    completion: {
      classification: "in_progress",
      counts: {
        total: 1,
        completed: 0,
        skipped: 0,
        blocked: 0,
        failed: 0,
        cancelled: 0,
        pending: 1,
      },
      warningCount: 0,
      partialChangesPossible: false,
      features: [],
    },
    progress: { currentFeature: null, currentAction: null },
    launchAction: null,
    ...overrides,
  };
}

function activeExecutionWorkflow(
  snapshot = realExecutionSnapshot(),
  events: ExecutionEvent[] = [],
  eventCursor = 0,
  baseWorkflow: WorkflowState = reviewWorkflow(),
): WorkflowState {
  return {
    ...baseWorkflow,
    step: "execution",
    executionGeneration: 1,
    execution: {
      kind: "active",
      generation: 1,
      mode: "real",
      snapshot,
      events,
      eventCursor,
      cancellationRequested: false,
    },
  };
}

function targetCandidate(
  candidateHandle: string,
  model: string,
): QualificationTargetCandidatePreview {
  const observed = <T,>(value: T) => ({ value, source: "production_observation" as const });
  return {
    candidateHandle,
    kind: "target_registration",
    capturedAt: "2026-08-23T09:00:00Z",
    target: {
      profileId: observed("profile.one"),
      manufacturer: observed("Ayaneo"),
      model: observed(model),
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

function targetCandidateSummary(
  candidate: QualificationTargetCandidatePreview,
): QualificationModeStatus["resumableCandidates"][number] {
  return {
    candidateHandle: candidate.candidateHandle,
    kind: "target_registration",
    capturedAt: candidate.capturedAt,
    promotable: candidate.promotable,
    nonPromotableReason: candidate.nonPromotableReason,
    target: candidate.target,
  };
}

function Harness({ workflow }: { workflow: WorkflowState }) {
  const workflowRef = useRef(workflow);
  workflowRef.current = workflow;
  const controller = useDeviceQualificationMode({ workflow, workflowRef });
  return (
    <>
      <output data-testid="qualification-active">{String(controller.intentLock !== null)}</output>
      <output data-testid="qualification-mode-enabled">{String(controller.status?.enabled ?? false)}</output>
      <output data-testid="qualification-session-present">
        {controller.session === null ? "absent" : "present"}
      </output>
      <output data-testid="qualification-device-selection-locked">
        {controller.deviceSelectionLocked ? "locked" : "unlocked"}
      </output>
      <output data-testid="qualification-plan">{controller.intentLock?.devicePlan ?? ""}</output>
      <output data-testid="qualification-recipes">{controller.intentLock?.selectedRecipes.join(",") ?? ""}</output>
      <output data-testid="qualification-candidate">{controller.targetCandidate?.target.model.value ?? ""}</output>
      <output data-testid="qualification-candidate-handle">{controller.targetCandidate?.candidateHandle ?? ""}</output>
      <output data-testid="qualification-run-candidates">{controller.runCandidates.map((candidate) => candidate.candidateHandle).join(",")}</output>
      <output data-testid="qualification-checkpoint">{controller.session?.recordedCheckpoints[0]?.observedAt ?? ""}</output>
      <output data-testid="qualification-phase">{controller.session?.phase ?? ""}</output>
      <output data-testid="qualification-error">{controller.error ?? ""}</output>
      <output data-testid="qualification-busy">{String(controller.busy)}</output>
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
      <button
        type="button"
        onClick={() => {
          const candidateHandle = controller.session?.candidate?.candidateHandle;
          if (candidateHandle) {
            void controller.recordRun(candidateHandle);
            void controller.recordRun(candidateHandle);
          }
        }}
      >
        Record session twice
      </button>
    </>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mockApi.deviceQualificationModeStatus.mockResolvedValue(disabledStatus());
  mockApi.beginQualificationSession.mockResolvedValue(sessionSnapshot());
  mockApi.recordQualificationCheckpoint.mockResolvedValue(sessionSnapshot());
  mockApi.abandonQualificationSession.mockResolvedValue(
    sessionSnapshot({ phase: "closed", runValidity: "invalid", recordable: false }),
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

test("execution event batches and same-phase snapshots do not poll qualification status", async () => {
  let workflow = activeExecutionWorkflow();
  const { rerender } = render(<Harness workflow={workflow} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(1));

  const event = (sequence: number): ExecutionEvent => ({
    sequence,
    timestamp: `2026-10-01T12:00:0${sequence}Z`,
    label: `Action ${sequence}`,
    status: "running",
    issue: null,
  });
  for (const sequence of [2, 3, 4]) {
    workflow = workflowReducer(workflow, {
      type: "execution-events",
      generation: 1,
      batch: {
        executionHandle: "execution-opaque",
        events: [event(sequence)],
        latestSequence: sequence,
        terminal: false,
      },
    });
    await act(async () => {
      rerender(<Harness workflow={workflow} />);
      await Promise.resolve();
    });
    workflow = workflowReducer(workflow, {
      type: "execution-snapshot",
      generation: 1,
      snapshot: realExecutionSnapshot({
        latestSequence: sequence,
        progress: { currentFeature: "feature.one", currentAction: `Action ${sequence}` },
      }),
    });
    await act(async () => {
      rerender(<Harness workflow={workflow} />);
      await Promise.resolve();
    });
  }

  expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(1);
});

test("device, facts, review, and execution lifecycle transitions refresh status", async () => {
  const { rerender } = render(<Harness workflow={initialWorkflowState} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(1));

  const selected = { ...initialWorkflowState, deviceHandle: "device-one" };
  rerender(<Harness workflow={selected} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(2));

  const probed = {
    ...selected,
    facts: {
      deviceHandle: "device-one",
      manufacturer: "Ayaneo",
      brand: null,
      model: "Pocket Fit",
      androidVersion: 14,
      androidApiLevel: 34,
      firmwareBuild: "build-one",
    },
  };
  rerender(<Harness workflow={probed} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(3));

  const reviewed: WorkflowState = {
    ...probed,
    review: {
      reviewHandle: "review-one",
    } as NonNullable<WorkflowState["review"]>,
  };
  rerender(<Harness workflow={reviewed} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(4));

  const starting: WorkflowState = {
    ...reviewed,
    executionGeneration: 1,
    execution: { kind: "starting", generation: 1, mode: "real" },
  };
  rerender(<Harness workflow={starting} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(5));

  const active = activeExecutionWorkflow(
    realExecutionSnapshot({ reviewHandle: "review-one" }),
    [],
    0,
    reviewed,
  );
  rerender(<Harness workflow={active} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(6));

  const activeExecution = active.execution;
  if (activeExecution.kind !== "active") throw new Error("active execution fixture must be active");
  const replacedExecutionReview: WorkflowState = {
    ...active,
    execution: {
      ...activeExecution,
      snapshot: realExecutionSnapshot({ reviewHandle: "review-replaced" }),
    },
  };
  rerender(<Harness workflow={replacedExecutionReview} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(7));

  const terminal: WorkflowState = {
    ...replacedExecutionReview,
    execution: {
      kind: "terminal",
      generation: active.executionGeneration,
      mode: "real",
      snapshot: realExecutionSnapshot({
        reviewHandle: "review-replaced",
        status: "succeeded",
        terminal: true,
        finishedAt: "2026-10-01T12:01:00Z",
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
      }),
      events: [],
      eventCursor: 0,
      cancellationRequested: false,
    },
  };
  rerender(<Harness workflow={terminal} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(8));
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

test("status refresh preserves the currently selected target candidate", async () => {
  let resolveRefresh!: (status: QualificationModeStatus) => void;
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus())
    .mockReturnValueOnce(new Promise((resolve) => { resolveRefresh = resolve; }));
  const latestCandidate = targetCandidate("zzz-latest", "Latest capture");
  mockApi.createQualificationTargetCandidate.mockResolvedValue(latestCandidate);
  const workflow = reviewWorkflow();
  const { rerender } = render(<Harness workflow={workflow} />);

  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("button", { name: "Capture target" }));
  await waitFor(() => {
    expect(screen.getByTestId("qualification-candidate-handle").textContent).toBe("zzz-latest");
  });

  rerender(<Harness workflow={{
    ...workflow,
    step: "execution",
    executionGeneration: 1,
    execution: { kind: "starting", generation: 1, mode: "real" },
  }} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(2));
  await act(async () => {
    resolveRefresh(activeStatus({
      resumableCandidates: [
        targetCandidateSummary(targetCandidate("aaa-earlier", "Earlier capture")),
        targetCandidateSummary(latestCandidate),
      ],
    }));
  });

  expect(screen.getByTestId("qualification-candidate-handle").textContent).toBe("zzz-latest");
  expect(screen.getByTestId("qualification-candidate").textContent).toBe("Latest capture");
});

test("a restored attempt projects sanitized locks and checkpoints without trusted calls", async () => {
  const recordedAt = "2026-08-23T09:30:00Z";
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus({
    deviceSelectionLocked: true,
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
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus())
    .mockResolvedValue(activeStatus({
      deviceSelectionLocked: true,
      resumableSession: sessionSnapshot(),
    }));

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-mode-enabled").textContent).toBe("true");
  });
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  await waitFor(() => {
    expect(screen.getByTestId("qualification-active").textContent).toBe("true");
  });
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

test("a successful begin keeps selection locked until status confirms association", async () => {
  let resolveStaleStatus!: (status: QualificationModeStatus) => void;
  let resolveCurrentStatus!: (status: QualificationModeStatus) => void;
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus())
    .mockReturnValueOnce(new Promise((resolve) => { resolveStaleStatus = resolve; }))
    .mockReturnValueOnce(new Promise((resolve) => { resolveCurrentStatus = resolve; }));

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-mode-enabled").textContent).toBe("true");
  });

  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));
  await waitFor(() => expect(mockApi.beginQualificationSession).toHaveBeenCalledTimes(1));
  await waitFor(() => {
    expect(screen.getByTestId("qualification-active").textContent).toBe("true");
    expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(2);
  });

  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("locked");

  await act(async () => {
    resolveStaleStatus(activeStatus({
      deviceSelectionLocked: false,
      resumableSession: sessionSnapshot(),
    }));
  });
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("locked");

  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(3));
  await act(async () => {
    resolveCurrentStatus(activeStatus({
      deviceSelectionLocked: true,
      resumableSession: sessionSnapshot(),
    }));
  });
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("locked");
});

test("a fresh status with no active session releases the temporary begin lock", async () => {
  let resolveNoSessionStatus!: (status: QualificationModeStatus) => void;
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus())
    .mockReturnValueOnce(new Promise((resolve) => { resolveNoSessionStatus = resolve; }));

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-mode-enabled").textContent).toBe("true");
  });

  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));
  await waitFor(() => {
    expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("locked");
    expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(2);
  });

  await act(async () => {
    resolveNoSessionStatus(activeStatus({
      deviceSelectionLocked: false,
      resumableSession: null,
    }));
  });

  await waitFor(() => {
    expect(screen.getByTestId("qualification-session-present").textContent).toBe("absent");
    expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("unlocked");
  });
});

test("an unrelated locked session cannot clear the pending begin lock", async () => {
  let resolveUnrelatedStatus!: (status: QualificationModeStatus) => void;
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus())
    .mockReturnValueOnce(new Promise((resolve) => { resolveUnrelatedStatus = resolve; }));

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-mode-enabled").textContent).toBe("true");
  });

  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));
  await waitFor(() => expect(mockApi.beginQualificationSession).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(2));
  await act(async () => {
    resolveUnrelatedStatus(activeStatus({
      deviceSelectionLocked: true,
      resumableSession: sessionSnapshot({ sessionHandle: "unrelated-session" }),
    }));
  });
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("locked");
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
  await waitFor(() => {
    expect(screen.getByTestId("qualification-mode-enabled").textContent).toBe("true");
  });
  fireEvent.click(screen.getByRole("button", { name: "Capture target" }));

  await waitFor(() => {
    expect(mockApi.createQualificationTargetCandidate).toHaveBeenCalledWith({
      deviceHandle: "device-opaque",
      devicePlan: "plan.bound",
      connectionType: "usb3",
    });
  });
  await waitFor(() => {
    expect(screen.getByTestId("qualification-candidate").textContent).toBe("Captured model");
  });
});

test("checkpoint recording forwards only the opaque handle and declared outcome", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus({
    deviceSelectionLocked: true,
    resumableSession: sessionSnapshot(),
  }));

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => expect(screen.getByTestId("qualification-active").textContent).toBe("true"));
  fireEvent.click(screen.getByRole("button", { name: "Record checkpoint" }));

  await waitFor(() => {
    expect(mockApi.recordQualificationCheckpoint).toHaveBeenCalledWith(
      "session-opaque",
      "device_state_verified",
      "pass",
    );
  });
});

test("a delayed checkpoint response cannot replace newer finalized status", async () => {
  let resolveCheckpoint!: (session: QualificationSessionSnapshot) => void;
  let resolveFollowUpStatus!: (status: QualificationModeStatus) => void;
  let statusCalls = 0;
  const finalizedStatus = activeStatus({
    resumableSession: null,
    resumableCandidates: [{
      candidateHandle: "finalized-run",
      kind: "qualification_run",
      capturedAt: "2026-10-03T10:00:00Z",
      promotable: true,
      nonPromotableReason: null,
      runValidity: "valid",
      qualificationOutcome: "passed",
    }],
  });
  mockApi.deviceQualificationModeStatus.mockImplementation(() => {
    statusCalls += 1;
    if (statusCalls === 1) return Promise.resolve(activeStatus({ resumableSession: sessionSnapshot() }));
    if (statusCalls === 2) return Promise.resolve(finalizedStatus);
    if (statusCalls === 3) return new Promise((resolve) => { resolveFollowUpStatus = resolve; });
    return Promise.resolve(finalizedStatus);
  });
  mockApi.recordQualificationCheckpoint.mockReturnValueOnce(
    new Promise((resolve) => { resolveCheckpoint = resolve; }),
  );

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-session-present").textContent).toBe("present");
  });
  fireEvent.click(screen.getByRole("button", { name: "Record checkpoint" }));
  await waitFor(() => expect(mockApi.recordQualificationCheckpoint).toHaveBeenCalledTimes(1));

  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => {
    expect(screen.getByTestId("qualification-run-candidates").textContent).toBe("finalized-run");
    expect(screen.getByTestId("qualification-session-present").textContent).toBe("absent");
  });

  await act(async () => {
    resolveCheckpoint(sessionSnapshot());
    await Promise.resolve();
  });
  expect(screen.getByTestId("qualification-run-candidates").textContent).toBe("finalized-run");
  expect(screen.getByTestId("qualification-session-present").textContent).toBe("absent");
  expect(statusCalls).toBe(3);
  await act(async () => resolveFollowUpStatus(finalizedStatus));
  await waitFor(() => expect(screen.getByTestId("qualification-busy").textContent).toBe("false"));
});

test("abandoning an attempt closes it and releases the projected locks", async () => {
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus({ deviceSelectionLocked: true, resumableSession: sessionSnapshot() }))
    .mockResolvedValue(activeStatus());

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => expect(screen.getByTestId("qualification-active").textContent).toBe("true"));
  fireEvent.click(screen.getByRole("button", { name: "Abandon" }));

  await waitFor(() => {
    expect(mockApi.abandonQualificationSession).toHaveBeenCalledWith("session-opaque");
    expect(screen.getByTestId("qualification-active").textContent).toBe("false");
  });
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("unlocked");
});

test("abandonment applies the returned closed snapshot even when status refresh fails", async () => {
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus({
      deviceSelectionLocked: true,
      resumableSession: sessionSnapshot(),
    }))
    .mockRejectedValueOnce(new Error("status temporarily unavailable"));

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-session-present").textContent).toBe("present");
  });
  fireEvent.click(screen.getByRole("button", { name: "Abandon" }));

  await waitFor(() => {
    expect(screen.getByTestId("qualification-phase").textContent).toBe("closed");
    expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("unlocked");
  });
  expect(mockApi.abandonQualificationSession).toHaveBeenCalledTimes(1);
});

test("a restored session awaiting device reassociation leaves product selection unlocked", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus({
    deviceSelectionLocked: false,
    resumableSession: sessionSnapshot(),
  }));

  render(<Harness workflow={reviewWorkflow()} />);

  await waitFor(() => expect(screen.getByTestId("qualification-session-present").textContent).toBe("present"));
  expect(screen.getByTestId("qualification-active").textContent).toBe("true");
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("unlocked");
  expect(screen.getByTestId("qualification-plan").textContent).toBe("plan.bound");
  expect(screen.getByTestId("qualification-recipes").textContent).toBe("recipe.one,recipe.dependency");
});

test("run candidates are projected independently of the resumable session", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus({
    resumableCandidates: [
      { candidateHandle: "run-one", kind: "qualification_run", capturedAt: "now", promotable: true, nonPromotableReason: null, runValidity: "valid", qualificationOutcome: "passed" },
      { candidateHandle: "run-two", kind: "qualification_run", capturedAt: "later", promotable: false, nonPromotableReason: "Source state is not clean.", runValidity: "invalid", qualificationOutcome: "not_observed" },
    ],
  }));

  render(<Harness workflow={reviewWorkflow()} />);

  await waitFor(() => {
    expect(screen.getByTestId("qualification-run-candidates").textContent).toBe("run-one,run-two");
  });
  expect(screen.getByTestId("qualification-active").textContent).toBe("false");
});

test("meaningful execution lifecycle transitions trigger a presentation-only status refresh", async () => {
  const initial = reviewWorkflow();
  const { rerender } = render(<Harness workflow={initial} />);
  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(1));

  rerender(<Harness workflow={{
    ...initial,
    step: "execution",
    executionGeneration: 1,
    execution: { kind: "starting", generation: 1, mode: "real" },
  }} />);

  await waitFor(() => expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(2));
  expect(mockApi.beginQualificationSession).not.toHaveBeenCalled();
  expect(mockApi.recordQualificationCheckpoint).not.toHaveBeenCalled();
});

test("a late earlier status refresh cannot replace the latest presentation", async () => {
  let resolveFirst!: (status: QualificationModeStatus) => void;
  let resolveSecond!: (status: QualificationModeStatus) => void;
  const first = new Promise<QualificationModeStatus>((resolve) => { resolveFirst = resolve; });
  const second = new Promise<QualificationModeStatus>((resolve) => { resolveSecond = resolve; });
  mockApi.deviceQualificationModeStatus.mockReturnValueOnce(first).mockReturnValueOnce(second);

  render(<Harness workflow={reviewWorkflow()} />);
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  expect(mockApi.deviceQualificationModeStatus).toHaveBeenCalledTimes(2);
  expect(screen.getByTestId("qualification-busy").textContent).toBe("true");

  const latest = activeStatus({
    resumableCandidates: [{
      candidateHandle: "latest-run",
      kind: "qualification_run",
      capturedAt: "latest",
      promotable: true,
      nonPromotableReason: null,
      runValidity: "valid",
      qualificationOutcome: "passed",
    }],
  });
  await act(async () => resolveSecond(latest));
  await waitFor(() => {
    expect(screen.getByTestId("qualification-run-candidates").textContent).toBe("latest-run");
  });
  expect(screen.getByTestId("qualification-busy").textContent).toBe("true");

  await act(async () => resolveFirst(activeStatus({
    resumableSession: sessionSnapshot(),
    resumableCandidates: [{
      candidateHandle: "stale-run",
      kind: "qualification_run",
      capturedAt: "stale",
      promotable: false,
      nonPromotableReason: "Stale presentation",
      runValidity: "invalid",
      qualificationOutcome: "not_observed",
    }],
  })));

  expect(screen.getByTestId("qualification-run-candidates").textContent).toBe("latest-run");
  expect(screen.getByTestId("qualification-session-present").textContent).toBe("absent");
  expect(screen.getByTestId("qualification-busy").textContent).toBe("false");
});

test("a failed operator action surfaces a bounded error without changing locks", async () => {
  mockApi.deviceQualificationModeStatus.mockResolvedValue(activeStatus());
  mockApi.beginQualificationSession.mockRejectedValue(new Error("qualification_target_unverified"));

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-mode-enabled").textContent).toBe("true");
  });
  fireEvent.click(screen.getByRole("button", { name: "Begin session" }));

  await waitFor(() => {
    expect(screen.getByTestId("qualification-error").textContent).toBe("qualification_target_unverified");
  });
  expect(screen.getByTestId("qualification-active").textContent).toBe("false");
  expect(screen.getByTestId("qualification-device-selection-locked").textContent).toBe("unlocked");
});

test("successful run recording clears the active qualification session", async () => {
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus({ deviceSelectionLocked: true, resumableSession: sessionSnapshot() }))
    .mockResolvedValue(activeStatus());

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-session-present").textContent).toBe("present");
  });

  fireEvent.click(screen.getByRole("button", { name: "Record session" }));

  await waitFor(() => expect(screen.getByTestId("qualification-active").textContent).toBe("false"));
  expect(screen.queryByRole("button", { name: "Record session" })).toBeNull();
  expect(mockApi.recordQualificationRun).toHaveBeenCalledTimes(1);
});

test("duplicate record requests for one candidate share an in-flight guard", async () => {
  let resolveRecord!: (result: { runId: string }) => void;
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus({ resumableSession: sessionSnapshot() }))
    .mockResolvedValue(activeStatus());
  mockApi.recordQualificationRun.mockReturnValueOnce(
    new Promise((resolve) => { resolveRecord = resolve; }),
  );

  render(<Harness workflow={reviewWorkflow()} />);
  await waitFor(() => {
    expect(screen.getByTestId("qualification-session-present").textContent).toBe("present");
  });
  fireEvent.click(screen.getByRole("button", { name: "Record session twice" }));

  expect(mockApi.recordQualificationRun).toHaveBeenCalledTimes(1);
  await act(async () => {
    resolveRecord({ runId: "qualification-run-opaque" });
    await Promise.resolve();
  });
  await waitFor(() => expect(screen.getByTestId("qualification-busy").textContent).toBe("false"));
});

test("authoritative lifecycle progress appears only through a sanitized status refresh", async () => {
  mockApi.deviceQualificationModeStatus
    .mockResolvedValueOnce(activeStatus({ deviceSelectionLocked: true, resumableSession: sessionSnapshot() }))
    .mockResolvedValueOnce(activeStatus({
      deviceSelectionLocked: true,
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
  await waitFor(() => {
    expect(screen.getByTestId("qualification-session-present").textContent).toBe("present");
  });
  expect(screen.getByTestId("qualification-phase").textContent).toBe("executionActive");

  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

  await waitFor(() => {
    expect(screen.getByTestId("qualification-phase").textContent).toBe("terminalAwaitingEvidence");
  });
  expect(mockApi.beginQualificationSession).not.toHaveBeenCalled();
  expect(mockApi.recordQualificationCheckpoint).not.toHaveBeenCalled();
  expect(mockApi.abandonQualificationSession).not.toHaveBeenCalled();
});
