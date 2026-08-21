import assert from "node:assert/strict";
import test from "node:test";

import { clampSidebarWidth, parseStoredSidebarWidth } from "../src/components/resizableEditorLayout.logic.js";

function clampOptions(overrides: Partial<{
  minSidebarWidth: number;
  maxSidebarWidth: number;
  containerWidth: number;
  minDetailWidth: number;
  handleWidth: number;
}> = {}) {
  return {
    minSidebarWidth: 100,
    maxSidebarWidth: 300,
    containerWidth: 0,
    minDetailWidth: 360,
    handleWidth: 8,
    ...overrides,
  };
}

test("clampSidebarWidth keeps in-range widths unchanged", () => {
  assert.equal(clampSidebarWidth(200, clampOptions()), 200);
  assert.equal(clampSidebarWidth(100, clampOptions()), 100);
  assert.equal(clampSidebarWidth(300, clampOptions()), 300);
});

test("clampSidebarWidth clamps to the configured bounds", () => {
  assert.equal(clampSidebarWidth(50, clampOptions()), 100);
  assert.equal(clampSidebarWidth(400, clampOptions()), 300);
});

test("clampSidebarWidth falls back to the minimum for non-finite widths", () => {
  assert.equal(clampSidebarWidth(Number.NaN, clampOptions()), 100);
  assert.equal(clampSidebarWidth(Number.POSITIVE_INFINITY, clampOptions()), 100);
});

test("clampSidebarWidth applies the detail-width constraint with a measured container", () => {
  const options = clampOptions({ containerWidth: 500 });
  assert.equal(clampSidebarWidth(250, options), 132);
  assert.equal(clampSidebarWidth(120, options), 120);
});

test("clampSidebarWidth skips the detail-width constraint without a measured container", () => {
  assert.equal(clampSidebarWidth(250, clampOptions()), 250);
});

test("clampSidebarWidth floors the detail-width constraint at zero", () => {
  const options = clampOptions({ containerWidth: 360 });
  assert.equal(clampSidebarWidth(150, options), 0);
});

test("parseStoredSidebarWidth accepts finite numeric strings", () => {
  assert.equal(parseStoredSidebarWidth("320"), 320);
  assert.equal(parseStoredSidebarWidth("12.5"), 12.5);
});

test("parseStoredSidebarWidth rejects null and non-finite values", () => {
  assert.equal(parseStoredSidebarWidth(null), null);
  assert.equal(parseStoredSidebarWidth("abc"), null);
  assert.equal(parseStoredSidebarWidth("Infinity"), null);
  assert.equal(parseStoredSidebarWidth("NaN"), null);
});
