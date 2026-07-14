import assert from "node:assert/strict";
import test from "node:test";

import {
  cleanupConfirmation,
  entriesForCleanup,
  initialSupportState,
  supportReducer,
} from "../src/support";
import type { CacheEntry, CacheInventory } from "../src/types";

const entries: CacheEntry[] = [
  {
    cacheEntryHandle: "cache-one",
    category: "artifact",
    artifactLabel: "recipe/one",
    sourceKind: "https",
    integrityState: "complete",
    sizeBytes: 100,
    ageBucket: "under_1_day",
    inUse: false,
    removable: true,
  },
  {
    cacheEntryHandle: "cache-two",
    category: "artifact",
    artifactLabel: "recipe/two",
    sourceKind: "file",
    integrityState: "unindexed",
    sizeBytes: 50,
    ageBucket: "1_to_7_days",
    inUse: true,
    removable: true,
  },
];

const inventory: CacheInventory = {
  generation: "7",
  entries,
  summary: {
    entryCount: 2,
    totalSizeBytes: 150,
    inUseCount: 1,
    unmanagedCount: 0,
    unmanagedSizeBytes: 0,
  },
};

test("diagnostics export states preserve cancellation and failure outcomes", () => {
  const started = supportReducer(initialSupportState, { type: "export-started", generation: 1 });
  assert.equal(started.exporting, true);
  const cancelled = supportReducer(started, { type: "export-finished", generation: 1, outcome: "cancelled" });
  assert.equal(cancelled.exportOutcome, "cancelled");
  const failed = supportReducer(started, { type: "export-failed", generation: 1, message: "Sanitized failure" });
  assert.equal(failed.exportOutcome, "failed");
  assert.equal(failed.error, "Sanitized failure");
});

test("stale inventory responses cannot overwrite the current generation", () => {
  const requested = supportReducer(initialSupportState, { type: "inventory-requested", generation: 2 });
  const stale = supportReducer(requested, { type: "inventory-loaded", generation: 1, inventory });
  assert.equal(stale, requested);
  const loaded = supportReducer(requested, { type: "inventory-loaded", generation: 2, inventory });
  assert.equal(loaded.inventory?.generation, "7");
});

test("selective, unused, and all-removable cleanup sets are explicit", () => {
  assert.deepEqual(
    entriesForCleanup(inventory, "selected", ["cache-two"]).map((entry) => entry.cacheEntryHandle),
    ["cache-two"],
  );
  assert.deepEqual(
    entriesForCleanup(inventory, "unused", []).map((entry) => entry.cacheEntryHandle),
    ["cache-one"],
  );
  assert.deepEqual(
    entriesForCleanup(inventory, "all_removable", []).map((entry) => entry.cacheEntryHandle),
    ["cache-one", "cache-two"],
  );
});

test("destructive confirmation uses exact logical-entry count and aggregate size", () => {
  assert.deepEqual(cleanupConfirmation(entries), { entryCount: 2, totalSizeBytes: 150 });
});

test("cleanup refresh replaces handles and renders sanitized outcomes", () => {
  const loaded = supportReducer(
    { ...initialSupportState, requestGeneration: 1, inventory },
    { type: "cleanup-started", generation: 2 },
  );
  const refreshed: CacheInventory = { ...inventory, generation: "8", entries: [] };
  const finished = supportReducer(loaded, {
    type: "cleanup-finished",
    generation: 2,
    inventory: refreshed,
    outcomes: [{
      entryHandle: "cache-one",
      outcome: "removed",
      code: "cache_entry_removed",
      message: "The cache entry was removed.",
    }],
  });
  assert.equal(finished.inventory?.generation, "8");
  assert.equal(finished.outcomes[0].code, "cache_entry_removed");
  assert.deepEqual(finished.selectedHandles, []);
});

test("runtime restart invalidates cache and export presentation state", () => {
  const restarted = supportReducer(
    { ...initialSupportState, open: true, inventory, exportOutcome: "saved" },
    { type: "runtime-restarted" },
  );
  assert.equal(restarted.open, true);
  assert.equal(restarted.inventory, null);
  assert.equal(restarted.exportOutcome, "idle");
});
