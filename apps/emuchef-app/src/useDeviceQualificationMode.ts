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
  /** Whether an active attempt owns the product device selection. */
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
  abandonSession: () => Promise<void>;
  recordRun: (candidateHandle: string) => Promise<void>;
  discardCandidate: (candidateHandle: string) => Promise<void>;
}

/** Inputs needed to resolve operator intent without owning product state. */
export interface UseDeviceQualificationModeOptions {
  enabled?: boolean;
  /**
   * Live product workflow snapshot supplied by the host component. It is only
   * read when an operator action needs the currently selected device or setup;
   * qualification lifecycle state always comes from Rust, never from workflow
   * comparison.
   */
  workflow?: WorkflowState;
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
 * Presentation and operator-intent adapter for device qualification mode.
 *
 * Trusted lifecycle ordering lives in Rust/Tauri: the product commits device,
 * root, review, admission, and terminal observations and synchronously feeds
 * the active qualification session. This hook therefore only loads sanitized
 * status, forwards explicit operator actions, and projects the resulting
 * sanitized session state. It never compares workflow snapshots, binds
 * reviews or executions, finalizes candidates, or retries trusted
 * transitions.
 */
export function useDeviceQualificationMode({
  enabled = true,
  workflowRef,
}: UseDeviceQualificationModeOptions): DeviceQualificationModeController {
  const [status, setStatus] = useState<QualificationModeStatus | null>(null);
  const [session, setSession] = useState<QualificationSessionSnapshot | null>(null);
  const [targetCandidate, setTargetCandidate] = useState<QualificationTargetCandidatePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busyCountRef = useRef(0);

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

  const applyStatus = useCallback((nextStatus: QualificationModeStatus) => {
    setStatus(nextStatus);
    if (!nextStatus.enabled) {
      setSession(null);
      setTargetCandidate(null);
      return;
    }
    setSession(nextStatus.resumableSession ?? null);
    setTargetCandidate(
      nextStatus.resumableCandidates
        .map(candidatePreviewFromSummary)
        .find((candidate): candidate is QualificationTargetCandidatePreview => candidate !== null)
        ?? null,
    );
  }, []);

  const refresh = useCallback(async () => {
    if (!enabled) return;
    startBusy();
    setError(null);
    try {
      applyStatus(await api.deviceQualificationModeStatus());
    } catch (refreshError) {
      setError(errorMessage(refreshError));
    } finally {
      finishBusy();
    }
  }, [applyStatus, enabled, finishBusy, startBusy]);

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
    if (!enabled || !status?.enabled || !session) return;
    await runOperation(
      () => api.recordQualificationCheckpoint(session.sessionHandle, checkpointId, outcome),
      (nextSession) => setSession(nextSession),
    );
  }, [enabled, runOperation, session, status?.enabled]);

  const abandonSession = useCallback(async () => {
    if (!enabled || !status?.enabled || !session) return;
    const result = await runOperation(
      () => api.abandonQualificationSession(session.sessionHandle),
    );
    if (result === null) return;
    await refresh();
  }, [enabled, refresh, runOperation, session, status?.enabled]);

  const recordRun = useCallback(async (candidateHandle: string) => {
    if (!enabled || !status?.enabled) return;
    const result = await runOperation<QualificationRunRecordingResult>(
      () => api.recordQualificationRun(candidateHandle),
    );
    if (result === null) return;
    await refresh();
  }, [enabled, refresh, runOperation, status?.enabled]);

  const discardCandidate = useCallback(async (candidateHandle: string) => {
    if (!enabled || !status?.enabled) return;
    const result = await runOperation(
      () => api.discardQualificationCandidate(candidateHandle),
    );
    if (result === null) return;
    setTargetCandidate((current) => current?.candidateHandle === candidateHandle ? null : current);
    if (session?.candidate?.candidateHandle === candidateHandle) setSession(null);
    await refresh();
  }, [enabled, refresh, runOperation, session, status?.enabled]);

  const intentLock = session
    ? { devicePlan: session.devicePlan, selectedRecipes: [...session.requiredRecipes] }
    : null;
  /**
   * An active attempt owns the product device selection: any other device
   * would invalidate the attempt, so selection stays locked until the attempt
   * closes. This is a projection of sanitized session state, not an inference
   * about restored process-local handles.
   */
  const deviceSelectionLocked = session !== null;

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
    abandonSession,
    recordRun,
    discardCandidate,
  };
}
