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
});
