import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  qualificationStateMarkerPath,
  sidecarBuildDisposition,
} from "./sidecar-build-contract.mjs";

const APP_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const REPO_ROOT = path.resolve(APP_ROOT, "../..");

test("ordinary Tauri sidecar hooks refuse to replace or package a pinned qualification build", () => {
  const ordinaryEnvironment = {};

  assert.equal(
    sidecarBuildDisposition("debug", ordinaryEnvironment, true),
    "refuse-ordinary-build",
  );
  assert.equal(
    sidecarBuildDisposition("release", ordinaryEnvironment, true),
    "refuse-release-qualification",
  );
  assert.equal(
    sidecarBuildDisposition("debug", { EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1" }, true),
    "verify-existing",
  );
  assert.equal(
    sidecarBuildDisposition("qualification", ordinaryEnvironment, true),
    "refuse-qualification-build",
  );
  assert.equal(
    sidecarBuildDisposition("qualification", ordinaryEnvironment, false),
    "build",
  );
  assert.equal(
    sidecarBuildDisposition("debug", ordinaryEnvironment, false),
    "build",
  );
});

test("qualification state survives cargo clean because it lives outside target", () => {
  const markerPath = qualificationStateMarkerPath(REPO_ROOT);
  const cargoTarget = path.join(
    REPO_ROOT,
    "crates/emuchef-rust-backend/target",
  );

  assert.equal(path.basename(markerPath), "configuration-create-qualification-state.json");
  assert.ok(path.relative(cargoTarget, markerPath).startsWith(".."));
});
