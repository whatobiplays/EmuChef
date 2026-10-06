import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useRef, type Dispatch } from "react";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mockApi = vi.hoisted(() => ({
  exportExecutionReport: vi.fn(),
  startRealExecution: vi.fn(),
  startSimulatedExecution: vi.fn(),
}));

vi.mock("../src/api", () => ({ api: mockApi }));

import { useExecution } from "../src/useExecution";
import type { DeviceQualificationSnapshot, ExecutionSnapshot } from "../src/types";
import type { WorkflowAction, WorkflowState } from "../src/workflow";

function terminalSnapshot(executionHandle: string, latestSequence = 1): ExecutionSnapshot {
  return {
    executionHandle,
    reviewHandle: "review-opaque",
    simulated: true,
    verificationScope: "simulation_only",
    status: "failed",
    startedAt: "2026-07-20T12:00:00Z",
    finishedAt: "2026-07-20T12:00:01Z",
    latestSequence,
    terminal: true,
    recipes: [],
    warnings: [],
    errors: [],
    progress: { currentFeature: null, currentAction: null },
    completion: {
      classification: "failed",
      counts: {
        total: 1,
        completed: 0,
        skipped: 0,
        blocked: 0,
        failed: 1,
        cancelled: 0,
        pending: 0,
      },
      warningCount: 0,
      partialChangesPossible: false,
      features: [],
    },
  };
}

function terminalWorkflow(
  executionHandle: string,
  generation: number,
  latestSequence = 1,
): WorkflowState {
  return {
    step: "execution",
    deviceHandle: "device-opaque",
    facts: null,
    match: null,
    devicePlan: "plan.one",
    selectedRecipes: ["recipe.one"],
    bindings: {},
    description: null,
    descriptionDirty: false,
    review: null,
    reviewStale: false,
    requestGeneration: 0,
    executionGeneration: generation,
    execution: {
      kind: "terminal",
      generation,
      mode: "simulated",
      snapshot: terminalSnapshot(executionHandle, latestSequence),
      events: [],
      eventCursor: latestSequence,
      cancellationRequested: false,
    },
    repairIntent: false,
    portableIntentDirty: false,
    savedIntentLoaded: false,
    requiredReentryBindings: [],
    reconnectDeviceHandle: null,
    unsupportedAcknowledged: false,
  };
}

function reviewWorkflow(): WorkflowState {
  return {
    ...terminalWorkflow("unused", 0),
    step: "review",
    review: {
      reviewHandle: "review-opaque",
      setup: { name: "Qualification setup" },
      target: { label: "Connected Android device" },
      features: [],
      inputs: [],
      notices: [],
      work: { actionCount: 1 },
      canExecute: true,
    },
    execution: { kind: "idle" },
  };
}

function deferred<Result>(): {
  promise: Promise<Result>;
  resolve: (result: Result) => void;
} {
  let resolve!: (result: Result) => void;
  const promise = new Promise<Result>((resolver) => {
    resolve = resolver;
  });
  return { promise, resolve };
}

