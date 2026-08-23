import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "./api";
import { errorMessage } from "./app-helpers";
import type {
  QualificationCheckpointOutcome,
  QualificationConnectionType,
  QualificationModeStatus,
  QualificationRunRecordingResult,
  QualificationSessionSnapshot,
  QualificationTargetCandidatePreview,
} from "./types";
import type { WorkflowState } from "./workflow";

/** The production intent that a qualification session owns for its lifetime. */
export interface QualificationIntentLock {
  devicePlan: string;
  selectedRecipes: string[];
}

/** The state and commands exposed to the qualification presentation layer. */
export interface DeviceQualificationModeController {
  status: QualificationModeStatus | null;
  session: QualificationSessionSnapshot | null;
  targetCandidate: QualificationTargetCandidatePreview | null;
  intentLock: QualificationIntentLock | null;
  busy: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  beginSession: (request: {
    deviceHandle: string;
    devicePlan: string;
    targetId: string;
    workflowId: string;
  }) => Promise<void>;
  createTargetCandidate: (connectionType: QualificationConnectionType) => Promise<void>;
  registerTarget: (candidateHandle: string) => Promise<void>;
  recordCheckpoint: (checkpointId: string, outcome: QualificationCheckpointOutcome) => Promise<void>;
  recordRun: (candidateHandle: string) => Promise<void>;
  discardCandidate: (candidateHandle: string) => Promise<void>;
}

/** Inputs needed to observe the ordinary workflow without owning it. */
export interface UseDeviceQualificationModeOptions {
  enabled?: boolean;
  workflow: WorkflowState;
  workflowRef: { current: WorkflowState };
}

function candidatePreviewFromSummary(
  candidate: QualificationModeStatus["resumableCandidates"][number],
): QualificationTargetCandidatePreview | null {
  if (candidate.kind !== "target_registration" || !candidate.target) return null;
  return {
    candidateHandle: candidate.candidateHandle,
    kind: "target_registration",
    capturedAt: candidate.capturedAt,
    target: candidate.target,
    promotable: candidate.promotable,
    nonPromotableReason: candidate.nonPromotableReason,
  };
}

/**
 * Observes production review/execution state and exposes qualification-only
 * operator actions. All repository, device, and evidence authority remains in
 * the Tauri API; this hook stores only sanitized DTOs and opaque handles.
 */
