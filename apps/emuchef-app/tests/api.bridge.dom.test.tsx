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
    devicePlan: "t03.copy_fixture.base",
    selectedRecipes: ["t03.copy_app_target"],
    bindings: {
      "t03.copy_app_target/source": "/fixture/source",
      "t03.copy_app_target/policy": "merge",
    },
    requestGeneration: 1,
  };
  const review = { reviewHandle: "review-opaque" };
  mockInvoke.mockResolvedValue(review);

  await expect(api.prepareExecutionReview(request)).resolves.toBe(review);

  expect(mockInvoke).toHaveBeenCalledOnce();
  expect(mockInvoke).toHaveBeenCalledWith("prepare_execution_review", request);
});
