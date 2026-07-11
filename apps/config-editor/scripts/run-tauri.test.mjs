import assert from "node:assert/strict";
import test from "node:test";

import { tauriArgs } from "./run-tauri.mjs";

test("adds the development-only config to tauri dev", () => {
  assert.deepEqual(tauriArgs(["dev"]), [
    "dev",
    "--config",
    "src-tauri/tauri.dev.conf.json",
  ]);
  assert.deepEqual(tauriArgs(["dev", "--no-watch"]), [
    "dev",
    "--no-watch",
    "--config",
    "src-tauri/tauri.dev.conf.json",
  ]);
});

test("does not add development config to builds or explicit config calls", () => {
  assert.deepEqual(tauriArgs(["build"]), ["build"]);
  assert.deepEqual(tauriArgs(["dev", "--config", "custom.json"]), [
    "dev",
    "--config",
    "custom.json",
  ]);
  assert.deepEqual(tauriArgs(["dev", "--config=custom.json"]), [
    "dev",
    "--config=custom.json",
  ]);
});

