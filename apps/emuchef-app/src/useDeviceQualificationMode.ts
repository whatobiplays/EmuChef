import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "./api";
import { errorMessage } from "./app-helpers";
import type {
  QualificationCheckpointOutcome,
  QualificationConnectionType,
  QualificationModeStatus,
  QualificationCandidateSummary,
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
  /** Persisted run candidates remain available after the active session closes. */
  runCandidates: QualificationCandidateSummary[];
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
   * Monotonic revision of successful authoritative inventory commits observed
   * by the host component. Rust invalidates or closes an active attempt during
   * inventory reconciliation without changing any workflow-visible value, so
   * the adapter re-reads the sanitized status whenever this revision advances.
   */
  inventoryRevision?: number;
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
 * Normalize a backend lifecycle revision. Only a finite positive revision
 * carries an ordering claim; a missing or zero revision means the projection
 * contains no lifecycle state and must never update the presented session.
 */
function orderingRevision(value: number | undefined): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : 0;
}

/** Refresh only when the execution phase or stable execution identity changes. */
function executionRefreshSignal(execution: WorkflowState["execution"] | undefined): string {
  if (!execution) return "none";
  switch (execution.kind) {
    case "idle":
      return "idle";
    case "starting":
      return `starting:${execution.generation}:${execution.mode}`;
    case "active":
    case "terminal":
      return `${execution.kind}:${execution.generation}:${execution.mode}:${execution.snapshot.executionHandle}:${execution.snapshot.reviewHandle}`;
    case "unavailable":
      return `unavailable:${execution.generation}:${execution.executionHandle}`;
  }
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
  inventoryRevision = 0,
  workflow,
  workflowRef,
}: UseDeviceQualificationModeOptions): DeviceQualificationModeController {
  const deviceHandle = workflow?.deviceHandle ?? null;
  const deviceFacts = workflow?.facts ?? null;
  const reviewHandle = workflow?.review?.reviewHandle ?? null;
  const executionSignal = executionRefreshSignal(workflow?.execution);
  const [status, setStatus] = useState<QualificationModeStatus | null>(null);
  const [session, setSession] = useState<QualificationSessionSnapshot | null>(null);
  const [targetCandidate, setTargetCandidate] = useState<QualificationTargetCandidatePreview | null>(null);
  const [runCandidates, setRunCandidates] = useState<QualificationCandidateSummary[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busyCountRef = useRef(0);
  const refreshGenerationRef = useRef(0);
  // Backend transition revision of the newest qualification lifecycle
  // projection this adapter has applied. Rust serializes every qualification
  // transition through the qualification transition gate and stamps each
  // projection with the revision of the acquisition that produced it, so
  // revisions are the only lifecycle ordering authority. A status projection
  // taken before a command always carries a lower revision than the committed
  // command snapshot, so it can never roll that snapshot back however late
  // its response arrives, while a status genuinely ordered after the command
  // supersedes it.
  const appliedLifecycleRevisionRef = useRef(0);
  // Backend-authored device-selection lock of the newest applied projection.
  const [deviceSelectionLocked, setDeviceSelectionLocked] = useState(false);
  // A successful begin result is authoritative for the active attempt it
  // committed, but the device-selection lock it owns is projected by the next
  // status. Until a revision-bearing status supersedes the begin snapshot, keep
  // presenting the lock that attempt owns.
  const [beginAssociationPending, setBeginAssociationPending] = useState(false);
  const recordingCandidateHandlesRef = useRef(new Set<string>());

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

  // Lifecycle state is ordered exclusively by the backend transition revision:
  // every successful status response offers its projection here, even when a
  // newer request superseded this one's frontend refresh generation, because a
  // projection taken under a newer Rust transition must win however late its
  // response arrives. Frontend request generations never order lifecycle state.
  const mergeLifecycleProjection = useCallback((nextStatus: QualificationModeStatus) => {
    const revision = orderingRevision(nextStatus.lifecycleRevision);
    // A response without a lifecycle revision makes no lifecycle claim, and a
    // projection older than the newest applied revision describes state that a
    // newer transition already superseded. Neither may update the presented
    // session, candidates, or device-selection lock.
    if (revision === 0 || revision < appliedLifecycleRevisionRef.current) return;
    appliedLifecycleRevisionRef.current = revision;
    setBeginAssociationPending(false);
    setDeviceSelectionLocked(nextStatus.deviceSelectionLocked);
    setSession(nextStatus.resumableSession ?? null);
    setRunCandidates(nextStatus.resumableCandidates.filter((candidate) => candidate.kind === "qualification_run"));
    setTargetCandidate((current) => {
      const retainedCurrent = current === null
        ? null
        : nextStatus.resumableCandidates
          .find((candidate) => candidate.candidateHandle === current.candidateHandle)
          ?? null;
      return (retainedCurrent && candidatePreviewFromSummary(retainedCurrent))
        ?? nextStatus.resumableCandidates
          .map(candidatePreviewFromSummary)
          .find((candidate): candidate is QualificationTargetCandidatePreview => candidate !== null)
        ?? null;
    });
  }, []);

  // Non-lifecycle presentation (build identity, catalog, recordability,
  // messages) reflects the newest status response; which response that is stays
  // a property of the frontend refresh generation.
  const applyStatusPresentation = useCallback((nextStatus: QualificationModeStatus) => {
    setStatus(nextStatus);
  }, []);

  // Apply the session snapshot a successful begin, checkpoint, or abandon
  // command returned. The snapshot carries the revision of the transition that
  // committed the command, so it is applied unless a genuinely newer
  // qualification projection already superseded it. Projecting it before the
  // follow-up status refresh means a failed refresh cannot leave a recorded
  // checkpoint looking unrecorded or an abandoned attempt looking active.
  const applyAuthoritativeSessionSnapshot = useCallback((
    nextSession: QualificationSessionSnapshot,
    options?: { pendingAssociation?: boolean },
  ) => {
    const revision = orderingRevision(nextSession.lifecycleRevision);
    if (revision !== 0 && revision < appliedLifecycleRevisionRef.current) return;
    if (revision !== 0) appliedLifecycleRevisionRef.current = revision;
    setSession(nextSession);
    if (nextSession.phase === "closed") {
      setBeginAssociationPending(false);
      return;
    }
    if (options?.pendingAssociation) setBeginAssociationPending(true);
  }, []);

  // Re-read the sanitized status. Operator and lifecycle refreshes present the
  // adapter as busy and surface failures; background synchronization after an
  // authoritative inventory commit neither flashes the operator controls into
  // a busy state nor replaces an operator-facing error.
  const loadStatus = useCallback(async (presentBusy: boolean) => {
    if (!enabled) return;
    const generation = ++refreshGenerationRef.current;
    if (presentBusy) {
      startBusy();
      setError(null);
    }
    try {
      const nextStatus = await api.deviceQualificationModeStatus();
      // Lifecycle merging is ordered only by the backend transition revision,
      // so every successful response offers its projection even when a newer
      // request superseded this one's frontend refresh generation.
      mergeLifecycleProjection(nextStatus);
      // The remaining status presentation still follows the newest request.
      if (generation === refreshGenerationRef.current) applyStatusPresentation(nextStatus);
    } catch (refreshError) {
      if (presentBusy && generation === refreshGenerationRef.current) {
        setError(errorMessage(refreshError));
      }
    } finally {
      if (presentBusy) finishBusy();
    }
  }, [applyStatusPresentation, enabled, finishBusy, mergeLifecycleProjection, startBusy]);

  const refresh = useCallback(async () => {
    await loadStatus(true);
  }, [loadStatus]);

  const presentationSignalsRef = useRef<{
    deviceFacts: unknown;
    deviceHandle: string | null;
    reviewHandle: string | null;
    executionSignal: string;
  } | null>(null);

  useEffect(() => {
    if (!enabled) {
      presentationSignalsRef.current = null;
      return;
    }
    const signals = { deviceFacts, deviceHandle, reviewHandle, executionSignal };
    const previous = presentationSignalsRef.current;
    presentationSignalsRef.current = signals;
    // Product lifecycle is owned by Rust. Only stable device, review, and
    // execution lifecycle signals trigger a presenting refresh; event batches,
    // progress snapshots, and editable workflow intent do not. An authoritative
    // inventory commit can close or invalidate an attempt without changing any
    // of those signals, so it synchronizes quietly instead.
    const inventoryOnly = previous !== null
      && previous.deviceFacts === signals.deviceFacts
      && previous.deviceHandle === signals.deviceHandle
      && previous.reviewHandle === signals.reviewHandle
      && previous.executionSignal === signals.executionSignal;
    void loadStatus(!inventoryOnly);
    return () => {
      refreshGenerationRef.current += 1;
    };
  }, [
    deviceFacts,
    deviceHandle,
    enabled,
    executionSignal,
    inventoryRevision,
    loadStatus,
    reviewHandle,
  ]);

  const beginSession = useCallback(async (request: {
    deviceHandle: string;
    devicePlan: string;
    targetId: string;
    workflowId: string;
  }) => {
    if (!enabled || !status?.enabled) return;
    const result = await runOperation(
      () => api.beginQualificationSession(request),
      (nextSession) => applyAuthoritativeSessionSnapshot(nextSession, { pendingAssociation: true }),
    );
    if (result !== null) await refresh();
  }, [applyAuthoritativeSessionSnapshot, enabled, refresh, runOperation, status?.enabled]);

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
    const result = await runOperation(
      () => api.recordQualificationCheckpoint(session.sessionHandle, checkpointId, outcome),
      (nextSession) => applyAuthoritativeSessionSnapshot(nextSession),
    );
    if (result !== null) await refresh();
  }, [
    applyAuthoritativeSessionSnapshot,
    enabled,
    refresh,
    runOperation,
    session,
    status?.enabled,
  ]);

  const abandonSession = useCallback(async () => {
    if (!enabled || !status?.enabled || !session) return;
    const result = await runOperation(
      () => api.abandonQualificationSession(session.sessionHandle),
      (closedSession) => applyAuthoritativeSessionSnapshot(closedSession),
    );
    if (result === null) return;
    await refresh();
  }, [
    applyAuthoritativeSessionSnapshot,
    enabled,
    refresh,
    runOperation,
    session,
    status?.enabled,
  ]);

  const recordRun = useCallback(async (candidateHandle: string) => {
    if (!enabled || !status?.enabled || recordingCandidateHandlesRef.current.has(candidateHandle)) return;
    recordingCandidateHandlesRef.current.add(candidateHandle);
    try {
      const result = await runOperation<QualificationRunRecordingResult>(
        () => api.recordQualificationRun(candidateHandle),
      );
      if (result === null) return;
      await refresh();
    } finally {
      recordingCandidateHandlesRef.current.delete(candidateHandle);
    }
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

  const presentedDeviceSelectionLocked = session?.phase === "closed"
    ? false
    : beginAssociationPending || deviceSelectionLocked;
  const intentLock = session && session.phase !== "closed"
    ? { devicePlan: session.devicePlan, selectedRecipes: [...session.requiredRecipes] }
    : null;

  return {
    status,
    session,
    targetCandidate,
    runCandidates,
    intentLock,
    deviceSelectionLocked: presentedDeviceSelectionLocked,
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
