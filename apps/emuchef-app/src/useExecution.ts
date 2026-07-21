import { useCallback, useEffect, useRef, useState, type Dispatch } from "react";

import { api } from "./api";
import { errorCode, errorMessage } from "./app-helpers";
import {
  claimFocusTransition,
  executionAnnouncement,
  restoreAccessibleFocus,
} from "./accessibility";
import type { RealExecutionConfirmation } from "./types";
import type { WorkflowAction, WorkflowState } from "./workflow";

export type ExecutionReportState = "idle" | "exporting" | "saved" | "failed";
export type ExecutionLaunchState = "idle" | "launching" | "launched" | "failed";

function executionReportIdentity(execution: WorkflowState["execution"]): string | null {
  if (execution.kind !== "active" && execution.kind !== "terminal") return null;
  return `${execution.generation}:${execution.snapshot.executionHandle}:${execution.snapshot.latestSequence}`;
}

interface MutableValueRef<Value> {
  current: Value;
}

interface UseExecutionOptions {
  announce: (text: string, assertive?: boolean) => void;
  dispatch: Dispatch<WorkflowAction>;
  mainRef: MutableValueRef<HTMLElement | null>;
  realExecutionEnabled: boolean;
  runtimeGenerationRef: MutableValueRef<number>;
  setBusy: (busy: boolean) => void;
  setNotice: (notice: string | null) => void;
  withNativeDialogFocus: <Result>(
    action: () => Promise<Result>,
    preferred?: Array<HTMLElement | null | undefined>,
  ) => Promise<Result>;
  workflow: WorkflowState;
  workflowRef: MutableValueRef<WorkflowState>;
}

