import { fireEvent, render, screen, within } from "@testing-library/react";
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
  features: [
    {
      name: "Example emulator",
      automaticallyAdded: false,
      sections: [
        {
          kind: "copies",
          label: "Copies",
          actions: [
            {
              title: "Copy firmware",
              description: "Place the selected firmware on the device.",
              requirement: "required",
              deviceLocation: "/sdcard/EmuChef/firmware",
            },
          ],
        },
      ],
    },
  ],
  notices: [],
  work: { actionCount: 1 },
  canExecute: true,
};

type CopyChanges = {
  selectedBoundary: string;
  additions: string[];
  replacements: string[];
  deletions: string[];
};

function reviewWithCopyChanges(copyChanges: CopyChanges | null) {
  return {
    ...review,
    features: review.features.map((feature) => ({
      ...feature,
      sections: feature.sections.map((section) => ({
        ...section,
        actions: section.actions.map((action) => ({ ...action, copyChanges })),
      })),
    })),
  };
}

describe("review presentation", () => {
  test("renders the backend-authored feature-first summary without technical authority data", () => {
    const { container } = render(
      <ReviewStep
        busy={false}
        executionKind="idle"
        realExecutionCompiled
        review={review}
        reviewStale={false}
        onApplyToDevice={vi.fn()}
        onBack={vi.fn()}
        onStartSimulation={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: "Pocket setup" })).toBeTruthy();
    expect(
      screen.getByRole("heading", { name: "Example emulator" }),
    ).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Copies" })).toBeTruthy();
    expect(screen.getByText("Copy firmware")).toBeTruthy();
    expect(screen.getByText("firmware.bin · Required")).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Copy changes" })).toBeNull();
    expect(container.textContent).not.toMatch(
      /digest|recipe[_-]?id|step[_-]?id/i,
    );
  });

  test("keeps the selected boundary and operation totals visible and expands the complete change list", () => {
    const copyChanges = {
      selectedBoundary: "/storage/emulated/0/Android/data/com.example/files",
      additions: [
        "Create the emulator profile",
        "Add controller mappings",
        "Install the app metadata",
      ],
      replacements: [
        "Replace the existing configuration",
        "Update the app manifest",
        "Refresh mappings",
      ],
      deletions: [
        "Remove the obsolete manifest",
        "Delete stale metadata",
        "Remove the old profile",
      ],
    };

    render(
      <ReviewStep
        busy={false}
        executionKind="idle"
        realExecutionCompiled
        review={reviewWithCopyChanges(copyChanges)}
        reviewStale={false}
        onApplyToDevice={vi.fn()}
        onBack={vi.fn()}
        onStartSimulation={vi.fn()}
      />,
    );

    const disclosure = screen.getByRole("region", { name: "Copy changes" });
    expect(
      within(disclosure).getByText(copyChanges.selectedBoundary),
    ).toBeTruthy();
    expect(within(disclosure).getByText("3 additions")).toBeTruthy();
    expect(within(disclosure).getByText("3 replacements")).toBeTruthy();
    expect(within(disclosure).getByText("3 deletions")).toBeTruthy();
    const additionSummary = within(disclosure).getByRole("region", {
      name: "Additions summary",
    });
    const replacementSummary = within(disclosure).getByRole("region", {
      name: "Replacements summary",
    });
    const deletionSummary = within(disclosure).getByRole("region", {
      name: "Deletions summary",
    });
    expect(
      within(additionSummary).getByText("Create the emulator profile"),
    ).toBeTruthy();
    expect(
      within(additionSummary).getByText("Add controller mappings"),
    ).toBeTruthy();
    expect(
      within(additionSummary).queryByText("Install the app metadata"),
    ).toBeNull();
    expect(
      within(replacementSummary).queryByText("Refresh mappings"),
    ).toBeNull();
    expect(
      within(deletionSummary).queryByText("Remove the old profile"),
    ).toBeNull();

    fireEvent.click(within(disclosure).getByText(/show all copy changes/i));

    const allAdditions = within(disclosure).getByRole("region", {
      name: "Additions",
    });
    const allReplacements = within(disclosure).getByRole("region", {
      name: "Replacements",
    });
    const allDeletions = within(disclosure).getByRole("region", {
      name: "Deletions",
    });
    expect(
      within(allAdditions).getByRole("heading", { name: "Additions" }),
    ).toBeTruthy();
    expect(
      within(allAdditions).getByText("Install the app metadata"),
    ).toBeTruthy();
    expect(
      within(allReplacements).getByRole("heading", { name: "Replacements" }),
    ).toBeTruthy();
    expect(
      within(allReplacements).getByText("Replace the existing configuration"),
    ).toBeTruthy();
    expect(
      within(allReplacements).getByText("Update the app manifest"),
    ).toBeTruthy();
    expect(within(allReplacements).getByText("Refresh mappings")).toBeTruthy();
    expect(
      within(allDeletions).getByRole("heading", { name: "Deletions" }),
    ).toBeTruthy();
    expect(
      within(allDeletions).getByText("Remove the obsolete manifest"),
    ).toBeTruthy();
    expect(
      within(allDeletions).getByText("Delete stale metadata"),
    ).toBeTruthy();
    expect(
      within(allDeletions).getByText("Remove the old profile"),
    ).toBeTruthy();
  });

  test("renders the selected boundary and omits empty copy operation groups", () => {
    const copyChanges = {
      selectedBoundary: "/storage/emulated/0/EmuChef",
      additions: [],
      replacements: [],
      deletions: [],
    };

    render(
      <ReviewStep
        busy={false}
        executionKind="idle"
        realExecutionCompiled
        review={reviewWithCopyChanges(copyChanges)}
        reviewStale={false}
        onApplyToDevice={vi.fn()}
        onBack={vi.fn()}
        onStartSimulation={vi.fn()}
      />,
    );

    const disclosure = screen.getByRole("region", { name: "Copy changes" });
    expect(
      within(disclosure).getByText(copyChanges.selectedBoundary),
    ).toBeTruthy();
    expect(
      within(disclosure).queryByRole("heading", { name: "Additions" }),
    ).toBeNull();
    expect(
      within(disclosure).queryByRole("heading", { name: "Replacements" }),
    ).toBeNull();
    expect(
      within(disclosure).queryByRole("heading", { name: "Deletions" }),
    ).toBeNull();
    expect(within(disclosure).queryByRole("list")).toBeNull();
  });

  test("blocks both execution paths when the backend cannot review the plan safely", () => {
    render(
      <ReviewStep
        busy={false}
        executionKind="idle"
        realExecutionCompiled
        review={{ ...review, canExecute: false }}
        reviewStale={false}
        onApplyToDevice={vi.fn()}
        onBack={vi.fn()}
        onStartSimulation={vi.fn()}
      />,
    );

    expect(
      (
        screen.getByRole("button", {
          name: "Start simulation",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (
        screen.getByRole("button", {
          name: "Apply to Device",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(screen.getByText(/cannot review safely/i)).toBeTruthy();
  });
});

describe("stale execution review", () => {
  test("blocks execution and directs the user to generate a fresh review", () => {
    render(
      <ReviewStep
        busy={false}
        executionKind="idle"
        realExecutionCompiled
        review={review}
        reviewStale
        onApplyToDevice={vi.fn()}
        onBack={vi.fn()}
        onStartSimulation={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Review is out of date" }),
    ).toBeTruthy();
    expect(screen.getByText(/cannot be run again/i)).toBeTruthy();
    expect(
      (
        screen.getByRole("button", {
          name: "Start simulation",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (
        screen.getByRole("button", {
          name: "Apply to Device",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      screen.getByText(/generate a fresh review before running again/i),
    ).toBeTruthy();
  });
});
