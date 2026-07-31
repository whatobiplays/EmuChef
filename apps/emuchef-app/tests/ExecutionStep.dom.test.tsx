import { render, screen, within } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";

import { ExecutionStep } from "../src/ExecutionStep";
import type { ExecutionSnapshot, RealExecutionSnapshot } from "../src/types";

function failedSnapshot(): ExecutionSnapshot {
  return {
    executionHandle: "execution-opaque",
    reviewHandle: "review-opaque",
    simulated: true,
    verificationScope: "simulation_only",
    status: "failed",
    startedAt: "2026-07-20T12:00:00Z",
    finishedAt: "2026-07-20T12:00:01Z",
    latestSequence: 1,
    terminal: true,
    recipes: [],
    warnings: [],
    errors: [],
    progress: { currentFeature: null, currentAction: null },
    completion: {
      classification: "failed",
      counts: {
        total: 1,
        completed: 0,
        skipped: 0,
        blocked: 0,
        failed: 1,
        cancelled: 0,
        pending: 0,
      },
      warningCount: 0,
      partialChangesPossible: false,
      features: [],
    },
  };
}

describe("execution recovery actions", () => {
  test("failed work is presented as a repair-and-fresh-review flow rather than an in-place retry", () => {
    const { container } = render(
      <ExecutionStep
        execution={{
          kind: "terminal",
          generation: 1,
          mode: "simulated",
          snapshot: failedSnapshot(),
          events: [],
          eventCursor: 1,
          cancellationRequested: false,
        }}
        launchState="idle"
        repairPreparing={false}
        reportState="idle"
        onCancel={vi.fn()}
        onExportReport={vi.fn()}
        onLaunchConfiguredApp={vi.fn()}
        onPrepareRepair={vi.fn()}
        onReturn={vi.fn()}
      />,
    );

    const repairButton = screen.getByRole("button", { name: "Repair setup" });
    expect(repairButton.getAttribute("aria-describedby")).toBe("execution-repair-explanation");
    expect(screen.queryByRole("button", { name: "Retry failed work" })).toBeNull();
    expect(screen.getByRole("button", { name: "View previous review" })).toBeTruthy();
    expect(screen.getByText(/require a fresh plan and review before another run/i)).toBeTruthy();
    expect(screen.getByText(/are not retried in place/i)).toBeTruthy();
    expect(container.textContent).not.toContain("2026-07-20T12:00:00Z");
  });

  test("active execution shows backend-projected current work and safe-boundary cancellation", () => {
    const snapshot: ExecutionSnapshot = {
      ...failedSnapshot(),
      status: "running",
      terminal: false,
      finishedAt: null,
      progress: { currentFeature: "RetroArch", currentAction: "Copy core files" },
      completion: {
        ...failedSnapshot().completion,
        classification: "in_progress",
        counts: { ...failedSnapshot().completion.counts, failed: 0, pending: 1 },
      },
    };
    render(
      <ExecutionStep
        execution={{
          kind: "active",
          generation: 1,
          mode: "simulated",
          snapshot,
          events: [],
          eventCursor: 0,
          cancellationRequested: true,
        }}
        launchState="idle"
        repairPreparing={false}
        reportState="idle"
        onCancel={vi.fn()}
        onExportReport={vi.fn()}
        onLaunchConfiguredApp={vi.fn()}
        onPrepareRepair={vi.fn()}
        onReturn={vi.fn()}
      />,
    );

    expect(screen.getByText(/Current action:/)).toBeTruthy();
    expect(screen.getByText(/Copy core files/)).toBeTruthy();
    expect(screen.getByText(/current simulated atomic step may finish/i)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Cancellation requested" }) as HTMLButtonElement).disabled).toBe(true);
  });

  test("terminal pending work is presented as not attempted and completes the progress accounting", () => {
    const snapshot: ExecutionSnapshot = {
      ...failedSnapshot(),
      status: "cancelled",
      recipes: [{
        name: "Cancelled feature",
        description: null,
        status: "cancelled",
        steps: [
          { name: "Completed action", note: null, status: "succeeded", message: null },
          { name: "Never-started action", note: null, status: "pending", message: null },
        ],
      }],
      completion: {
        ...failedSnapshot().completion,
        classification: "cancelled",
        counts: {
          total: 2,
          completed: 1,
          skipped: 0,
          blocked: 0,
          failed: 0,
          cancelled: 0,
          pending: 1,
        },
      },
    };
    const { container } = render(
      <ExecutionStep
        execution={{
          kind: "terminal",
          generation: 1,
          mode: "simulated",
          snapshot,
          events: [],
          eventCursor: 0,
          cancellationRequested: true,
        }}
        launchState="idle"
        repairPreparing={false}
        reportState="idle"
        onCancel={vi.fn()}
        onExportReport={vi.fn()}
        onLaunchConfiguredApp={vi.fn()}
        onPrepareRepair={vi.fn()}
        onReturn={vi.fn()}
      />,
    );

    expect(screen.getAllByText("Not attempted")).toHaveLength(2);
    expect(screen.getByText("Not attempted", { selector: "dt" }).parentElement?.textContent).toBe("Not attempted1");
    expect((container.querySelector("progress") as HTMLProgressElement).value).toBe(1);
  });

  test("failed real work describes partial changes as possible when none are proven complete", () => {
    const snapshot: RealExecutionSnapshot = {
      ...failedSnapshot(),
      simulated: false,
      verificationScope: "real_device",
      target: { label: "Connected Android device" },
      launchAction: null,
      completion: {
        ...failedSnapshot().completion,
        partialChangesPossible: true,
      },
    };
    render(
      <ExecutionStep
        execution={{
          kind: "terminal",
          generation: 1,
          mode: "real",
          snapshot,
          events: [],
          eventCursor: 0,
          cancellationRequested: false,
        }}
        launchState="idle"
        repairPreparing={false}
        reportState="idle"
        onCancel={vi.fn()}
        onExportReport={vi.fn()}
        onLaunchConfiguredApp={vi.fn()}
        onPrepareRepair={vi.fn()}
        onReturn={vi.fn()}
      />,
    );

    expect(screen.getByText(/device changes may have occurred before this failed result/i)).toBeTruthy();
    expect(screen.queryByText(/some device changes completed before this failed result/i)).toBeNull();
  });

  test("separates failed and blocked results without exposing raw result names or changing order", () => {
    const snapshot: ExecutionSnapshot = {
      ...failedSnapshot(),
      recipes: [
        {
          name: "First feature",
          description: "The first result remains first.",
          status: "failed",
          steps: [{ name: "Install first feature", note: null, status: "failed", message: "Completed work could not be verified." }],
        },
        {
          name: "Second feature",
          description: "The blocked result remains second.",
          status: "blocked",
          steps: [{ name: "Install second feature", note: null, status: "blocked", message: "Required work was blocked." }],
        },
      ],
      warnings: [{
        message: "Review the connection before trying again.",
        remediation: { kind: "reconnect_device", title: "Reconnect", message: "Reconnect the intended device." },
      }],
      errors: [
        {
          message: "The first problem is separate.",
          remediation: { kind: "view_report", title: "Review", message: "Review the report." },
        },
        {
          message: "The second problem is separate.",
          remediation: { kind: "review_inputs", title: "Review inputs", message: "Review the selected inputs." },
        },
      ],
    };
    const { container } = render(
      <ExecutionStep
        execution={{
          kind: "terminal",
          generation: 1,
          mode: "simulated",
          snapshot,
          events: [
            { sequence: 1, timestamp: "2026-07-20T12:00:00Z", label: "Feature updated", status: "succeeded_with_warnings", issue: null },
            { sequence: 2, timestamp: "2026-07-20T12:00:01Z", label: "Internal status handled", status: "internal_result_name", issue: null },
          ],
          eventCursor: 2,
          cancellationRequested: false,
        }}
        launchState="idle"
        repairPreparing={false}
        reportState="idle"
        onCancel={vi.fn()}
        onExportReport={vi.fn()}
        onLaunchConfiguredApp={vi.fn()}
        onPrepareRepair={vi.fn()}
        onReturn={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: "Warning 1" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Problem 1" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Problem 2" })).toBeTruthy();
    expect(container.querySelectorAll(".result-card")).toHaveLength(3);

    const groups = Array.from(container.querySelectorAll<HTMLElement>(".execution-group"));
    expect(groups).toHaveLength(2);
    expect(groups[0].classList.contains("status-failed")).toBe(true);
    expect(groups[1].classList.contains("status-blocked")).toBe(true);
    expect(within(groups[0]).getByText("Failed", { selector: ".execution-status" })).toBeTruthy();
    expect(within(groups[1]).getByText("Blocked", { selector: ".execution-status" })).toBeTruthy();
    expect(groups[0].textContent).toContain("First feature");
    expect(groups[1].textContent).toContain("Second feature");
    expect(container.textContent).toContain("Completed with warnings");
    expect(container.textContent).toContain("Updated");
    expect(container.textContent).not.toMatch(/succeeded_with_warnings|internal_result_name/);
  });
});
