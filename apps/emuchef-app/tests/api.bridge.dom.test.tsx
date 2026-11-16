import { beforeEach, expect, test, vi } from "vitest";

const mockInvoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke: mockInvoke }));

import { api } from "../src/api";

beforeEach(() => {
  mockInvoke.mockReset();
});

test("prepares the execution review through the native bridge", async () => {
  const request = {
    deviceHandle: "device-opaque",
    selectionHandle: "selection-opaque",
    devicePlan: "t03.copy_fixture.base",
    selectedRecipes: ["t03.copy_app_target"],
    bindings: {
      "t03.copy_app_target/source": "/fixture/source",
      "t03.copy_app_target/policy": "merge",
    },
    requestGeneration: 1,
    mode: "dry_run" as const,
  };
  const review = { reviewHandle: "review-opaque" };
  mockInvoke.mockResolvedValue(review);

  await expect(api.prepareExecutionReview(request)).resolves.toBe(review);

  expect(mockInvoke).toHaveBeenCalledOnce();
  expect(mockInvoke).toHaveBeenCalledWith("prepare_execution_review", request);
});

test("lists Setup Locations for the requested device plan", async () => {
  const locations = [{
    id: "location-opaque",
    name: "Games",
    relativePath: "games/primary",
    volumeKind: "removableSd",
    volumeLabel: "SD card",
    ready: true,
    detail: "Available",
  }];
  mockInvoke.mockResolvedValue(locations);

  await expect(api.listSetupLocations("device-opaque", "plan.one", "selection-opaque")).resolves.toBe(locations);

  expect(mockInvoke).toHaveBeenCalledOnce();
  expect(mockInvoke).toHaveBeenCalledWith("list_setup_locations", {
    deviceHandle: "device-opaque",
    devicePlan: "plan.one",
    selectionHandle: "selection-opaque",
  });
});

test("saves a Setup Location with the current observation and opaque volume handle", async () => {
  const locations: Awaited<ReturnType<typeof api.listSetupLocations>> = [];
  mockInvoke.mockResolvedValue(locations);

  await expect(api.saveSetupLocation(
    "device-opaque",
    "plan.one",
    "observation-opaque",
    "volume-opaque",
    "location-existing",
    "Games",
    "games/primary",
    "selection-opaque",
  )).resolves.toBe(locations);

  expect(mockInvoke).toHaveBeenCalledOnce();
  expect(mockInvoke).toHaveBeenCalledWith("save_setup_location", {
    deviceHandle: "device-opaque",
    devicePlan: "plan.one",
    observationToken: "observation-opaque",
    volumeHandle: "volume-opaque",
    existingId: "location-existing",
    name: "Games",
    relativePath: "games/primary",
    selectionHandle: "selection-opaque",
  });
});

test("creates a Setup Location without an existing ID", async () => {
  mockInvoke.mockResolvedValue([]);

  await api.saveSetupLocation(
    "device-opaque",
    "plan.two",
    "observation-opaque",
    "volume-opaque",
    undefined,
    "Internal games",
    "games",
    "selection-created",
  );

  expect(mockInvoke).toHaveBeenCalledWith("save_setup_location", {
    deviceHandle: "device-opaque",
    devicePlan: "plan.two",
    observationToken: "observation-opaque",
    volumeHandle: "volume-opaque",
    existingId: null,
    name: "Internal games",
    relativePath: "games",
    selectionHandle: "selection-created",
  });
});

test("removes a Setup Location only within the requested device plan", async () => {
  const locations: Awaited<ReturnType<typeof api.listSetupLocations>> = [];
  mockInvoke.mockResolvedValue(locations);

  await expect(api.removeSetupLocation("device-opaque", "plan.one", "location-opaque", "selection-opaque"))
    .resolves.toBe(locations);

  expect(mockInvoke).toHaveBeenCalledOnce();
  expect(mockInvoke).toHaveBeenCalledWith("remove_setup_location", {
    deviceHandle: "device-opaque",
    devicePlan: "plan.one",
    id: "location-opaque",
    selectionHandle: "selection-opaque",
  });
});

test("describes configuration with the selected profile authority", async () => {
  const request = { deviceHandle: "device-opaque", selectionHandle: "selection-opaque", devicePlan: "plan.one", selectedRecipes: ["recipe.one"], bindings: { "recipe.one/theme": "dark" }, requestGeneration: 7 };
  const description = { devicePlan: "plan.one" };
  mockInvoke.mockResolvedValue(description);
  await expect(api.describeConfiguration(request)).resolves.toBe(description);
  expect(mockInvoke).toHaveBeenCalledExactlyOnceWith("describe_configuration", request);
});

for (const genericAcknowledged of [false, true]) {
  test(`selects a profile with its resolution token and acknowledgment ${genericAcknowledged}`, async () => {
    const selected = { selectionHandle: "selected-opaque" };
    mockInvoke.mockResolvedValue(selected);
    await expect(api.selectDeviceProfile("device-opaque", "resolution-opaque", "profile.one", genericAcknowledged)).resolves.toBe(selected);
    expect(mockInvoke).toHaveBeenCalledExactlyOnceWith("select_device_profile", { deviceHandle: "device-opaque", resolutionHandle: "resolution-opaque", profileId: "profile.one", genericAcknowledged });
  });
}