export function useDeviceQualificationMode({
  enabled = true,
  workflow,
  workflowRef,
}: UseDeviceQualificationModeOptions): DeviceQualificationModeController {
  const [status, setStatus] = useState<QualificationModeStatus | null>(null);
  const [session, setSession] = useState<QualificationSessionSnapshot | null>(null);
  const [targetCandidate, setTargetCandidate] = useState<QualificationTargetCandidatePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busyCountRef = useRef(0);
  const sessionRef = useRef<QualificationSessionSnapshot | null>(null);
  const targetCandidateRef = useRef<QualificationTargetCandidatePreview | null>(null);
  const boundReviewKeysRef = useRef(new Set<string>());
  const executionBindingPromisesRef = useRef(
    new Map<string, Promise<QualificationSessionSnapshot | null>>(),
  );
  const finalizedExecutionKeysRef = useRef(new Set<string>());

  sessionRef.current = session;
  targetCandidateRef.current = targetCandidate;

  const startBusy = useCallback(() => {
    busyCountRef.current += 1;
    setBusy(true);
  }, []);

  const finishBusy = useCallback(() => {
    busyCountRef.current = Math.max(0, busyCountRef.current - 1);
    if (busyCountRef.current === 0) setBusy(false);
  }, []);

  const runOperation = useCallback(async <Result,>(
    operation: () => Promise<Result>,
    onSuccess?: (result: Result) => void,
  ): Promise<Result | null> => {
    startBusy();
    setError(null);
    try {
      const result = await operation();
      onSuccess?.(result);
      return result;
    } catch (operationError) {
      setError(errorMessage(operationError));
      return null;
    } finally {
      finishBusy();
    }
  }, [finishBusy, startBusy]);

  const refresh = useCallback(async () => {
    if (!enabled) return;
    startBusy();
    setError(null);
    try {
      const nextStatus = await api.deviceQualificationModeStatus();
      setStatus(nextStatus);
      if (!nextStatus.enabled) {
        setSession(null);
        setTargetCandidate(null);
        return;
      }
      const recoveredCandidate = nextStatus.resumableCandidates
        .map(candidatePreviewFromSummary)
        .find((candidate): candidate is QualificationTargetCandidatePreview => candidate !== null)
        ?? null;
      setTargetCandidate((current) => {
        if (!current) return recoveredCandidate;
        const persisted = nextStatus.resumableCandidates.find(
          (candidate) => candidate.candidateHandle === current.candidateHandle,
        );
        return persisted ? candidatePreviewFromSummary(persisted) ?? current : current;
      });
    } catch (refreshError) {
      setError(errorMessage(refreshError));
    } finally {
      finishBusy();
    }
  }, [enabled, finishBusy, startBusy]);

  useEffect(() => {
    if (!enabled) return;
    void refresh();
  }, [enabled, refresh]);

  const beginSession = useCallback(async (request: {
    deviceHandle: string;
    devicePlan: string;
    targetId: string;
    workflowId: string;
  }) => {
    if (!enabled || !status?.enabled) return;
    await runOperation(
      () => api.beginQualificationSession(request),
      (nextSession) => setSession(nextSession),
    );
  }, [enabled, runOperation, status?.enabled]);

  const createTargetCandidate = useCallback(async (connectionType: QualificationConnectionType) => {
    if (!enabled || !status?.enabled) return;
    const current = workflowRef.current;
    if (!current.deviceHandle || !current.devicePlan) {
      setError("Select a device and setup in the normal workflow before capturing a target.");
      return;
    }
    await runOperation(
      () => api.createQualificationTargetCandidate({
        deviceHandle: current.deviceHandle!,
        devicePlan: current.devicePlan!,
        connectionType,
      }),
      (candidate) => setTargetCandidate(candidate),
    );
  }, [enabled, runOperation, status?.enabled, workflowRef]);

  const registerTarget = useCallback(async (candidateHandle: string) => {
    if (!enabled || !status?.enabled) return;
    const result = await runOperation(
      () => api.registerQualificationTarget(candidateHandle),
    );
    if (result === null) return;
    setTargetCandidate((current) => current?.candidateHandle === candidateHandle ? null : current);
    await refresh();
  }, [enabled, refresh, runOperation, status?.enabled]);

  const recordCheckpoint = useCallback(async (
    checkpointId: string,
    outcome: QualificationCheckpointOutcome,
  ) => {
    if (!enabled || !status?.enabled || !sessionRef.current) return;
    const sessionHandle = sessionRef.current.sessionHandle;
    await runOperation(
      () => api.recordQualificationCheckpoint(sessionHandle, checkpointId, outcome),
      (nextSession) => setSession(nextSession),
    );
  }, [enabled, runOperation, status?.enabled]);

  const recordRun = useCallback(async (candidateHandle: string) => {
    if (!enabled || !status?.enabled) return;
    const result = await runOperation<QualificationRunRecordingResult>(
      () => api.recordQualificationRun(candidateHandle),
    );
    if (result !== null) await refresh();
  }, [enabled, refresh, runOperation, status?.enabled]);

  const discardCandidate = useCallback(async (candidateHandle: string) => {
    if (!enabled || !status?.enabled) return;
    const result = await runOperation(
      () => api.discardQualificationCandidate(candidateHandle),
    );
    if (result === null) return;
    setTargetCandidate((current) => current?.candidateHandle === candidateHandle ? null : current);
    setSession((current) => current?.candidate?.candidateHandle === candidateHandle ? null : current);
    await refresh();
  }, [enabled, refresh, runOperation, status?.enabled]);

  const reviewHandle = workflow.review?.reviewHandle ?? null;
  const productionExecution = workflow.execution.kind === "active" || workflow.execution.kind === "terminal"
    ? workflow.execution
    : null;
  const executionHandle = productionExecution?.snapshot.executionHandle ?? null;
  const executionIdentity = productionExecution && executionHandle
    ? `${productionExecution.generation}:${executionHandle}`
    : null;

  useEffect(() => {
    if (!enabled || !session || !reviewHandle) return;
    const key = `${session.sessionHandle}:${reviewHandle}`;
    if (boundReviewKeysRef.current.has(key)) return;
    boundReviewKeysRef.current.add(key);
    void runOperation(
      () => api.bindQualificationReview(session.sessionHandle, reviewHandle),
      (nextSession) => setSession(nextSession),
    ).then((result) => {
      if (result === null) boundReviewKeysRef.current.delete(key);
    });
  }, [enabled, reviewHandle, runOperation, session]);

  useEffect(() => {
    if (!enabled || !session || !productionExecution || productionExecution.mode !== "real" || !executionIdentity) {
      return;
    }
    const key = `${session.sessionHandle}:${executionIdentity}`;
    let binding = executionBindingPromisesRef.current.get(key);
    if (!binding) {
      binding = runOperation(
        () => api.bindQualificationExecution(session.sessionHandle, executionHandle!),
        (nextSession) => setSession(nextSession),
      );
      executionBindingPromisesRef.current.set(key, binding);
      void binding.then((result) => {
        if (result === null) executionBindingPromisesRef.current.delete(key);
      });
    }
    if (productionExecution.kind !== "terminal" || finalizedExecutionKeysRef.current.has(key)) return;
    finalizedExecutionKeysRef.current.add(key);
    void binding.then((result) => {
      if (result === null) {
        finalizedExecutionKeysRef.current.delete(key);
        return null;
      }
      return runOperation(
        () => api.finalizeQualificationCandidate(session.sessionHandle),
        (nextSession) => setSession(nextSession),
      );
    }).then((result) => {
      if (result === null) finalizedExecutionKeysRef.current.delete(key);
    });
  }, [enabled, executionHandle, executionIdentity, productionExecution, runOperation, session]);

  const intentLock = session
    ? { devicePlan: session.devicePlan, selectedRecipes: [...session.requiredRecipes] }
    : null;

  return {
    status,
    session,
    targetCandidate,
    intentLock,
    busy,
    error,
    refresh,
    beginSession,
    createTargetCandidate,
    registerTarget,
    recordCheckpoint,
    recordRun,
    discardCandidate,
  };
}
