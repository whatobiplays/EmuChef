import assert from "node:assert/strict";
import test from "node:test";

import { packagedRuntimePath } from "./smoke-packaged-runtime-network.mjs";

test("resolves the stable bundled Rust executable name", () => {
  assert.equal(
    packagedRuntimePath("/tmp/Example.app"),
    "/tmp/Example.app/Contents/MacOS/emuchef",
  );
});

