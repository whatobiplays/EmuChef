import { render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";

import { ExecutionStep } from "../src/ExecutionStep";
import type { ExecutionSnapshot } from "../src/types";

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
});
