import { act, fireEvent, render, screen } from "@testing-library/react";
import { useRef, type Dispatch } from "react";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mockApi = vi.hoisted(() => ({
  exportExecutionReport: vi.fn(),
}));

vi.mock("../src/api", () => ({ api: mockApi }));

import { useExecution } from "../src/useExecution";
import type { ExecutionSnapshot } from "../src/types";
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

function Harness({ workflow }: { workflow: WorkflowState }) {
  const workflowRef = useRef(workflow);
  const runtimeGenerationRef = useRef(1);
  const mainRef = useRef<HTMLElement | null>(null);
  workflowRef.current = workflow;

  const execution = useExecution({
    announce: vi.fn(),
    dispatch: vi.fn() as unknown as Dispatch<WorkflowAction>,
    mainRef,
    realExecutionCompiled: false,
    runtimeGenerationRef,
    setBusy: vi.fn(),
    setNotice: vi.fn(),
    withNativeDialogFocus: async <Result,>(action: () => Promise<Result>) => action(),
    workflow,
    workflowRef,
  });

  return (
    <button onClick={() => void execution.exportExecutionReport()}>
      {execution.reportState}
    </button>
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
