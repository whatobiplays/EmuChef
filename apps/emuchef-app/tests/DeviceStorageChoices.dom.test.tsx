import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { DeviceStorageChoices, type DeviceStorageChoicesProps } from "../src/DeviceStorageChoices";
import type { DeviceVolumeChoice, DeviceVolumesProjection } from "../src/types";

const INTERNAL = "11111111111111111111111111111111";
const SD = "22222222222222222222222222222222";
const READ_ONLY_SD = "55555555555555555555555555555555";
const ADOPTED = "44444444444444444444444444444444";
const USB = "33333333333333333333333333333333";

function volume(overrides: Partial<DeviceVolumeChoice> = {}): DeviceVolumeChoice {
  const volumeHandle = overrides.volumeHandle ?? INTERNAL;
  return {
    presentationKey: volumeHandle,
    volumeHandle,
    kind: "internalShared",
    label: "Internal storage",
    status: "available",
    selectable: true,
    detail: "Mounted. EmuChef has not verified write access.",
    ...overrides,
  };
}

function projection(overrides: Partial<DeviceVolumesProjection> = {}): DeviceVolumesProjection {
  return {
    deviceHandle: "device-opaque",
    sessionEpoch: 1,
    observationToken: "aaaaaaaabbbbbbbbccccccccdddddddd",
    discoveryStatus: "observed",
    evidence: "live",
    volumes: [
      volume(),
      volume({ volumeHandle: SD, kind: "removableSd", label: "SD card" }),
    ],
    selectedVolumeHandle: null,
    selectionNotice: null,
    ...overrides,
  };
}

function choices(overrides: Partial<DeviceStorageChoicesProps> = {}): DeviceStorageChoicesProps {
  return {
    projection: projection(),
    loading: false,
    error: null,
    onRefresh: vi.fn(),
    onSelect: vi.fn(),
    announce: vi.fn(),
    ...overrides,
  };
}

test("renders storage options without preselecting a destination", () => {
  render(<DeviceStorageChoices {...choices()} />);

  expect(screen.getByRole("group", { name: "Storage options" })).toBeTruthy();
  const internal = screen.getByRole("radio", { name: "Internal storage" }) as HTMLInputElement;
  const sd = screen.getByRole("radio", { name: "SD card" }) as HTMLInputElement;
  expect(internal.checked).toBe(false);
  expect(sd.checked).toBe(false);
  expect(screen.getByText("Showing a storage report read from the device just now.")).toBeTruthy();
});

test("labels cached evidence as a recent report", () => {
  render(<DeviceStorageChoices {...choices({ projection: projection({ evidence: "cached" }) })} />);

  expect(screen.getByText("Showing a recent storage report. Refresh to read the device again.")).toBeTruthy();
  expect(screen.queryByText("Showing a storage report read from the device just now.")).toBeNull();
});

test("labels a refresh without claiming a recent report", () => {
  render(
    <DeviceStorageChoices
      {...choices({
        projection: projection({ discoveryStatus: "refreshing", evidence: "unavailable", volumes: [] }),
      })}
    />,
  );

  expect(screen.getByText("Storage options have not been read yet.")).toBeTruthy();
  expect(screen.getByRole("status").textContent).toBe("Reading storage options from the device.");
  expect(screen.queryByText("Showing a recent storage report. Refresh to read the device again.")).toBeNull();
});

test("renders a failed read as unavailable evidence with no retained choice", () => {
  render(
    <DeviceStorageChoices
      {...choices({
        projection: projection({
          discoveryStatus: "unavailable",
          evidence: "unavailable",
          volumes: [],
          selectedVolumeHandle: null,
        }),
        error: "Device storage volume details could not be read.",
      })}
    />,
  );

  expect(
    screen.getAllByText("Storage options could not be read. Refresh storage options to try again.")
      .length,
  ).toBeGreaterThan(0);
  expect(screen.queryByRole("radio")).toBeNull();
  expect(screen.queryByText("Showing a storage report read from the device just now.")).toBeNull();
  expect(screen.queryByText("Showing a recent storage report. Refresh to read the device again.")).toBeNull();
  expect(screen.getByRole("alert").textContent).toContain("could not be read");
});

test("records the chosen opaque handle", async () => {
  const user = userEvent.setup();
  const props = choices();
  render(<DeviceStorageChoices {...props} />);

  await user.click(screen.getByRole("radio", { name: "SD card" }));

  expect(props.onSelect).toHaveBeenCalledWith(SD, projection().observationToken);
});

