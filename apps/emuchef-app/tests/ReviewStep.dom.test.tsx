import { render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";

import { ReviewStep } from "../src/ReviewStep";
import type { ReviewSummary } from "../src/types";

const review: ReviewSummary = {
  reviewHandle: "review-opaque",
  setup: { name: "Pocket setup", description: "A complete handheld setup." },
  target: {
    label: "Connected Android device",
    manufacturer: "AYANEO",
    model: "Pocket",
    androidVersion: 14,
    androidApiLevel: 34,
  },
  inputs: [{ label: "BIOS file", summary: "firmware.bin", required: true }],
  features: [{
    name: "Example emulator",
    automaticallyAdded: false,
    sections: [{
      kind: "copies",
      label: "Copies",
      actions: [{
        title: "Copy firmware",
        description: "Place the selected firmware on the device.",
        requirement: "required",
        deviceLocation: "/sdcard/EmuChef/firmware",
      }],
    }],
  }],
  notices: [],
  work: { actionCount: 1 },
  canExecute: true,
};

describe("review presentation", () => {
  test("renders the backend-authored feature-first summary without technical authority data", () => {
    const { container } = render(
      <ReviewStep
        busy={false}
        executionKind="idle"
        realExecutionEnabled
        review={review}
        reviewStale={false}
        onApplyToDevice={vi.fn()}
        onBack={vi.fn()}
        onStartSimulation={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: "Pocket setup" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Example emulator" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Copies" })).toBeTruthy();
    expect(screen.getByText("Copy firmware")).toBeTruthy();
    expect(screen.getByText("firmware.bin · Required")).toBeTruthy();
    expect(container.textContent).not.toMatch(/digest|recipe[_-]?id|step[_-]?id/i);
  });

  test("blocks both execution paths when the backend cannot review the plan safely", () => {
    render(
      <ReviewStep
        busy={false}
        executionKind="idle"
        realExecutionEnabled
        review={{ ...review, canExecute: false }}
        reviewStale={false}
        onApplyToDevice={vi.fn()}
        onBack={vi.fn()}
        onStartSimulation={vi.fn()}
      />,
    );

    expect((screen.getByRole("button", { name: "Start simulation" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Apply to Device" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/cannot review safely/i)).toBeTruthy();
  });
});

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
    expect((screen.getByRole("button", { name: "Start simulation" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Apply to Device" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/generate a fresh review before running again/i)).toBeTruthy();
  });
});
