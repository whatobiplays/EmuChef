import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import {
  DeviceStorageChoices,
  type DeviceStorageChoicesProps,
} from "../src/DeviceStorageChoices";
import { InputsStep } from "../src/InputsStep";
import type {
  ConfigurationDescription,
  DeviceVolumeChoice,
  DeviceVolumesProjection,
  SetupLocationProjection,
} from "../src/types";

const INTERNAL = "volume-internal-opaque";
const SD = "volume-sd-opaque";
const OBSERVATION = "observation-opaque";

function location(overrides: Partial<SetupLocationProjection> = {}): SetupLocationProjection {
  return {
    id: "location-opaque",
    name: "Games",
    relativePath: "games/primary",
    volumeKind: "removableSd",
    volumeLabel: "SD card",
    ready: true,
    detail: "Available for selection.",
    ...overrides,
  };
}

function volume(overrides: Partial<DeviceVolumeChoice> = {}): DeviceVolumeChoice {
  const volumeHandle = overrides.volumeHandle ?? INTERNAL;
  return {
    presentationKey: volumeHandle,
    volumeHandle,
    kind: "internalShared",
    label: "Internal storage",
    status: "available",
    selectable: true,
    detail: "Mounted.",
    ...overrides,
  };
}

function volumes(overrides: Partial<DeviceVolumesProjection> = {}): DeviceVolumesProjection {
  return {
    deviceHandle: "device-opaque",
    sessionEpoch: 1,
    observationToken: OBSERVATION,
    discoveryStatus: "observed",
    evidence: "live",
    volumes: [
      volume(),
      volume({ volumeHandle: SD, kind: "removableSd", label: "SD card" }),
    ],
    selectedVolumeHandle: INTERNAL,
    selectionNotice: null,
    ...overrides,
  };
}

function setupLocationProps(overrides: Partial<DeviceStorageChoicesProps> = {}) {
  return {
    projection: volumes(),
    loading: false,
    error: null,
    onRefresh: vi.fn(),
    onSelect: vi.fn(),
    announce: vi.fn(),
    devicePlan: "plan.one",
    setupLocations: [],
    setupLocationLoading: false,
    setupLocationError: null,
    onSaveSetupLocation: vi.fn().mockResolvedValue(true),
    onRemoveSetupLocation: vi.fn(),
    ...overrides,
  };
}

function renderSetupLocationInput(listStatus: "loading" | "unavailable" | "ready") {
  const description: ConfigurationDescription = {
    devicePlan: "plan.one",
    selectedRecipes: [],
    expandedRecipes: [],
    recipeOptions: [],
    inputs: [
      {
        key: "recipe.demo.destination",
        recipeId: "recipe.demo",
        inputId: "destination",
        type: "setup_location_ref",
        label: "Destination",
        description: null,
        required: true,
        sensitive: false,
        value: null,
        valueSource: null,
        diagnostics: [],
      },
    ],
    diagnostics: [],
  };

  return render(
    <InputsStep
      description={description}
      setupLocations={[]}
      setupLocationListStatus={listStatus}
      bindings={{}}
      busy={false}
      touchedInputKeys={new Set()}
      validationRequested={false}
      validationErrors={[]}
      validationSummaryRef={createRef<HTMLElement>()}
      onBack={vi.fn()}
      onBindingChange={vi.fn()}
      onClearInput={vi.fn()}
      onPickInput={vi.fn()}
      onRemoveInputEntry={vi.fn()}
      onRefreshValidation={vi.fn()}
      onReview={vi.fn()}
    />,
  );
}

test("saves a named location using a fresh opaque volume handle and observation token", async () => {
  const user = userEvent.setup();
  const props = setupLocationProps();
  render(<DeviceStorageChoices {...props} />);

  await user.selectOptions(screen.getByLabelText("Storage volume for Setup Location"), SD);
  await user.type(screen.getByLabelText("Setup Location name"), "Games");
  await user.type(screen.getByLabelText("Relative folder"), "games/primary");
  await user.click(screen.getByRole("button", { name: "Save Setup Location" }));

  expect(props.onSaveSetupLocation).toHaveBeenCalledWith(
    SD,
    OBSERVATION,
    undefined,
    "Games",
    "games/primary",
  );
});

test("keeps an in-progress Setup Location draft when the first storage observation arrives", async () => {
  const user = userEvent.setup();
  const props = setupLocationProps({ projection: null, loading: true });
  const { rerender } = render(<DeviceStorageChoices {...props} />);

  await user.type(screen.getByLabelText("Setup Location name"), "Games");
  await user.type(screen.getByLabelText("Relative folder"), "games/primary");

  rerender(<DeviceStorageChoices {...props} projection={volumes()} loading={false} />);

  expect((screen.getByLabelText("Setup Location name") as HTMLInputElement).value)
    .toBe("Games");
  expect((screen.getByLabelText("Relative folder") as HTMLInputElement).value)
    .toBe("games/primary");

  rerender(
    <DeviceStorageChoices
      {...props}
      projection={volumes({ deviceHandle: "next-device", sessionEpoch: 2 })}
      loading={false}
    />,
  );

  expect((screen.getByLabelText("Setup Location name") as HTMLInputElement).value)
    .toBe("");
  expect((screen.getByLabelText("Relative folder") as HTMLInputElement).value)
    .toBe("");
});