test("lists unsupported volumes with reasons without making them selectable", () => {
  render(
    <DeviceStorageChoices
      {...choices({
        projection: projection({
          volumes: [
            volume(),
            volume({ volumeHandle: SD, kind: "removableSd", label: "SD card" }),
            volume({
              volumeHandle: READ_ONLY_SD,
              kind: "removableSd",
              label: "SD card 2",
              selectable: false,
              detail: "Mounted read-only. EmuChef cannot use read-only storage as a destination.",
            }),
            volume({
              volumeHandle: ADOPTED,
              kind: "adoptedPrivate",
              label: "Adopted storage",
              status: "unsupported",
              selectable: false,
              detail: "Adopted private storage is not a portable destination.",
            }),
            volume({
              volumeHandle: USB,
              kind: "usb",
              label: "USB storage",
              status: "unsupported",
              selectable: false,
              detail: "USB storage is not supported for this device.",
            }),
          ],
        }),
      })}
    />,
  );

  expect(screen.getByText("Not offered as a destination")).toBeTruthy();
  expect(screen.queryByText("Not available")).toBeNull();
  expect(screen.getByText("Adopted private storage is not a portable destination.")).toBeTruthy();
  expect(screen.getByText("USB storage is not supported for this device.")).toBeTruthy();
  expect(
    screen.getByText("Mounted read-only. EmuChef cannot use read-only storage as a destination."),
  ).toBeTruthy();
  expect(screen.queryByRole("radio", { name: /^Adopted storage/ })).toBeNull();
  expect(screen.queryByRole("radio", { name: /^USB storage/ })).toBeNull();
  expect(screen.getAllByRole("radio")).toHaveLength(2);
});

test("renders an invalidation notice in one live region without a shell announcement", () => {
  const notice = "This device's storage session changed. Refresh storage options and choose again.";
  const props = choices({ projection: projection({ selectionNotice: notice }) });
  render(<DeviceStorageChoices {...props} />);

  expect(screen.getByText(notice)).toBeTruthy();
  expect(screen.getAllByRole("status")).toHaveLength(1);
  expect(props.announce).not.toHaveBeenCalled();
});

test("refresh requests a live observation and disables itself while reading", async () => {
  const user = userEvent.setup();
  const props = choices();
  const { rerender } = render(<DeviceStorageChoices {...props} />);

  await user.click(screen.getByRole("button", { name: "Refresh storage" }));
  expect(props.onRefresh).toHaveBeenCalledTimes(1);

  rerender(<DeviceStorageChoices {...props} loading />);
  const busy = screen.getByRole("button", { name: "Refreshing storage…" }) as HTMLButtonElement;
  expect(busy.disabled).toBe(true);
  expect(busy.getAttribute("aria-describedby")).toBeTruthy();
  expect(screen.getByText("Refreshing storage options…")).toBeTruthy();
});

test("blocks selection during refresh while preserving keyboard focus", async () => {
  const user = userEvent.setup();
  const props = choices({ loading: true });
  render(<DeviceStorageChoices {...props} />);
  const sd = screen.getByRole("radio", { name: "SD card" }) as HTMLInputElement;

  expect(sd.getAttribute("aria-disabled")).toBe("true");
  sd.focus();
  expect(document.activeElement).toBe(sd);

  await user.keyboard(" ");

  expect(document.activeElement).toBe(sd);
  expect(props.onSelect).not.toHaveBeenCalled();
  expect(sd.checked).toBe(false);
});

test("uses selection-specific busy text instead of claiming a refresh", () => {
  render(<DeviceStorageChoices {...choices({ loading: true, loadingKind: "selection" })} />);

  const busy = screen.getByRole("button", { name: "Selecting storage…" }) as HTMLButtonElement;
  expect(busy.disabled).toBe(true);
  expect(screen.queryByText("Refreshing storage options…")).toBeNull();
  expect(screen.getAllByText("Selecting storage…")).toHaveLength(2);
});

test("selects with the keyboard", async () => {
  const user = userEvent.setup();
  const props = choices();
  render(<DeviceStorageChoices {...props} />);

  await user.tab();
  expect(document.activeElement).toBe(screen.getByRole("radio", { name: "Internal storage" }));
  await user.keyboard(" ");

  expect(props.onSelect).toHaveBeenCalledWith(INTERNAL, projection().observationToken);
});

test("moves focus to Refresh storage and announces when the focused option disappears", async () => {
  const user = userEvent.setup();
  const announce = vi.fn();
  const props = choices({ announce });
  const { rerender } = render(<DeviceStorageChoices {...props} />);
  const sd = screen.getByRole("radio", { name: "SD card" });
  await user.click(sd);
  expect(document.activeElement).toBe(sd);

  rerender(<DeviceStorageChoices {...props} loading />);
  rerender(
    <DeviceStorageChoices
      {...props}
      loading={false}
      projection={projection({ volumes: [volume()] })}
    />,
  );

  expect(screen.queryByRole("radio", { name: "SD card" })).toBeNull();
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Refresh storage" }));
  expect(announce).toHaveBeenCalledWith(
    "That storage option is no longer available after refresh. Choose another option or refresh again.",
    true,
  );
});

