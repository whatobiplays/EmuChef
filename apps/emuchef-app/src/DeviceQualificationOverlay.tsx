import { useEffect, useState } from "react";

import type {
  QualificationCheckpointOutcome,
  QualificationFactPreview,
  QualificationTargetCandidatePreview,
} from "./types";
import type { DeviceQualificationModeController } from "./useDeviceQualificationMode";

const checkpointOutcomeLabels: Record<QualificationCheckpointOutcome, string> = {
  pass: "Pass",
  fail: "Fail",
  unable_to_verify: "Unable to verify",
};

interface DeviceQualificationOverlayProps {
  controller: DeviceQualificationModeController;
  deviceHandle?: string | null;
  devicePlan?: string | null;
}

function FactValue({ fact }: { fact: QualificationFactPreview<unknown> }) {
  return (
    <>
      <span>{String(fact.value)}</span>
      <small>Source: {fact.source}</small>
    </>
  );
}

function TargetCandidate({
  candidate,
  controller,
}: {
  candidate: QualificationTargetCandidatePreview;
  controller: DeviceQualificationModeController;
}) {
  const facts: Array<[string, QualificationFactPreview<unknown>]> = [
    ["Profile", candidate.target.profileId],
    ["Manufacturer", candidate.target.manufacturer],
    ["Model", candidate.target.model],
    ["Android version", candidate.target.androidVersion],
    ["Android API", candidate.target.androidApi],
    ["ABI / SoC class", candidate.target.abiSocClass],
    ["Root state", candidate.target.rootState],
    ["Connection type", candidate.target.connectionType],
    ["Firmware build", candidate.target.firmwareBuild],
  ];

  return (
    <section className="qualification-candidate" aria-labelledby="qualification-candidate-heading">
      <div className="qualification-section-heading">
        <div>
          <p className="eyebrow">Stored candidate</p>
          <h3 id="qualification-candidate-heading">Review captured target facts</h3>
        </div>
        <span className={`status ${candidate.promotable ? "qualification-supported" : "qualification-unsupported"}`}>
          {candidate.promotable ? "Ready to register" : "Not promotable"}
        </span>
      </div>
      <p>Captured {candidate.capturedAt}. Values and provenance come from the trusted capture and cannot be edited here.</p>
      <dl className="qualification-facts">
        {facts.map(([label, fact]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd><FactValue fact={fact} /></dd>
          </div>
        ))}
      </dl>
      {candidate.target.capabilities.length > 0 && (
        <p>Capabilities: {candidate.target.capabilities.join(", ")}</p>
      )}
      {candidate.nonPromotableReason && <p className="error">{candidate.nonPromotableReason}</p>}
      <div className="button-row">
        <button
          type="button"
          disabled={controller.busy || !candidate.promotable}
          onClick={() => void controller.registerTarget(candidate.candidateHandle)}
        >
          Register device target
        </button>
        <button
          className="secondary"
          type="button"
          disabled={controller.busy}
          onClick={() => void controller.discardCandidate(candidate.candidateHandle)}
        >
          Discard candidate
        </button>
      </div>
    </section>
  );
}

function terminalClassification(controller: DeviceQualificationModeController): string | null {
  const current = controller.session;
  if (!current) return null;
  if (current.runValidity === "invalid") return "Invalid qualification run — not product evidence";
  if (current.qualificationOutcome === "failed") return "Product qualification failure";
  if (current.qualificationOutcome === "passed") return "Product qualification passed";
  return null;
}

/**
 * Renders the persistent development controller beside the ordinary product
 * workflow. It never discovers devices, configures recipes, reviews plans, or
 * starts execution; those responsibilities remain in the existing App UI.
 */