test("clears the Setup Location draft when a live observation becomes unavailable", async () => {
  const user = userEvent.setup();
  const props = setupLocationProps();
  const { rerender } = render(<DeviceStorageChoices {...props} />);

  await user.selectOptions(screen.getByLabelText("Storage volume for Setup Location"), SD);
  await user.type(screen.getByLabelText("Setup Location name"), "Games");
  await user.type(screen.getByLabelText("Relative folder"), "games/primary");

  rerender(
    <DeviceStorageChoices
      {...props}
      projection={volumes({
        observationToken: "",
        discoveryStatus: "unavailable",
        evidence: "unavailable",
        volumes: [],
        selectedVolumeHandle: null,
      })}
    />,
  );

  expect((screen.getByLabelText("Setup Location name") as HTMLInputElement).value)
    .toBe("");
  expect((screen.getByLabelText("Relative folder") as HTMLInputElement).value)
    .toBe("");
  expect((screen.getByLabelText("Storage volume for Setup Location") as HTMLSelectElement).value)
    .toBe("");
});

test("editing keeps the location ID while requiring an explicit current volume choice", async () => {
  const user = userEvent.setup();
  const props = setupLocationProps({ setupLocations: [location()] });
  render(<DeviceStorageChoices {...props} />);

  await user.click(screen.getByRole("button", { name: "Edit Games" }));
  expect((screen.getByRole("button", { name: "Update Setup Location" }) as HTMLButtonElement).disabled)
    .toBe(true);
  expect((screen.getByLabelText("Storage volume for Setup Location") as HTMLSelectElement).value)
    .toBe("");
  await user.selectOptions(screen.getByLabelText("Storage volume for Setup Location"), INTERNAL);
  await user.clear(screen.getByLabelText("Relative folder"));
  await user.type(screen.getByLabelText("Relative folder"), "games/alternate");
  await user.click(screen.getByRole("button", { name: "Update Setup Location" }));

  expect(props.onSaveSetupLocation).toHaveBeenCalledWith(
    INTERNAL,
    OBSERVATION,
    "location-opaque",
    "Games",
    "games/alternate",
  );
});

test("cached T04 evidence disables updating an existing Setup Location", async () => {
  const user = userEvent.setup();
  const props = setupLocationProps({
    projection: volumes({ evidence: "cached" }),
    setupLocations: [location()],
  });
  render(<DeviceStorageChoices {...props} />);

  await user.click(screen.getByRole("button", { name: "Edit Games" }));

  expect((screen.getByRole("button", { name: "Update Setup Location" }) as HTMLButtonElement).disabled)
    .toBe(true);
  expect((screen.getByLabelText("Storage volume for Setup Location") as HTMLSelectElement).disabled)
    .toBe(true);
  expect(props.onSaveSetupLocation).not.toHaveBeenCalled();
});

test("removing the edited Setup Location clears its form after the list updates", async () => {
  const user = userEvent.setup();
  const props = setupLocationProps({ setupLocations: [location()] });
  const { rerender } = render(<DeviceStorageChoices {...props} />);

  await user.click(screen.getByRole("button", { name: "Edit Games" }));
  await user.click(screen.getByRole("button", { name: "Remove Games" }));

  expect(props.onRemoveSetupLocation).toHaveBeenCalledWith("location-opaque");
  expect(screen.getByRole("button", { name: "Update Setup Location" })).not.toBeNull();

  rerender(<DeviceStorageChoices {...props} setupLocations={[]} />);

  expect(screen.getByRole("button", { name: "Save Setup Location" })).not.toBeNull();
  expect(screen.queryByRole("button", { name: "Update Setup Location" })).toBeNull();

  await user.selectOptions(screen.getByLabelText("Storage volume for Setup Location"), INTERNAL);
  await user.type(screen.getByLabelText("Setup Location name"), "Replacement");
  await user.type(screen.getByLabelText("Relative folder"), "games/recreated");
  await user.click(screen.getByRole("button", { name: "Save Setup Location" }));

  expect(props.onSaveSetupLocation).toHaveBeenLastCalledWith(
    INTERNAL,
    OBSERVATION,
    undefined,
    "Replacement",
    "games/recreated",
  );
});

test("Setup Location input reports a loading list without claiming it is empty", () => {
  renderSetupLocationInput("loading");
  const status = screen.getByRole("status");
  expect(status.textContent).toBe("Loading saved Setup Locations…");
  expect(screen.getByLabelText("Destination").getAttribute("aria-describedby"))
    .toContain(status.id);
  expect(
    screen.queryByText("No named Setup Locations are available for this device plan. Define one in device setup."),
  ).toBeNull();
});

