import assert from "node:assert/strict";
import test from "node:test";

import { parseBindingText } from "../src/components/userConfiguration.logic.js";

test("binding text uses JSON values when valid", () => {
  assert.deepEqual(parseBindingText('["zip","7z"]'), ["zip", "7z"]);
  assert.equal(parseBindingText("false"), false);
  assert.equal(parseBindingText("42"), 42);
});

test("binding text preserves non-JSON values as strings", () => {
  assert.equal(parseBindingText("/sdcard/ROMs"), "/sdcard/ROMs");
  assert.equal(parseBindingText("merge"), "merge");
});