export function DeviceQualificationOverlay({
  controller,
  deviceHandle = null,
  devicePlan = null,
}: DeviceQualificationOverlayProps) {
  const status = controller.status;
  const [targetId, setTargetId] = useState("");
  const [workflowId, setWorkflowId] = useState("");
  const [connectionType, setConnectionType] = useState<"usb2" | "usb3">("usb3");

  useEffect(() => {
    setTargetId((current) => status?.targets.some((target) => target.id === current)
      ? current
      : status?.targets[0]?.id ?? "");
    setWorkflowId((current) => status?.workflows.some((workflow) => workflow.id === current)
      ? current
      : status?.workflows[0]?.id ?? "");
  }, [status?.targets, status?.workflows]);

  if (!status?.enabled) return null;

  const classification = terminalClassification(controller);
  const selectedTarget = status.targets.find((target) => target.id === targetId) ?? null;
  const selectedWorkflow = status.workflows.find((workflow) => workflow.id === workflowId) ?? null;
  const canBeginSession = Boolean(
    deviceHandle
      && devicePlan
      && selectedTarget
      && selectedWorkflow
      && !controller.busy,
  );

  return (
    <aside className="qualification-overlay" aria-labelledby="qualification-overlay-heading" data-testid="device-qualification-overlay">
      <div className="qualification-overlay-heading">
        <div>
          <p className="eyebrow">Development controller</p>
          <h2 id="qualification-overlay-heading">Device qualification mode</h2>
        </div>
        <span className={`status ${status.recordable ? "qualification-supported" : "qualification-unsupported"}`}>
          {status.recordable ? "Recordable build" : "Inspection only"}
        </span>
      </div>
      <p>
        This controller observes the normal EmuChef workflow. Complete device setup, inputs, review,
        confirmation, execution, and report inspection in the product surface below.
      </p>
      {status.build && (
        <dl className="qualification-build">
          <div><dt>Build</dt><dd>{status.build.appVersion}</dd></div>
          <div><dt>Runtime contract</dt><dd>{status.runtimeContract ?? "Unavailable"}</dd></div>
        </dl>
      )}
      {status.message && <p className="warning" role="status">{status.message}</p>}
      {controller.error && <p className="error" role="alert">{controller.error}</p>}

      {!controller.session && (
        <section className="qualification-session-start" aria-labelledby="qualification-session-heading">
          <h3 id="qualification-session-heading">Bind a target and workflow</h3>
          <div className="qualification-selectors">
            <label>
              Registered target
              <select aria-label="Registered target" value={targetId} onChange={(event) => setTargetId(event.currentTarget.value)}>
                <option value="">Choose a registered target</option>
                {status.targets.map((target) => (
                  <option key={target.id} value={target.id}>{target.manufacturer} {target.model}</option>
                ))}
              </select>
            </label>
            <label>
              Canonical workflow
              <select aria-label="Canonical workflow" value={workflowId} onChange={(event) => setWorkflowId(event.currentTarget.value)}>
                <option value="">Choose a workflow</option>
                {status.workflows.map((workflow) => (
                  <option key={workflow.id} value={workflow.id}>{workflow.purpose}</option>
                ))}
              </select>
            </label>
          </div>
          <button
            type="button"
            disabled={!canBeginSession}
            onClick={() => {
              if (!selectedTarget || !selectedWorkflow || !deviceHandle || !devicePlan) return;
              void controller.beginSession({
                deviceHandle,
                devicePlan,
                targetId: selectedTarget.id,
                workflowId: selectedWorkflow.id,
              });
            }}
          >
            Begin qualification session
          </button>
          {(!deviceHandle || !devicePlan) && (
            <p className="disabled-reason">Connect a device and choose a setup in the normal workflow before binding a session.</p>
          )}
        </section>
      )}

      <section className="qualification-target-capture" aria-labelledby="qualification-capture-heading">
        <h3 id="qualification-capture-heading">Register a device target</h3>
        <p>Capture trusted device facts from the selected device, then review the stored candidate before registration.</p>
        <label>
          Connection type attestation
          <select aria-label="Connection type attestation" value={connectionType} onChange={(event) => setConnectionType(event.currentTarget.value as "usb2" | "usb3")}>
            <option value="usb2">USB 2</option>
            <option value="usb3">USB 3</option>
          </select>
        </label>
        <button
          type="button"
          disabled={!deviceHandle || !devicePlan || controller.busy}
          onClick={() => void controller.createTargetCandidate(connectionType)}
        >
          Capture target facts
        </button>
      </section>

      {controller.targetCandidate && (
        <TargetCandidate candidate={controller.targetCandidate} controller={controller} />
      )}

      {controller.session && (
        <section className="qualification-session" aria-labelledby="qualification-session-state-heading">
          <div className="qualification-section-heading">
            <div>
              <p className="eyebrow">Active session</p>
              <h3 id="qualification-session-state-heading">Normal workflow intent is locked</h3>
            </div>
            <span className="status qualification-supported">Bound</span>
          </div>
          <dl className="qualification-session-facts">
            <div><dt>Target</dt><dd>{controller.session.targetId}</dd></div>
            <div><dt>Workflow</dt><dd>{controller.session.workflowId}</dd></div>
            <div><dt>Device setup</dt><dd>{controller.session.devicePlan}</dd></div>
            <div><dt>Required recipes</dt><dd>{controller.session.requiredRecipes.join(", ") || "None"}</dd></div>
          </dl>
          {controller.session.humanCheckpoints.length > 0 && (
            <div className="qualification-checkpoints">
              <h4>Declared checkpoints</h4>
              {controller.session.humanCheckpoints.map((checkpoint) => {
                const recorded = controller.session?.recordedCheckpoints.find(
                  (candidate) => candidate.checkpointId === checkpoint.id,
                );
                return (
                  <fieldset key={checkpoint.id}>
                    <legend>{checkpoint.fact}</legend>
                    <p>{checkpoint.instruction}</p>
                    <div className="qualification-outcomes">
                      {checkpoint.allowedOutcomes.map((outcome) => (
                        <label key={outcome}>
                          <input
                            type="radio"
                            name={`checkpoint-${checkpoint.id}`}
                            checked={recorded?.outcome === outcome}
                            disabled={recorded !== undefined || controller.busy}
                            onChange={() => void controller.recordCheckpoint(checkpoint.id, outcome)}
                          />
                          {checkpointOutcomeLabels[outcome]}
                        </label>
                      ))}
                    </div>
                    {recorded && <small>Recorded at {recorded.observedAt}</small>}
                  </fieldset>
                );
              })}
            </div>
          )}
          {classification && (
            <p className={controller.session.runValidity === "invalid" ? "error" : "warning"} role="status">
              {classification}
            </p>
          )}
          {controller.session.candidate && (
            <button
              type="button"
              disabled={controller.busy}
              onClick={() => void controller.recordRun(controller.session!.candidate!.candidateHandle)}
            >
              Record qualification run
            </button>
          )}
        </section>
      )}
    </aside>
  );
}
