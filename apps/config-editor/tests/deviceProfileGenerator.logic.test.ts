import assert from "node:assert/strict";
import test from "node:test";

import type {
  DeviceProfileDraftResult,
  DeviceProfileV1Dto,
  SafeDetectedDeviceFactsDto,
} from "../src/api/types.js";
import {
  formToProfile,
  initialDeviceProfileGeneratorState,
  parseMetadataObject,
  profileToForm,
  reduceDeviceProfileGenerator,
} from "../src/components/deviceProfileGenerator.logic.js";

const profile: DeviceProfileV1Dto = {
  schema_version: 1,
  kind: "device_profile",
  id: "ayaneo.pocket_s_mini",
  name: "AYANEO Pocket S Mini",
  match: {
    manufacturer_contains: ["AYANEO"],
    brand_contains: ["AYANEO"],
    model_patterns: ["^Pocket S Mini$"],
    android_version: { min: 13 },
  },
  capability_defaults: {
    adb_available: true,
    apk_install: true,
    shared_storage_write: true,
    app_launch: true,
    shell_command: true,
    package_remove_for_user: false,
    root_shell: false,
    app_data_write: false,
  },
  device_tags: [],
  metadata: {},
};

const draft: DeviceProfileDraftResult = {
  profile,
  canonicalYaml: "schema_version: 1\nkind: device_profile\n",
  evidence: [],
  diagnostics: [],
  destination: {
    fileName: "ayaneo.pocket_s_mini.yaml",
    relativePath: "device_profiles/ayaneo.pocket_s_mini.yaml",
  },
};

const facts: SafeDetectedDeviceFactsDto = {
  manufacturer: "AYANEO",
  brand: "AYANEO",
  model: "Pocket S Mini",
  product: "pocket_s_mini",
  device: "pocket_s_mini",
  board: "kalama",
  hardware: "qcom",
  abis: ["arm64-v8a"],
  androidVersion: 13,
  androidApiLevel: 33,
};

test("wizard transitions through probe, edit, review, and successful save", () => {
  let state = reduceDeviceProfileGenerator(initialDeviceProfileGeneratorState, {
    type: "sessionStarted",
    sessionHandle: "generator-1",
  });
  state = reduceDeviceProfileGenerator(state, { type: "devicesLoading" });
  state = reduceDeviceProfileGenerator(state, {
    type: "devicesLoaded",
    devices: [{ deviceHandle: "device-1", state: "device", model: "Pocket S Mini" }],
  });
  state = reduceDeviceProfileGenerator(state, {
    type: "deviceSelected",
    deviceHandle: "device-1",
  });
  state = reduceDeviceProfileGenerator(state, { type: "probeLoaded", facts, draft });
  assert.equal(state.phase, "facts");
  assert.equal(state.form?.id, profile.id);
  state = reduceDeviceProfileGenerator(state, { type: "editStarted" });
  state = reduceDeviceProfileGenerator(state, {
    type: "rootSelected",
    rootHandle: "root-1",
    rootLabel: "Selected authored root",
  });
  state = reduceDeviceProfileGenerator(state, {
    type: "reviewLoaded",
    draft,
    collisions: { collisions: [], blocking: false },
  });
  assert.equal(state.phase, "review");
  state = reduceDeviceProfileGenerator(state, { type: "saveStarted" });
  state = reduceDeviceProfileGenerator(state, {
    type: "saveSucceeded",
    saved: {
      fileName: "ayaneo.pocket_s_mini.yaml",
      displayPath: "device_profiles/ayaneo.pocket_s_mini.yaml",
    },
  });
  assert.equal(state.phase, "saved");
  assert.equal(state.saved?.displayPath, "device_profiles/ayaneo.pocket_s_mini.yaml");
});

test("validation failure remains editable and cancel or restart removes session state", () => {
  const started = reduceDeviceProfileGenerator(initialDeviceProfileGeneratorState, {
    type: "sessionStarted",
    sessionHandle: "generator-1",
  });
  const invalid = reduceDeviceProfileGenerator(started, {
    type: "draftInvalid",
    draft: { ...draft, canonicalYaml: null },
    message: "Resolve validation errors.",
  });
  assert.equal(invalid.phase, "edit");
  assert.equal(invalid.error, "Resolve validation errors.");
  const cancelled = reduceDeviceProfileGenerator(invalid, { type: "cancelled" });
  assert.equal(cancelled.sessionHandle, null);
  const restarted = reduceDeviceProfileGenerator(started, { type: "restartInvalidated" });
  assert.equal(restarted.sessionHandle, null);
});

test("form conversion fixes schema identity and validates Android minimum", () => {
  const form = profileToForm(profile);
  const converted = formToProfile({
    ...form,
    id: "author.edited",
    manufacturers: "AYANEO\n\n ARBOR ",
    metadata: '{"vendor":"AYANEO"}',
  });
  assert.equal(converted.ok, true);
  if (converted.ok) {
    assert.equal(converted.profile.schema_version, 1);
    assert.equal(converted.profile.kind, "device_profile");
    assert.deepEqual(converted.profile.match.manufacturer_contains, ["AYANEO", "ARBOR"]);
    assert.deepEqual(converted.profile.metadata, { vendor: "AYANEO" });
  }
  assert.deepEqual(formToProfile({ ...form, androidMinimum: "13.5" }), {
    ok: false,
    message: "Android minimum must be a whole number.",
  });
});

test("metadata accepts only strict JSON objects without duplicate keys", () => {
  assert.deepEqual(parseMetadataObject('{"vendor":"AYANEO","nested":{"value":1}}'), {
    ok: true,
    value: { vendor: "AYANEO", nested: { value: 1 } },
  });
  for (const invalid of [
    "[]",
    "null",
    '"scalar"',
    "vendor: AYANEO",
    '{"vendor":"first","vendor":"second"}',
    '{"nested":{"value":1,"value":2}}',
  ]) {
    assert.equal(parseMetadataObject(invalid).ok, false, invalid);
  }
});