test("renders no raw device identifiers", () => {
  const { container } = render(
    <DeviceStorageChoices
      {...choices({
        projection: projection({
          volumes: [
            volume({ detail: "Mounted. EmuChef has not verified write access." }),
            volume({ volumeHandle: SD, kind: "removableSd", label: "SD card", detail: "Mounted read-only." }),
          ],
        }),
      })}
    />,
  );

  const rendered = container.innerHTML;
  for (const forbidden of ["/storage/", "1234-ABCD", "T04-SERIAL", "fsUuid", "diskId"]) {
    expect(rendered.includes(forbidden)).toBe(false);
  }
  expect(rendered.includes(INTERNAL)).toBe(true);
});

test("states when no storage can be chosen", () => {
  const props = choices({
    projection: projection({
      volumes: [
        volume({
          volumeHandle: ADOPTED,
          kind: "adoptedPrivate",
          label: "Adopted storage",
          status: "unsupported",
          selectable: false,
          detail: "Adopted private storage is not a portable destination.",
        }),
      ],
    }),
  });
  render(<DeviceStorageChoices {...props} />);

  expect(screen.queryAllByRole("radio")).toHaveLength(0);
  expect(
    screen.getByText(
      "No selectable storage is available right now. EmuChef has not chosen a storage option for you.",
    ),
  ).toBeTruthy();
});

test("reports a device inventory gap truthfully", () => {
  render(
    <DeviceStorageChoices
      {...choices({ projection: projection({ discoveryStatus: "noDevice", volumes: [] }) })}
    />,
  );

  expect(screen.getByText("No connected device is available for storage discovery.")).toBeTruthy();
  expect(screen.queryAllByRole("radio")).toHaveLength(0);
});

test("never claims write access from a mount state", () => {
  render(<DeviceStorageChoices {...choices()} />);

  const rendered = document.body.textContent ?? "";
  expect(rendered.includes("Mounted. EmuChef has not verified write access.")).toBe(true);
  expect(rendered.includes("read-write")).toBe(false);
  expect(rendered.includes("A mount does not prove EmuChef can write to the storage")).toBe(true);
  expect(rendered.includes("writable")).toBe(false);
  expect(rendered.includes("execution eligible")).toBe(false);
});

test("announces a read failure through one live region", () => {
  render(
    <DeviceStorageChoices
      {...choices({
        projection: projection({
          discoveryStatus: "unavailable",
          volumes: [],
          selectedVolumeHandle: null,
        }),
        error: "Storage discovery is unavailable.",
      })}
    />,
  );

  expect(screen.getAllByRole("alert")).toHaveLength(1);
  expect(screen.queryAllByRole("status")).toHaveLength(0);
});

test("surfaces a trusted bridge error", () => {
  render(<DeviceStorageChoices {...choices({ projection: null, error: "Storage discovery is unavailable." })} />);

  expect(screen.getByRole("alert").textContent).toBe("Storage discovery is unavailable.");
  expect(screen.getByText("Storage options have not been read yet.")).toBeTruthy();
});

test("uses the exact concise radio name and exposes mount detail as a description", () => {
  render(<DeviceStorageChoices {...choices()} />);

  const internal = screen.getByRole("radio", { name: "Internal storage" });
  expect(screen.queryByRole("radio", { name: /Mounted/ })).toBeNull();
  const describedBy = internal.getAttribute("aria-describedby") ?? "";
  const descriptionText = describedBy
    .split(" ")
    .map((id) => document.getElementById(id)?.textContent ?? "")
    .join(" ");
  expect(descriptionText).toContain("Mounted. EmuChef has not verified write access.");
});

test("tracks focus on an unselected radio and restores it after refresh", async () => {
  const user = userEvent.setup();
  const props = choices();
  const { rerender } = render(<DeviceStorageChoices {...props} />);
  await user.tab();
  expect(document.activeElement).toBe(screen.getByRole("radio", { name: "Internal storage" }));
  const sd = screen.getByRole("radio", { name: "SD card" });
  (sd as HTMLElement).focus();
  expect(document.activeElement).toBe(sd);
  expect((sd as HTMLInputElement).checked).toBe(false);

  rerender(<DeviceStorageChoices {...props} loading />);
  (sd as HTMLElement).blur();
  rerender(<DeviceStorageChoices {...props} loading={false} />);

  expect(document.activeElement).toBe(screen.getByRole("radio", { name: "SD card" }));
});

