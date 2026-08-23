import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";

import { api } from "../src/api";

const invokeMock = vi.mocked(invoke);

describe("device qualification API", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it("keeps status and target capture behind opaque Tauri commands", async () => {
    await api.deviceQualificationModeStatus();
    expect(invokeMock).toHaveBeenLastCalledWith("get_device_qualification_mode_status");

    await api.createQualificationTargetCandidate({
      deviceHandle: "device_opaque",
      devicePlan: "ayaneo.pocket_s2",
      connectionType: "usb3",
    });
    expect(invokeMock).toHaveBeenLastCalledWith("create_qualification_target_candidate", {
      request: {
        deviceHandle: "device_opaque",
        devicePlan: "ayaneo.pocket_s2",
        connectionType: "usb3",
      },
    });
  });

  it("registers and discards only by opaque candidate handle", async () => {
    await api.registerQualificationTarget(
      "qualification-candidate-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    expect(invokeMock).toHaveBeenLastCalledWith("register_qualification_target", {
      candidateHandle: "qualification-candidate-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    });

    await api.discardQualificationCandidate(
      "qualification-candidate-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    expect(invokeMock).toHaveBeenLastCalledWith("discard_qualification_candidate", {
      candidateHandle: "qualification-candidate-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    });
  });

  it("orchestrates qualification sessions through typed opaque handles", async () => {
    await api.beginQualificationSession({
      deviceHandle: "device_opaque",
      devicePlan: "plan_opaque",
      targetId: "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      workflowId: "workflow_opaque",
    });
    expect(invokeMock).toHaveBeenLastCalledWith("begin_qualification_session", {
      request: {
        deviceHandle: "device_opaque",
        devicePlan: "plan_opaque",
        targetId: "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        workflowId: "workflow_opaque",
      },
    });

    await api.refreshQualificationSession("qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "device_opaque");
    expect(invokeMock).toHaveBeenLastCalledWith("refresh_qualification_session", {
      sessionHandle: "qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      deviceHandle: "device_opaque",
    });

    await api.bindQualificationReview("qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "review_opaque");
    expect(invokeMock).toHaveBeenLastCalledWith("bind_qualification_review", {
      sessionHandle: "qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      reviewHandle: "review_opaque",
    });

    await api.bindQualificationExecution("qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "execution_opaque");
    expect(invokeMock).toHaveBeenLastCalledWith("bind_qualification_execution", {
      sessionHandle: "qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      executionHandle: "execution_opaque",
    });

    await api.recordQualificationCheckpoint(
      "qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "clean_or_deliberately_reset_device",
      "pass",
    );
    expect(invokeMock).toHaveBeenLastCalledWith("record_qualification_checkpoint", {
      sessionHandle: "qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      checkpointId: "clean_or_deliberately_reset_device",
      outcome: "pass",
    });

    await api.finalizeQualificationCandidate("qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    expect(invokeMock).toHaveBeenLastCalledWith("finalize_qualification_candidate", {
      sessionHandle: "qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    });

    await api.recordQualificationRun(
      "qualification-candidate-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    expect(invokeMock).toHaveBeenLastCalledWith("record_qualification_run", {
      candidateHandle: "qualification-candidate-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    });
  });
});
