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

  it("exposes no frontend lifecycle orchestration commands", () => {
    for (const removed of [
      "refreshQualificationSession",
      "bindQualificationReview",
      "bindQualificationExecution",
      "finalizeQualificationCandidate",
    ]) {
      expect(Object.prototype.hasOwnProperty.call(api, removed)).toBe(false);
    }
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

  it("exposes only explicit operator actions over opaque session handles", async () => {
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

    await api.abandonQualificationSession(
      "qualification-session-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    expect(invokeMock).toHaveBeenLastCalledWith("abandon_qualification_session", {
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