test("does not move focus the user placed on another control", async () => {
  const user = userEvent.setup();
  const announce = vi.fn();
  const props = choices({ announce });
  const { rerender } = render(<DeviceStorageChoices {...props} />);
  const sd = screen.getByRole("radio", { name: "SD card" });
  await user.click(sd);
  rerender(<DeviceStorageChoices {...props} loading />);
  const internal = screen.getByRole("radio", { name: "Internal storage" });
  (internal as HTMLElement).focus();
  rerender(
    <DeviceStorageChoices
      {...props}
      loading={false}
      projection={projection({ volumes: [volume()] })}
    />,
  );

  expect(document.activeElement).toBe(internal);
  expect(announce).not.toHaveBeenCalled();
});

test("restores focus and announces once when a radio focused during loading disappears", async () => {
  const user = userEvent.setup();
  const announce = vi.fn();
  const props = choices({ announce });
  const { rerender } = render(<DeviceStorageChoices {...props} />);

  await user.click(screen.getByRole("button", { name: "Refresh storage" }));
  rerender(<DeviceStorageChoices {...props} loading />);
  const sd = screen.getByRole("radio", { name: "SD card" });
  (sd as HTMLElement).focus();
  expect(document.activeElement).toBe(sd);

  rerender(
    <DeviceStorageChoices
      {...props}
      loading={false}
      projection={projection({ volumes: [volume()] })}
    />,
  );

  expect(screen.queryByRole("radio", { name: "SD card" })).toBeNull();
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Refresh storage" }));
  expect(announce).toHaveBeenCalledTimes(1);
  expect(announce).toHaveBeenCalledWith(expect.stringContaining("no longer available"), true);
});

test("selection notice owns invalidation when a focused selected option disappears", async () => {
  const user = userEvent.setup();
  const announce = vi.fn();
  const notice = "The selected storage is no longer available. Choose another storage option.";
  const props = choices({ announce });
  const selected = projection({ selectedVolumeHandle: SD });
  const { rerender } = render(<DeviceStorageChoices {...props} projection={selected} />);

  const sd = screen.getByRole("radio", { name: "SD card" });
  await user.click(sd);
  expect((sd as HTMLInputElement).checked).toBe(true);
  rerender(<DeviceStorageChoices {...props} loading projection={selected} />);
  rerender(
    <DeviceStorageChoices
      {...props}
      loading={false}
      projection={projection({
        volumes: [volume()],
        selectedVolumeHandle: null,
        selectionNotice: notice,
      })}
    />,
  );

  expect(screen.queryByRole("radio", { name: "SD card" })).toBeNull();
  expect(screen.getAllByRole("status")).toHaveLength(1);
  expect(screen.getByRole("status").textContent).toBe(notice);
  expect(announce).not.toHaveBeenCalled();
});

test("a failed refresh lets the alert own the error without an option-loss announcement", async () => {
  const user = userEvent.setup();
  const announce = vi.fn();
  const props = choices({ announce });
  const { rerender } = render(<DeviceStorageChoices {...props} />);
  const sd = screen.getByRole("radio", { name: "SD card" });
  await user.click(sd);
  rerender(<DeviceStorageChoices {...props} loading />);
  (sd as HTMLElement).blur();
  rerender(
    <DeviceStorageChoices
      {...props}
      loading={false}
      error="Device storage volume details could not be read."
      projection={projection({
        discoveryStatus: "unavailable",
        evidence: "unavailable",
        volumes: [],
        selectedVolumeHandle: null,
      })}
    />,
  );

  expect(screen.getAllByRole("alert")).toHaveLength(1);
  expect(screen.queryAllByRole("status")).toHaveLength(0);
  expect(announce).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Refresh storage" }));
});

test("duplicate non-selectable presentation entries use distinct React keys", () => {
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  render(
    <DeviceStorageChoices
      {...choices({
        projection: projection({
          volumes: [
            volume({
              presentationKey: SD + ":1",
              volumeHandle: SD,
              kind: "removableSd",
              label: "SD card",
              status: "ambiguous",
              selectable: false,
              detail: "EmuChef cannot tell these storage options apart.",
            }),
            volume({
              presentationKey: SD + ":2",
              volumeHandle: SD,
              kind: "removableSd",
              label: "SD card",
              status: "ambiguous",
              selectable: false,
              detail: "EmuChef cannot tell these storage options apart.",
            }),
          ],
        }),
      })}
    />,
  );

  expect(screen.getAllByText("SD card")).toHaveLength(2);
  expect(consoleError).not.toHaveBeenCalled();
  consoleError.mockRestore();
});