test("Setup Location input reports an unavailable list without claiming it is empty", () => {
  renderSetupLocationInput("unavailable");
  const status = screen.getByRole("status");
  expect(status.textContent)
    .toBe("Saved Setup Locations could not be loaded. Refresh device storage to retry.");
  expect(screen.getByLabelText("Destination").getAttribute("aria-describedby"))
    .toContain(status.id);
  expect(
    screen.queryByText("No named Setup Locations are available for this device plan. Define one in device setup."),
  ).toBeNull();
});

test("Setup Location input reports a successfully empty list", () => {
  renderSetupLocationInput("ready");
  const status = screen.getByRole("status");
  expect(status.textContent)
    .toBe("No named Setup Locations are available for this device plan. Define one in device setup.");
  expect(screen.getByLabelText("Destination").getAttribute("aria-describedby"))
    .toContain(status.id);
});

test("removes only the selected Setup Location ID", async () => {
  const user = userEvent.setup();
  const props = setupLocationProps({ setupLocations: [location()] });
  render(<DeviceStorageChoices {...props} />);

  await user.click(screen.getByRole("button", { name: "Remove Games" }));

  expect(props.onRemoveSetupLocation).toHaveBeenCalledWith("location-opaque");
});

test("requires a live T04 observation before creating or updating a location", () => {
  const props = setupLocationProps({
    projection: volumes({ evidence: "cached" }),
  });
  render(<DeviceStorageChoices {...props} />);

  expect(screen.getByLabelText("Storage volume for Setup Location").hasAttribute("disabled"))
    .toBe(true);
  expect(screen.getByRole("button", { name: "Save Setup Location" }).hasAttribute("disabled"))
    .toBe(true);
  expect(screen.getByText("Refresh storage options before adding or updating a Setup Location."))
    .toBeTruthy();
});

test("renders safe Setup Location references without exposing physical binding data", async () => {
  const user = userEvent.setup();
  const onBindingChange = vi.fn();
  const description: ConfigurationDescription = {
    devicePlan: "plan.one",
    selectedRecipes: ["recipe.one"],
    expandedRecipes: ["recipe.one"],
    recipeOptions: [],
    inputs: [{
      key: "recipe.one/destination",
      recipeId: "recipe.one",
      inputId: "destination",
      type: "setup_location_ref",
      label: "Install destination",
      description: "Choose a named destination.",
      required: true,
      sensitive: false,
      value: null,
      valueSource: null,
      diagnostics: [],
    }],
    diagnostics: [],
  };
  const runtimeShapedLocation = Object.assign(location(), {
    mountRoot: "/private/T05_MOUNT_ROOT_SENTINEL",
    deviceSerial: "T05_DEVICE_SERIAL_SENTINEL",
    filesystemUuid: "T05_FILESYSTEM_UUID_SENTINEL",
    volumeIdentity: "T05_VOLUME_IDENTITY_SENTINEL",
    volumeContinuity: "T05_VOLUME_CONTINUITY_SENTINEL",
  });
  const setupLocations = [
    runtimeShapedLocation,
    location({
      id: "location-stale-opaque",
      name: "Old card",
      relativePath: "archive",
      ready: false,
      detail: "Refresh storage options to verify this location.",
    }),
  ];

  const { container } = render(
    <InputsStep
      description={description}
      bindings={{}}
      busy={false}
      touchedInputKeys={new Set()}
      validationRequested={false}
      validationErrors={[]}
      validationSummaryRef={createRef<HTMLElement>()}
      onBack={vi.fn()}
      onBindingChange={onBindingChange}
      onClearInput={vi.fn()}
      onPickInput={vi.fn()}
      onRemoveInputEntry={vi.fn()}
      onRefreshValidation={vi.fn()}
      onReview={vi.fn()}
      setupLocations={setupLocations}
      setupLocationListStatus="ready"
    />,
  );

  const picker = screen.getByLabelText("Install destination") as HTMLSelectElement;
  expect(screen.getByRole("option", { name: /Games.*games\/primary.*SD card/ })).toBeTruthy();
  expect(screen.getByRole("option", { name: /Old card.*archive.*SD card.*Refresh/ }).hasAttribute("disabled"))
    .toBe(true);
  await user.selectOptions(picker, "location-opaque");
  expect(onBindingChange).toHaveBeenCalledWith("recipe.one/destination", "location-opaque");
  const rendered = container.innerHTML;
  for (const forbidden of [
    "/private/T05_MOUNT_ROOT_SENTINEL",
    "T05_DEVICE_SERIAL_SENTINEL",
    "T05_FILESYSTEM_UUID_SENTINEL",
    "T05_VOLUME_IDENTITY_SENTINEL",
    "T05_VOLUME_CONTINUITY_SENTINEL",
  ]) {
    expect(rendered).not.toContain(forbidden);
  }
});
