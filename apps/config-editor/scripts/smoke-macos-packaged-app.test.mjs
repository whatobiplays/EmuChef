import assert from "node:assert/strict";
import test from "node:test";

import {
  processListContainsSidecar,
  validateSidecarResponses,
} from "./smoke-macos-packaged-app.mjs";

test("validates exact hello and ping response identities", () => {
  const responses = validateSidecarResponses([
    JSON.stringify({ id: "bundle-hello", ok: true, result: {} }),
    JSON.stringify({ id: "bundle-ping", ok: true, result: {} }),
  ]);
  assert.equal(responses.length, 2);
  assert.throws(() => validateSidecarResponses([]), /expected two/);
  assert.throws(
    () =>
      validateSidecarResponses([
        JSON.stringify({ id: "wrong", ok: true }),
        JSON.stringify({ id: "bundle-ping", ok: true }),
      ]),
    /bundle-hello/,
  );
});

test("matches only the exact bundled sidecar command", () => {
  const sidecar = "/tmp/Example.app/Contents/MacOS/emuchef";
  const processes = `10 1 ${sidecar} --sidecar\n11 1 /tmp/emuchef --sidecar\n`;
  assert.equal(processListContainsSidecar(processes, sidecar), true);
  assert.equal(processListContainsSidecar("11 1 /tmp/emuchef --sidecar", sidecar), false);
  assert.equal(processListContainsSidecar(`10 1 ${sidecar} plan`, sidecar), false);
});

