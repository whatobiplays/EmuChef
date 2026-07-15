import assert from "node:assert/strict";
import test from "node:test";

import { formatUpdateBytes, nextInteractionGeneration, updateNavigationBlocked } from "../src/update-policy";

test("navigation is blocked until a synchronized safe panel is active", () => {
  const safe = {
    startupReady: true,
    busy: false,
    executionKind: "idle",
    appDialogOpen: false,
    supportOpen: false,
    updatePanelOpen: true,
    updateChecking: false,
    updateOpening: false,
  };
  assert.equal(updateNavigationBlocked(safe), false);
  for (const key of ["busy", "appDialogOpen", "supportOpen", "updateChecking"] as const) {
    assert.equal(updateNavigationBlocked({ ...safe, [key]: true }), true);
  }
  assert.equal(updateNavigationBlocked({ ...safe, updateOpening: true }), false);
  assert.equal(updateNavigationBlocked({ ...safe, startupReady: false }), true);
  assert.equal(updateNavigationBlocked({ ...safe, updatePanelOpen: false }), true);
  assert.equal(updateNavigationBlocked({ ...safe, executionKind: "starting" }), true);
  assert.equal(updateNavigationBlocked({ ...safe, executionKind: "active" }), true);
});

test("interaction generations are bounded", () => {
  assert.equal(nextInteractionGeneration(0), 1);
  assert.equal(nextInteractionGeneration(999_999), 1_000_000);
  assert.equal(nextInteractionGeneration(1_000_000), null);
  assert.equal(nextInteractionGeneration(-1), null);
  assert.equal(nextInteractionGeneration(1.5), null);
});

test("DMG sizes are display-only and safely formatted", () => {
  assert.equal(formatUpdateBytes(1024 * 1024), "1.0 MiB");
  assert.equal(formatUpdateBytes(null), "Unknown size");
  assert.equal(formatUpdateBytes(-1), "Unknown size");
});
