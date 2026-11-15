import assert from "node:assert/strict";
import test from "node:test";
import { ensureConfigCreateQualification } from "./ensure-retroarch-config-create-qualification.mjs";

const TARGET_ID = "konkr-pocket-advance-retroarch-config-create";

test("ensure requires explicit target selection before verifying", () => {
  let calls = 0;
  assert.throws(
    () => ensureConfigCreateQualification({ verify: () => (calls += 1) }),
    /explicit configuration-create target ID/,
  );
  assert.equal(calls, 0);
});

test("ensure verifies and reuses the exact pinned target", () => {
  const result = {
    manifest: { qualificationTargetId: TARGET_ID },
    sidecarPath: "/isolated/pinned/emuchef",
  };
  let calls = 0;
  const verified = ensureConfigCreateQualification({
    targetId: TARGET_ID,
    env: { PATH: "/qualification/bin" },
    verify(options) {
      calls += 1;
      assert.equal(options.env.EMUCHEF_DEVICE_QUALIFICATION, "1");
      assert.equal(options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION, "1");
      assert.equal(options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID, TARGET_ID);
      assert.equal(options.env.PATH, "/qualification/bin");
      return result;
    },
  });
  assert.equal(calls, 1);
  assert.equal(verified, result);
});

test("ensure refuses a verifier result pinned to another target", () => {
  assert.throws(
    () =>
      ensureConfigCreateQualification({
        targetId: TARGET_ID,
        verify: () => ({
          manifest: { qualificationTargetId: "another-target" },
        }),
      }),
    /different configuration-create target/,
  );
});
