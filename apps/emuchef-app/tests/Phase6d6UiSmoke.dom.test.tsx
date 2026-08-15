import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { Phase6d6UiSmoke } from "../src/Phase6d6UiSmoke";
import type {
  Phase6d6UiSmokeStatus,
  RealExecutionSnapshot,
} from "../src/types";

const mockApi = vi.hoisted(() => ({
  phase6d6LoadProjection: vi.fn(),
  phase6d6Capture: vi.fn(),
}));

vi.mock("../src/api", () => ({ api: mockApi }));

function statusWithCandidates(): Phase6d6UiSmokeStatus {
  return {
    enabled: true,
    ready: true,
    message: null,
    candidates: [
      {
        subcase: "cancellation",
        handle: "binding-cancel-1",
        label: "Cancellation — active interruption — physical repetition 1",
        repetition: 1,
      },
      {
        subcase: "transport",
        handle: "binding-transport-1",
        label: "Transport — active interruption — physical repetition 1",
        repetition: 1,
      },
      {
        subcase: "root",
        handle: "binding-root-1",
        label: "Root — physical repetition 1",
        repetition: 1,
      },
      {
        subcase: "storage",
        handle: "binding-storage-1",
        label: "Storage — physical repetition 1",
        repetition: 1,
      },
    ],
  };
}

function terminalSnapshot(): RealExecutionSnapshot {
  return {
    executionHandle: "phase6d6-projection-test",
    reviewHandle: "phase6d6-ui-smoke-review",
    simulated: false,
    verificationScope: "real_device",
    target: { label: "Connected Android device" },
    status: "failed",
    startedAt: null,
    finishedAt: null,
    latestSequence: 0,
    terminal: true,
    recipes: [{
      name: "Reviewed device setup",
      description: null,
      status: "failed",
      steps: [
        { name: "Prepare reviewed setup", note: null, status: "succeeded", message: null },
        {
          name: "Apply reviewed changes",
          note: null,
          status: "failed",
          message: "This device operation did not complete.",
        },
        { name: "Verify completed setup", note: null, status: "pending", message: null },
      ],
    }],
    warnings: [],
    errors: [{
      message: "The device connection was lost during execution.",
      remediation: {
        kind: "reconnect_device",
        title: "Reconnect and requalify",
        message:
          "Reconnect or authorize the intended reviewed device, then complete fresh qualification and generate and review a fresh plan before another real run. Reconnecting does not resume the old execution.",
      },
    }],
    completion: {
      classification: "failed",
      counts: { total: 3, completed: 1, skipped: 0, blocked: 0, failed: 1, cancelled: 0, pending: 1 },
      warningCount: 0,
      partialChangesPossible: true,
      features: [{
        name: "Reviewed device setup",
        status: "failed",
        counts: { completed: 1, failed: 1, pending: 1 },
      }],
    },
    progress: { currentFeature: null, currentAction: null },
    launchAction: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

test("qualification shell shows the development label and keeps host sleep unavailable", () => {
  render(<Phase6d6UiSmoke status={statusWithCandidates()} />);
  expect(
    screen.getByRole("heading", { name: /Phase 6D\.6 development UI-smoke qualification/i }),
  ).toBeTruthy();
  expect(screen.getByText(/Host sleep/i)).toBeTruthy();
  expect(screen.getByText(/no accepted physical host-sleep binding/i)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Request cancellation" })).toBeNull();
  expect(screen.queryByRole("button", { name: /Apply to Device/i })).toBeNull();
});

test("loading a candidate renders the normal terminal UI and sends only the binding handle", async () => {
  const user = userEvent.setup();
  mockApi.phase6d6LoadProjection.mockResolvedValue({
    projectionHandle: "phase6d6-projection-1",
    snapshot: terminalSnapshot(),
  });
  render(<Phase6d6UiSmoke status={statusWithCandidates()} />);
  await user.click(
    screen.getByLabelText("Transport — active interruption — physical repetition 1"),
  );
  await user.click(screen.getByRole("button", { name: "Load projection" }));
  await waitFor(() =>
    expect(mockApi.phase6d6LoadProjection).toHaveBeenCalledWith("binding-transport-1"),
  );
  expect(
    await screen.findByRole("heading", { name: /Real-device installation failed/i }),
  ).toBeTruthy();
  expect(screen.getAllByText(/Not attempted/i).length).toBeGreaterThan(0);
  expect(screen.queryByRole("button", { name: "Request cancellation" })).toBeNull();
});

test("capture sends only the projection handle and UI repetition", async () => {
  const user = userEvent.setup();
  mockApi.phase6d6LoadProjection.mockResolvedValue({
    projectionHandle: "phase6d6-projection-1",
    snapshot: terminalSnapshot(),
  });
  mockApi.phase6d6Capture.mockResolvedValue({
    subcase: "transport",
    subRunId: `ui-subrun-sha256:${"a".repeat(64)}`,
    backendRunId: `physical-run-sha256:${"b".repeat(64)}`,
    backendTraceDigest: `sha256:${"c".repeat(64)}`,
    backendIssueCode: "device_transport_lost",
    developmentBuild: {
      identity: "emuchef-app:development-ui-smoke",
      version: "0.1.0",
      digest: `sha256:${"d".repeat(64)}`,
    },
    artifact: {
      kind: "ui_state_capture",
      path: "docs/testing/phase-6d6/evidence/ui/ui_state_transport_rep2_aaaa_bbbb.json",
      content: {
        backendRunId: `physical-run-sha256:${"b".repeat(64)}`,
        authoredTitle: "Reconnect and requalify",
        authoredIssueText: "The device connection was lost during execution.",
        authoredRemediation:
          "Reconnect or authorize the intended reviewed device, then complete fresh qualification and generate and review a fresh plan before another real run.",
        terminalStepProjection: "failed",
        notAttempted: 1,
        partialChangePresentation: "possible_partial_change",
        authorityInvalidated: true,
        recoveryState: "requalification_required",
        availableControls: ["export_report", "repair_setup", "fresh_workflow"],
      },
      digest: `sha256:${"e".repeat(64)}`,
    },
  });
  render(<Phase6d6UiSmoke status={statusWithCandidates()} />);
  await user.click(
    screen.getByLabelText("Transport — active interruption — physical repetition 1"),
  );
  await user.click(screen.getByRole("button", { name: "Load projection" }));
  await user.selectOptions(screen.getByLabelText("UI-smoke repetition"), "2");
  await user.click(screen.getByRole("button", { name: "Capture UI state" }));
  await waitFor(() =>
    expect(mockApi.phase6d6Capture).toHaveBeenCalledWith("phase6d6-projection-1", 2),
  );
  expect(await screen.findByText(/ui-subrun-sha256:/i)).toBeTruthy();
});

test("not-ready qualification status blocks selection with a sanitized message", () => {
  render(
    <Phase6d6UiSmoke
      status={{ enabled: true, ready: false, message: "ui_binding_index_invalid", candidates: [] }}
    />,
  );
  expect(screen.getByText("ui_binding_index_invalid")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Load projection" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Capture UI state" })).toBeNull();
});
