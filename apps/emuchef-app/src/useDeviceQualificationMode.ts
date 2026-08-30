import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "./api";
import { errorMessage } from "./app-helpers";
import type {
  DeviceFacts,
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
  /** Whether this process has validated and bound the session to its live device handle. */
  deviceSelectionLocked: boolean;
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

function deviceFactsKey(facts: DeviceFacts | null): string {
  if (!facts) return "";
  return [
    facts.deviceHandle,
    facts.manufacturer ?? "",
    facts.brand ?? "",
    facts.model ?? "",
    facts.androidVersion ?? "",
    facts.androidApiLevel ?? "",
    facts.firmwareBuild ?? "",
  ].join("\u0000");
}

function workflowDeviceObservationKey(workflow: WorkflowState): string {
  return [
    workflow.deviceHandle ?? "",
    workflow.devicePlan ?? "",
    deviceFactsKey(workflow.facts),
  ].join("\u0001");
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
  const [transitionRetryRevision, setTransitionRetryRevision] = useState(0);
  const [reviewBindingRevision, setReviewBindingRevision] = useState(0);
  const busyCountRef = useRef(0);
  const sessionRef = useRef<QualificationSessionSnapshot | null>(null);
  const sessionDeviceHandleRef = useRef<string | null>(null);
  const boundDeviceFactsKeyRef = useRef("");
  const lastSessionObservationKeyRef = useRef<string | null>(null);
  const boundReviewKeysRef = useRef(new Set<string>());
  const reviewBindingPromisesRef = useRef(
    new Map<string, Promise<QualificationSessionSnapshot | null>>(),
  );
  const executionBindingPromisesRef = useRef(
    new Map<string, Promise<QualificationSessionSnapshot | null>>(),
  );
  const executionFinalizationPromisesRef = useRef(
    new Map<string, Promise<QualificationSessionSnapshot | null>>(),
  );
  const finalizedExecutionKeysRef = useRef(new Set<string>());
  const sessionRefreshPromisesRef = useRef(
    new Map<string, Promise<QualificationSessionSnapshot | null>>(),
  );
  const transitionRetryAttemptsRef = useRef(new Map<string, number>());
  const recordingCandidateHandlesRef = useRef(new Set<string>());
  const recordingCandidateInFlightRef = useRef(new Set<string>());

  sessionRef.current = session;

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

  const scheduleTransitionRetry = useCallback((key: string) => {
    const attempts = transitionRetryAttemptsRef.current.get(key) ?? 0;
    if (attempts >= 1) return;
    transitionRetryAttemptsRef.current.set(key, attempts + 1);
    queueMicrotask(() => setTransitionRetryRevision((revision) => revision + 1));
  }, []);

  const clearTransitionRetry = useCallback((key: string) => {
    transitionRetryAttemptsRef.current.delete(key);
  }, []);

  const applySessionIfCurrent = useCallback((
    sessionHandle: string,
    nextSession: QualificationSessionSnapshot,
  ) => {
    if (sessionRef.current?.sessionHandle !== sessionHandle) return;
    setSession(nextSession);
  }, []);

  const clearActiveSession = useCallback(() => {
    sessionRef.current = null;
    sessionDeviceHandleRef.current = null;
    boundDeviceFactsKeyRef.current = "";
    lastSessionObservationKeyRef.current = null;
    setSession(null);
  }, []);

  const refresh = useCallback(async () => {
    if (!enabled) return;
    transitionRetryAttemptsRef.current.clear();
    setTransitionRetryRevision((revision) => revision + 1);
    startBusy();
    setError(null);
    try {
      const nextStatus = await api.deviceQualificationModeStatus();
      setStatus(nextStatus);
      if (!nextStatus.enabled) {
        clearActiveSession();
        setTargetCandidate(null);
        return;
      }
      const recoveredSession = nextStatus.resumableSession ?? null;
      setSession((current) => current ?? recoveredSession);
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
  }, [clearActiveSession, enabled, finishBusy, startBusy]);

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
      (nextSession) => {
        sessionDeviceHandleRef.current = request.deviceHandle;
        boundDeviceFactsKeyRef.current = deviceFactsKey(workflowRef.current.facts);
        lastSessionObservationKeyRef.current = null;
        transitionRetryAttemptsRef.current.clear();
        setSession(nextSession);
      },
    );
  }, [enabled, runOperation, status?.enabled, workflowRef]);

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
      (nextSession) => applySessionIfCurrent(sessionHandle, nextSession),
    );
  }, [applySessionIfCurrent, enabled, runOperation, status?.enabled]);

  const recordRun = useCallback(async (candidateHandle: string) => {
    if (
      !enabled
      || !status?.enabled
      || recordingCandidateHandlesRef.current.has(candidateHandle)
      || recordingCandidateInFlightRef.current.has(candidateHandle)
    ) return;
    recordingCandidateInFlightRef.current.add(candidateHandle);
    try {
      const result = await runOperation<QualificationRunRecordingResult>(
        () => api.recordQualificationRun(candidateHandle),
      );
      if (result === null) return;
      recordingCandidateHandlesRef.current.add(candidateHandle);
      clearActiveSession();
      await refresh();
    } finally {
      recordingCandidateInFlightRef.current.delete(candidateHandle);
    }
  }, [clearActiveSession, enabled, refresh, runOperation, status?.enabled]);

  const discardCandidate = useCallback(async (candidateHandle: string) => {
    if (!enabled || !status?.enabled) return;
    const result = await runOperation(
      () => api.discardQualificationCandidate(candidateHandle),
    );
    if (result === null) return;
    setTargetCandidate((current) => current?.candidateHandle === candidateHandle ? null : current);
    if (sessionRef.current?.candidate?.candidateHandle === candidateHandle) clearActiveSession();
    await refresh();
  }, [clearActiveSession, enabled, refresh, runOperation, status?.enabled]);

  const reviewHandle = workflow.review?.reviewHandle ?? null;
  const productionExecution = workflow.execution.kind === "active" || workflow.execution.kind === "terminal"
    ? workflow.execution
    : null;
  const executionHandle = productionExecution?.snapshot.executionHandle ?? null;
  const executionIdentity = productionExecution && executionHandle
    ? `${productionExecution.generation}:${executionHandle}`
    : null;
  const observedDeviceHandle = workflow.deviceHandle;
  const observedFactsKey = deviceFactsKey(workflow.facts);
  const deviceObservationKey = workflowDeviceObservationKey(workflow);

  useEffect(() => {
    if (!enabled || !session || !sessionDeviceHandleRef.current || !reviewHandle) return;
    const sessionHandle = session.sessionHandle;
    const key = `${sessionHandle}:${reviewHandle}`;
    if (boundReviewKeysRef.current.has(key)) return;
    if (reviewBindingPromisesRef.current.has(key)) return;
    const binding = runOperation(
      () => api.bindQualificationReview(sessionHandle, reviewHandle),
      (nextSession) => applySessionIfCurrent(sessionHandle, nextSession),
    );
    reviewBindingPromisesRef.current.set(key, binding);
    void binding.then((result) => {
      reviewBindingPromisesRef.current.delete(key);
      if (result === null) {
        boundReviewKeysRef.current.delete(key);
        scheduleTransitionRetry(`review-bind:${key}`);
      } else {
        boundReviewKeysRef.current.add(key);
        setReviewBindingRevision((revision) => revision + 1);
        clearTransitionRetry(`review-bind:${key}`);
      }
    });
  }, [
    applySessionIfCurrent,
    clearTransitionRetry,
    enabled,
    reviewHandle,
    runOperation,
    scheduleTransitionRetry,
    session,
    transitionRetryRevision,
  ]);

  useEffect(() => {
    if (
      !enabled
      || !session
      || !sessionDeviceHandleRef.current
      || !productionExecution
      || productionExecution.mode !== "real"
      || !executionIdentity
    ) {
      return;
    }
    const sessionHandle = session.sessionHandle;
    const key = `${sessionHandle}:${executionIdentity}`;
    const reviewBindingKey = reviewHandle ? `${sessionHandle}:${reviewHandle}` : null;
    if (!reviewBindingKey || !boundReviewKeysRef.current.has(reviewBindingKey)) return;
    let binding = executionBindingPromisesRef.current.get(key);
    if (!binding) {
      binding = runOperation(
        () => api.bindQualificationExecution(sessionHandle, executionHandle!),
        (nextSession) => applySessionIfCurrent(sessionHandle, nextSession),
      );
      executionBindingPromisesRef.current.set(key, binding);
      void binding.then((result) => {
        if (result === null) {
          executionBindingPromisesRef.current.delete(key);
          scheduleTransitionRetry(`execution-bind:${key}`);
        } else {
          clearTransitionRetry(`execution-bind:${key}`);
        }
      });
    }
    if (
      productionExecution.kind !== "terminal"
      || finalizedExecutionKeysRef.current.has(key)
      || executionFinalizationPromisesRef.current.has(key)
    ) return;
    const finalization = binding.then((result) => {
      if (result === null || sessionRef.current?.sessionHandle !== sessionHandle) return null;
      return runOperation(
        () => api.finalizeQualificationCandidate(sessionHandle),
        (nextSession) => applySessionIfCurrent(sessionHandle, nextSession),
      );
    });
    executionFinalizationPromisesRef.current.set(key, finalization);
    void finalization.then((result) => {
      executionFinalizationPromisesRef.current.delete(key);
      if (result === null) {
        scheduleTransitionRetry(`execution-finalize:${key}`);
      } else {
        finalizedExecutionKeysRef.current.add(key);
        clearTransitionRetry(`execution-finalize:${key}`);
      }
    });
  }, [
    applySessionIfCurrent,
    clearTransitionRetry,
    enabled,
    executionHandle,
    executionIdentity,
    productionExecution,
    runOperation,
    scheduleTransitionRetry,
    reviewBindingRevision,
    reviewHandle,
    session,
    transitionRetryRevision,
  ]);

  useEffect(() => {
    if (!enabled || !session) return;
    const sessionHandle = session.sessionHandle;
    const associatedDeviceHandle = sessionDeviceHandleRef.current;
    const establishingAssociation = associatedDeviceHandle === null;
    if (establishingAssociation && (!observedDeviceHandle || observedFactsKey === "")) return;
    const refreshDeviceHandle = associatedDeviceHandle ?? observedDeviceHandle!;
    const deviceUnavailable = associatedDeviceHandle !== null
      && observedDeviceHandle !== associatedDeviceHandle;
    const factsBecameAvailable = boundDeviceFactsKeyRef.current === "" && observedFactsKey !== "";
    const identityChanged = boundDeviceFactsKeyRef.current !== ""
      && observedFactsKey !== boundDeviceFactsKeyRef.current;
    const planChanged = workflow.devicePlan !== session.devicePlan;
    const driftKey = `${sessionHandle}:${deviceObservationKey}`;
    if (!establishingAssociation && !deviceUnavailable && !factsBecameAvailable && !identityChanged && !planChanged) {
      lastSessionObservationKeyRef.current = driftKey;
      return;
    }
    if (lastSessionObservationKeyRef.current === driftKey) return;
    lastSessionObservationKeyRef.current = driftKey;
    let refreshSession = sessionRefreshPromisesRef.current.get(sessionHandle);
    if (!refreshSession) {
      refreshSession = runOperation(
        () => api.refreshQualificationSession(sessionHandle, refreshDeviceHandle),
        (nextSession) => {
          const refreshStillTargetsCurrentDevice =
            workflowRef.current.deviceHandle === refreshDeviceHandle;
          if (
            sessionRef.current?.sessionHandle === sessionHandle
            && establishingAssociation
            && nextSession.runValidity === "valid"
            && refreshStillTargetsCurrentDevice
          ) {
            sessionDeviceHandleRef.current = refreshDeviceHandle;
          }
          if (
            sessionRef.current?.sessionHandle === sessionHandle
            && factsBecameAvailable
            && nextSession.runValidity === "valid"
            && refreshStillTargetsCurrentDevice
          ) {
            boundDeviceFactsKeyRef.current = observedFactsKey;
          }
          if (
            sessionRef.current?.sessionHandle === sessionHandle
            && establishingAssociation
            && !refreshStillTargetsCurrentDevice
          ) {
            // The selected device changed while the refresh was in flight.
            // Let the current selection run through the normal validation path
            // instead of retaining the old observation key.
            lastSessionObservationKeyRef.current = null;
          }
          applySessionIfCurrent(sessionHandle, nextSession);
        },
      );
      sessionRefreshPromisesRef.current.set(sessionHandle, refreshSession);
      void refreshSession.then((result) => {
        sessionRefreshPromisesRef.current.delete(sessionHandle);
        if (result === null) {
          lastSessionObservationKeyRef.current = null;
          scheduleTransitionRetry(`session-refresh:${driftKey}`);
        } else {
          clearTransitionRetry(`session-refresh:${driftKey}`);
        }
      });
    }
  }, [
    applySessionIfCurrent,
    clearTransitionRetry,
    deviceObservationKey,
    enabled,
    observedDeviceHandle,
    observedFactsKey,
    runOperation,
    scheduleTransitionRetry,
    session,
    transitionRetryRevision,
    workflowRef,
    workflow.devicePlan,
  ]);

  const intentLock = session
    ? { devicePlan: session.devicePlan, selectedRecipes: [...session.requiredRecipes] }
    : null;
  const deviceSelectionLocked = session !== null && sessionDeviceHandleRef.current !== null;

  return {
    status,
    session,
    targetCandidate,
    intentLock,
    deviceSelectionLocked,
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
