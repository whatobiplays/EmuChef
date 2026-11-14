import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { APP_ROOT } from "./run-device-qualification.mjs";
import { prepareConfigCreateQualification } from "./prepare-retroarch-config-create-qualification.mjs";

test("configuration-create build preparation requires the exact target ID before invoking host build steps", () => {
  const calls = [];
  assert.throws(
    () => prepareConfigCreateQualification({
      spawnSync(command, args) {
        calls.push({ command, args });
        return { status: 0 };
      },
    }),
    /explicit configuration-create target ID/,
  );
  assert.deepEqual(calls, []);
});

test("configuration-create preparation verifies the sidecar through Tauri Cargo check without launching the app", () => {
  const calls = [];
  const targetId = "konkr-pocket-advance-retroarch-config-create";
  const env = { PATH: "/test/bin" };

  prepareConfigCreateQualification({
    targetId,
    env,
    spawnSync(command, args, options) {
      calls.push({ command, args, options });
      return { status: 0 };
    },
    exit(status) {
      throw new Error(`unexpected exit ${status}`);
    },
  });

  assert.deepEqual(
    calls.map(({ command, args }) => [command, args]),
    [
      ["npm", ["run", "build"]],
      ["npm", ["run", "sidecar:qualification"]],
      ["node", ["scripts/verify-qualification-sidecar.mjs"]],
      ["cargo", ["check", "--manifest-path", "src-tauri/Cargo.toml", "--features", "real-execution"]],
    ],
  );
  for (const call of calls) {
    assert.equal(call.options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID, targetId);
    assert.equal(call.options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION, "1");
  }
  assert.equal(calls[0].options.env, calls[1].options.env);
  assert.equal(calls[2].options.env.EMUCHEF_DEVICE_QUALIFICATION, undefined);
  assert.notEqual(calls[2].options.env, calls[3].options.env);
  assert.equal(calls[3].options.env.EMUCHEF_DEVICE_QUALIFICATION, "1");
  assert.equal(calls.some(({ command, args }) => command === "tauri" || args?.includes("tauri")), false);
  assert.equal(calls.some(({ command, args }) => command === "cargo" && args?.[0] === "run"), false);
});

test("Tauri external and debug runtime paths share the verified qualification bytes", () => {
  const config = JSON.parse(readFileSync(path.join(APP_ROOT, "src-tauri/tauri.conf.json"), "utf8"));
  const verifier = readFileSync(path.join(APP_ROOT, "scripts/verify-qualification-sidecar.mjs"), "utf8");

  assert.deepEqual(config.bundle.externalBin, ["binaries/emuchef"]);
  assert.match(verifier, /externalBinArtifactName\(target\)/);
  assert.match(verifier, /expectedRuntimeSidecarPath/);
  assert.match(verifier, /runtimeSidecarSha256/);
});