function Harness({
  workflow,
  qualification,
  dispatch = vi.fn() as unknown as Dispatch<WorkflowAction>,
  onRuntimeSessionLost = vi.fn(),
}: {
  workflow: WorkflowState;
  qualification?: DeviceQualificationSnapshot;
  dispatch?: Dispatch<WorkflowAction>;
  onRuntimeSessionLost?: () => void;
}) {
  const workflowRef = useRef(workflow);
  const runtimeGenerationRef = useRef(1);
  const mainRef = useRef<HTMLElement | null>(null);
  workflowRef.current = workflow;

  const execution = useExecution({
    announce: vi.fn(),
    dispatch,
    mainRef,
    onRuntimeSessionLost,
    realExecutionCompiled: qualification !== undefined,
    qualification,
    runtimeGenerationRef,
    setBusy: vi.fn(),
    setNotice: vi.fn(),
    withNativeDialogFocus: async <Result,>(action: () => Promise<Result>) => action(),
    workflow,
    workflowRef,
  });

  return qualification === undefined
    ? (
      <button onClick={() => void execution.exportExecutionReport()}>
        {execution.reportState}
      </button>
    )
    : (
      <>
        <button
          onClick={() => void execution.startRealExecution({
            phrase: "RUN",
            irreversibleChangesAcknowledged: true,
            noRollbackAcknowledged: true,
            keepDeviceConnectedAcknowledged: true,
          })}
        >
          start real execution
        </button>
        <button onClick={() => void execution.startSimulation()}>
          start simulated execution
        </button>
      </>
    );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("execution report export identity", () => {
  test("saved confirmation resets when a different execution report becomes active", async () => {
    mockApi.exportExecutionReport.mockResolvedValue({ outcome: "saved" });
    const { rerender } = render(<Harness workflow={terminalWorkflow("execution-one", 1)} />);

    fireEvent.click(screen.getByRole("button", { name: "idle" }));
    expect(await screen.findByRole("button", { name: "saved" })).toBeTruthy();

    rerender(<Harness workflow={terminalWorkflow("execution-two", 2)} />);
    expect(await screen.findByRole("button", { name: "idle" })).toBeTruthy();
  });

  test("an export completion from an older execution cannot mark the current report saved", async () => {
    const pending = deferred<{ outcome: "saved" }>();
    mockApi.exportExecutionReport.mockReturnValue(pending.promise);
    const { rerender } = render(<Harness workflow={terminalWorkflow("execution-one", 1)} />);

    fireEvent.click(screen.getByRole("button", { name: "idle" }));
    expect(await screen.findByRole("button", { name: "exporting" })).toBeTruthy();

    rerender(<Harness workflow={terminalWorkflow("execution-two", 2)} />);
    expect(await screen.findByRole("button", { name: "idle" })).toBeTruthy();

    await act(async () => {
      pending.resolve({ outcome: "saved" });
      await pending.promise;
    });

    expect(screen.getByRole("button", { name: "idle" })).toBeTruthy();
  });
});

test("unsupported qualification remains blocking in the React execution boundary", () => {
  const qualification: DeviceQualificationSnapshot = {
    state: "unsupported",
    summary: "This device is unsupported.",
    limitations: ["Android API level 30 or newer is required."],
    androidMajor: 10,
    androidApiLevel: 29,
    abiClass: "arm64",
    storage: "available",
    packageManager: "available",
    activityManager: "available",
    root: null,
    runtimeGeneration: 7,
    qualificationRevision: 9,
    deviceIdentity: "opaque-authority",
  };
  render(<Harness workflow={reviewWorkflow()} qualification={qualification} />);

  fireEvent.click(screen.getByRole("button", { name: "start real execution" }));

  expect(mockApi.startRealExecution).not.toHaveBeenCalled();
});

function supportedQualification(): DeviceQualificationSnapshot {
  return {
    state: "supported",
    summary: "This device is supported.",
    limitations: [],
    androidMajor: 15,
    androidApiLevel: 35,
    abiClass: "arm64",
    storage: "available",
    packageManager: "available",
    activityManager: "available",
    root: null,
    runtimeGeneration: 7,
    qualificationRevision: 9,
    deviceIdentity: "opaque-authority",
  };
}

describe("real start failure classification", () => {
  test("a lost runtime session clears stale projections and refreshes runtime state", async () => {
    mockApi.startRealExecution.mockRejectedValue(
      JSON.stringify({
        code: "runtime_session_lost",
        message: "The execution runtime session is no longer available.",
      }),
    );
    const dispatch = vi.fn();
    const onRuntimeSessionLost = vi.fn();
    render(
      <Harness
        dispatch={dispatch as unknown as Dispatch<WorkflowAction>}
        onRuntimeSessionLost={onRuntimeSessionLost}
        qualification={supportedQualification()}
        workflow={reviewWorkflow()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "start real execution" }));

    await waitFor(() => {
      expect(onRuntimeSessionLost).toHaveBeenCalledTimes(1);
    });
  expect(dispatch).toHaveBeenCalledWith({ type: "execution-start-failed", generation: 1 });
  // The runtime-invalidated dispatch belongs to the centralized runtime-loss
  // handler in the application shell, so this hook must not duplicate it.
  expect(dispatch).not.toHaveBeenCalledWith({ type: "runtime-invalidated" });
});

test("a simulated start that loses the runtime session invokes the runtime-loss handler", async () => {
  mockApi.startSimulatedExecution.mockRejectedValue(
    JSON.stringify({
      code: "runtime_session_lost",
      message: "The execution runtime session is no longer available.",
    }),
  );
  const dispatch = vi.fn();
  const onRuntimeSessionLost = vi.fn();
  render(
    <Harness
      dispatch={dispatch as unknown as Dispatch<WorkflowAction>}
      onRuntimeSessionLost={onRuntimeSessionLost}
      qualification={supportedQualification()}
      workflow={reviewWorkflow()}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: "start simulated execution" }));

  await waitFor(() => {
    expect(onRuntimeSessionLost).toHaveBeenCalledTimes(1);
  });
  expect(dispatch).toHaveBeenCalledWith({ type: "execution-start-failed", generation: 1 });
  expect(dispatch).not.toHaveBeenCalledWith({ type: "runtime-invalidated" });
});

test("an ordinary simulated start rejection does not invoke the runtime-loss handler", async () => {
  mockApi.startSimulatedExecution.mockRejectedValue(
    JSON.stringify({
      code: "review_unknown",
      message: "The reviewed plan is no longer available.",
    }),
  );
  const dispatch = vi.fn();
  const onRuntimeSessionLost = vi.fn();
  render(
    <Harness
      dispatch={dispatch as unknown as Dispatch<WorkflowAction>}
      onRuntimeSessionLost={onRuntimeSessionLost}
      qualification={supportedQualification()}
      workflow={reviewWorkflow()}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: "start simulated execution" }));

  await waitFor(() => {
    expect(dispatch).toHaveBeenCalledWith({ type: "execution-start-failed", generation: 1 });
  });
  expect(onRuntimeSessionLost).not.toHaveBeenCalled();
  expect(dispatch).not.toHaveBeenCalledWith({ type: "runtime-invalidated" });
});

  test("an ordinary start rejection neither resets the workflow nor refreshes runtime state", async () => {
    mockApi.startRealExecution.mockRejectedValue(
      JSON.stringify({
        code: "review_unknown",
        message: "The reviewed plan is no longer available.",
      }),
    );
    const dispatch = vi.fn();
    const onRuntimeSessionLost = vi.fn();
    render(
      <Harness
        dispatch={dispatch as unknown as Dispatch<WorkflowAction>}
        onRuntimeSessionLost={onRuntimeSessionLost}
        qualification={supportedQualification()}
        workflow={reviewWorkflow()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "start real execution" }));

    await waitFor(() => {
      expect(dispatch).toHaveBeenCalledWith({ type: "execution-start-failed", generation: 1 });
    });
    expect(onRuntimeSessionLost).not.toHaveBeenCalled();
    expect(dispatch).not.toHaveBeenCalledWith({ type: "runtime-invalidated" });
  });
});
