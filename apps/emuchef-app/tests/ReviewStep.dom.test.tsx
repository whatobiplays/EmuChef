import { render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";

import { ReviewStep } from "../src/ReviewStep";
import type { ReviewSummary } from "../src/types";

const review: ReviewSummary = {
  reviewHandle: "review-opaque",
  planDigest: "digest-opaque",
  target: {
    manufacturer: "AYANEO",
    model: "Pocket",
    androidVersion: 14,
    androidApiLevel: 34,
  },
  selectedInputs: [],
  groups: [],
  warnings: [],
};

describe("stale execution review", () => {
  test("blocks execution and directs the user to generate a fresh review", () => {
    render(
      <ReviewStep
        busy={false}
        executionKind="idle"
        realExecutionEnabled
        review={review}
        reviewStale
        onApplyToDevice={vi.fn()}
        onBack={vi.fn()}
        onStartSimulation={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: "Review is out of date" })).toBeTruthy();
    expect(screen.getByText(/cannot be run again/i)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Start Simulated Dry Run" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Apply to Device" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/generate a fresh review before running again/i)).toBeTruthy();
  });
});