export function useExecution({
  announce,
  dispatch,
  mainRef,
  realExecutionEnabled,
  runtimeGenerationRef,
  setBusy,
  setNotice,
  withNativeDialogFocus,
  workflow,
  workflowRef,
}: UseExecutionOptions) {
  const [reportState, setReportState] = useState<ExecutionReportState>("idle");
  const [launchState, setLaunchState] = useState<ExecutionLaunchState>("idle");
  const announcementKeyRef = useRef<string | null>(null);
  const reportIdentityRef = useRef<string | null>(executionReportIdentity(workflow.execution));
  const reportIdentity = executionReportIdentity(workflow.execution);

  useEffect(() => {
    if (reportIdentityRef.current === reportIdentity) return;
    reportIdentityRef.current = reportIdentity;
    setReportState("idle");
  }, [reportIdentity]);

  const resetExecutionPresentation = useCallback(() => {
    reportIdentityRef.current = executionReportIdentity(workflowRef.current.execution);
    setReportState("idle");
    setLaunchState("idle");
    announcementKeyRef.current = null;
  }, [workflowRef]);

  const startSimulation = useCallback(async () => {
    const current = workflowRef.current;
    if (!current.review || current.execution.kind === "starting") return;
    const generation = current.executionGeneration + 1;
    dispatch({ type: "execution-starting", generation });
    setBusy(true);
    setNotice(null);
    announce("Starting the simulated dry run.");
    try {
      const snapshot = await api.startSimulatedExecution(current.review.reviewHandle);
      dispatch({ type: "execution-started", generation, snapshot });
    } catch (error) {
      dispatch({ type: "execution-start-failed", generation });
      setNotice(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }, [announce, dispatch, setBusy, setNotice, workflowRef]);

  const startRealExecution = useCallback(async (confirmation: RealExecutionConfirmation) => {
    const current = workflowRef.current;
    if (!realExecutionEnabled || !current.review || current.execution.kind === "starting") return;
    const generation = current.executionGeneration + 1;
    dispatch({ type: "execution-starting", generation, mode: "real" });
    setBusy(true);
    setNotice(null);
    try {
      const snapshot = await api.startRealExecution(current.review.reviewHandle, confirmation);
      dispatch({ type: "execution-started", generation, snapshot });
    } catch (error) {
      dispatch({ type: "execution-start-failed", generation });
      setNotice(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }, [dispatch, realExecutionEnabled, setBusy, setNotice, workflowRef]);

  useEffect(() => {
    if (workflow.execution.kind !== "active" && workflow.execution.kind !== "terminal") return;
    const next = executionAnnouncement(workflow.execution.snapshot, announcementKeyRef.current);
    if (!next) return;
    announcementKeyRef.current = next.key;
    announce(next.message, next.assertive);
    if (workflow.execution.kind === "terminal") {
      const generation = claimFocusTransition();
      queueMicrotask(() => restoreAccessibleFocus({
        preferred: [mainRef.current?.querySelector<HTMLElement>("[data-step-heading]")],
        generation,
      }));
    }
  }, [announce, mainRef, workflow.execution]);

  const activeExecution = workflow.execution.kind === "active" ? workflow.execution : null;

  useEffect(() => {
    if (!activeExecution) return;
    let disposed = false;
    let timer: number | null = null;
    const { generation, snapshot, mode } = activeExecution;
    const executionHandle = snapshot.executionHandle;
    let eventCursor = activeExecution.eventCursor;

    async function pollExecution() {
      try {
        const nextSnapshot = mode === "real"
          ? await api.getRealExecution(executionHandle)
          : await api.getSimulatedExecution(executionHandle);
        if (disposed) return;
        const currentExecution = workflowRef.current.execution;
        if (
          workflowRef.current.executionGeneration !== generation
          || (currentExecution.kind !== "active" && currentExecution.kind !== "terminal")
          || currentExecution.snapshot.executionHandle !== executionHandle
        ) {
          announce("An outdated execution response was ignored.");
          return;
        }
        dispatch({ type: "execution-snapshot", generation, snapshot: nextSnapshot });
        eventCursor = Math.max(eventCursor, nextSnapshot.latestSequence);
        if (nextSnapshot.terminal) return;

        const batch = mode === "real"
          ? await api.getRealExecutionEvents(executionHandle, eventCursor)
          : await api.getSimulatedExecutionEvents(executionHandle, eventCursor);
        if (disposed) return;
        dispatch({ type: "execution-events", generation, batch });
        for (const event of batch.events) eventCursor = Math.max(eventCursor, event.sequence);
        timer = window.setTimeout(pollExecution, 500);
      } catch (error) {
        if (disposed) return;
        if (errorCode(error) === "execution_unavailable") {
          dispatch({
            type: "execution-unavailable",
            generation,
            executionHandle,
            message: errorMessage(error),
          });
          return;
        }
        setNotice(errorMessage(error));
        timer = window.setTimeout(pollExecution, 1000);
      }
    }

    void pollExecution();
    return () => {
      disposed = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [
    activeExecution?.generation,
    activeExecution?.mode,
    activeExecution?.snapshot.executionHandle,
    announce,
    dispatch,
    setNotice,
    workflowRef,
  ]);

  const cancelExecution = useCallback(async () => {
    const current = workflowRef.current;
    if (current.execution.kind !== "active" || current.execution.cancellationRequested) return;
    const { generation, snapshot, mode } = current.execution;
    const runtimeGeneration = runtimeGenerationRef.current;
    try {
      const cancellation = mode === "real"
        ? await api.cancelRealExecution(snapshot.executionHandle)
        : await api.cancelSimulatedExecution(snapshot.executionHandle);
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      if (cancellation.accepted) {
        dispatch({ type: "execution-cancellation-requested", generation });
      }
    } catch (error) {
      if (runtimeGenerationRef.current === runtimeGeneration) setNotice(errorMessage(error));
    }
  }, [dispatch, runtimeGenerationRef, setNotice, workflowRef]);

  const exportExecutionReport = useCallback(async () => {
    const current = workflowRef.current;
    if (current.execution.kind !== "terminal") return;
    const exportIdentity = executionReportIdentity(current.execution);
    if (!exportIdentity) return;
    const executionHandle = current.execution.snapshot.executionHandle;
    const runtimeGeneration = runtimeGenerationRef.current;
    setReportState("exporting");
    setNotice(null);
    try {
      const result = await withNativeDialogFocus(() => api.exportExecutionReport(executionHandle));
      if (
        runtimeGenerationRef.current !== runtimeGeneration
        || executionReportIdentity(workflowRef.current.execution) !== exportIdentity
      ) {
        return;
      }
      setReportState(result.outcome === "saved" ? "saved" : "idle");
    } catch (error) {
      if (
        runtimeGenerationRef.current !== runtimeGeneration
        || executionReportIdentity(workflowRef.current.execution) !== exportIdentity
      ) {
        return;
      }
      setReportState("failed");
      setNotice(errorMessage(error));
    }
  }, [runtimeGenerationRef, setNotice, withNativeDialogFocus, workflowRef]);

  const launchConfiguredApp = useCallback(async () => {
    const current = workflowRef.current;
    if (
      current.execution.kind !== "terminal"
      || current.execution.snapshot.simulated
      || !current.execution.snapshot.launchAction
    ) return;
    const { generation, snapshot } = current.execution;
    const launchAction = snapshot.launchAction;
    if (!launchAction) return;
    const runtimeGeneration = runtimeGenerationRef.current;
    setLaunchState("launching");
    setNotice(null);
    try {
      const result = await api.launchConfiguredApp(launchAction.handle);
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      setLaunchState("launched");
      setNotice(result.message);
    } catch (error) {
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      setLaunchState("failed");
      setNotice(errorMessage(error));
      try {
        const refreshed = await api.getRealExecution(snapshot.executionHandle);
        if (runtimeGenerationRef.current !== runtimeGeneration) return;
        dispatch({ type: "execution-snapshot", generation, snapshot: refreshed });
      } catch {
        // The original sanitized launch error remains authoritative.
      }
    }
  }, [dispatch, runtimeGenerationRef, setNotice, workflowRef]);

  return {
    cancelExecution,
    exportExecutionReport,
    launchConfiguredApp,
    launchState,
    reportState,
    resetExecutionPresentation,
    startRealExecution,
    startSimulation,
  };
}
