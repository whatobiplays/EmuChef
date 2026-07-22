import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { UpdatesPanel } from "../src/UpdatesPanel";
import type { UpdatePanelState } from "../src/update-policy";

const readyState: UpdatePanelState = {
  open: true,
  checking: false,
  opening: false,
  status: {
    state: "update_available",
    currentVersion: "0.1.0",
    latestVersion: "0.2.0",
    publishedAt: null,
    expiresAt: null,
    notes: "A clearer update experience.",
    dmgSizeBytes: 12 * 1024 * 1024,
    dmgSha256: "internal-release-digest",
    minimumMacosVersion: null,
    minimumMacosVersionIsInformational: true,
    canOpenDownload: true,
    message: "An update is available.",
  },
};

let rectsSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  rectsSpy = vi.spyOn(HTMLElement.prototype, "getClientRects")
    .mockReturnValue([{}] as unknown as DOMRectList);
});

afterEach(() => rectsSpy.mockRestore());

describe("Updates dialog presentation", () => {
  test("uses the shared Close placement, traps focus, dismisses with Escape, and restores focus", async () => {
    const invoker = document.createElement("button");
    invoker.textContent = "Open updates";
    document.body.append(invoker);
    invoker.focus();
    const onClose = vi.fn();
    const onAnnounce = vi.fn();
    const { rerender } = render(
      <UpdatesPanel
        state={readyState}
        returnFocus={invoker}
        navigationBlocked={false}
        onClose={onClose}
        onCheck={vi.fn()}
        onOpenDownload={vi.fn()}
        onAnnounce={onAnnounce}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: "EmuChef updates" });
    const close = screen.getByRole("button", { name: "Close" });
    await waitFor(() => expect(document.activeElement).toBe(close));
    expect(close.classList.contains("secondary")).toBe(true);
    expect(close.classList.contains("text-button")).toBe(false);
    expect(close.closest(".dialog-heading")).toBeTruthy();

    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Open download page" }));
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);

    rerender(
      <UpdatesPanel
        state={{ ...readyState, open: false }}
        returnFocus={invoker}
        navigationBlocked={false}
        onClose={onClose}
        onCheck={vi.fn()}
        onOpenDownload={vi.fn()}
        onAnnounce={onAnnounce}
      />,
    );
    await waitFor(() => expect(document.activeElement).toBe(invoker));
    invoker.remove();
  });

  test("blocks unsafe dismissal with a real disabled state and explanation", async () => {
    const onClose = vi.fn();
    const onAnnounce = vi.fn();
    render(
      <UpdatesPanel
        state={{ ...readyState, checking: true }}
        returnFocus={null}
        navigationBlocked
        onClose={onClose}
        onCheck={vi.fn()}
        onOpenDownload={vi.fn()}
        onAnnounce={onAnnounce}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: "EmuChef updates" });
    const close = screen.getByRole("button", { name: "Close" }) as HTMLButtonElement;
    expect(close.disabled).toBe(true);
    expect(close.getAttribute("aria-describedby")).toBe("updates-close-reason");
    expect(screen.getByText(/Close is unavailable while the current update action finishes/)).toBeTruthy();
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    expect(onAnnounce).toHaveBeenCalledWith(
      "Updates cannot close until the current update action finishes.",
      true,
    );
  });

  test("keeps update trust consequences while hiding digest and signing internals", () => {
    const { container } = render(
      <UpdatesPanel
        state={readyState}
        returnFocus={null}
        navigationBlocked={false}
        onClose={vi.fn()}
        onCheck={vi.fn()}
        onOpenDownload={vi.fn()}
        onAnnounce={vi.fn()}
      />,
    );

    expect(screen.getByText("Current version")).toBeTruthy();
    expect(screen.getByText("Latest version")).toBeTruthy();
    expect(screen.getByText("Download size")).toBeTruthy();
    expect(screen.getByText(/EmuChef verifies the release information/)).toBeTruthy();
    expect(container.textContent).not.toMatch(/internal-release-digest|SHA-256|Developer ID|notarization|stapling|Gatekeeper/i);
  });
});
