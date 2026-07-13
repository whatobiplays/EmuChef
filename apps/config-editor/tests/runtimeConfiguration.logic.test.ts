import assert from "node:assert/strict";
import test from "node:test";

import type { RuntimeConfigurationInputDto } from "../src/api/types.js";
import {
  runtimeConfigurationInvokeArgs,
  type RuntimeConfigurationRequest,
} from "../src/api/runtimeConfiguration.js";
import { runtimeControlKind } from "../src/components/runtimeConfiguration.logic.js";

function input(type: string, multiple = false): RuntimeConfigurationInputDto {
  return {
    id: "value",
    recipeId: "feature.test",
    inputId: "value",
    key: "feature.test/value",
    type,
    role: "generic",
    label: "Value",
    description: "",
    required: false,
    multiple,
    validation: { mustExist: false, allowedExtensions: [], pathKind: null, allowedPrefixes: [] },
    default: null,
    options: [],
    sensitive: false,
    advanced: false,
    metadata: {},
    value: null,
    valueSource: null,
    diagnostics: [],
  };
}

test("runtime controls derive from semantic input types", () => {
  assert.equal(runtimeControlKind(input("boolean")), "boolean");
  assert.equal(runtimeControlKind(input("enum")), "enum");
  assert.equal(runtimeControlKind(input("integer")), "integer");
  assert.equal(runtimeControlKind(input("file")), "host_path");
  assert.equal(runtimeControlKind(input("directory")), "host_path");
  assert.equal(runtimeControlKind(input("device_path")), "text");
  assert.equal(runtimeControlKind(input("object")), "json");
  assert.equal(runtimeControlKind(input("string_list")), "json");
  assert.equal(runtimeControlKind(input("string", true)), "json");
});

test("runtime requests type and forward inline canonical documents unchanged", () => {
  const request: RuntimeConfigurationRequest = {
    authoredRoot: "/tmp/authored",
    userConfiguration: {
      schema_version: 1,
      kind: "user_configuration",
      id: "inline.default",
      name: "Inline default",
      device_plan: "test.plan",
      selected_recipes: ["feature.test"],
      bindings: {
        "feature.test/value": { value: "saved" },
      },
    },
  };

  assert.deepEqual(runtimeConfigurationInvokeArgs(request), { request });
  assert.equal(
    (runtimeConfigurationInvokeArgs(request).request.userConfiguration as { device_plan: string })
      .device_plan,
    "test.plan",
  );
});

test("runtime requests continue accepting user-configuration references", () => {
  const request: RuntimeConfigurationRequest = {
    authoredRoot: "/tmp/authored",
    configurationRoot: "/tmp/configurations",
    userConfiguration: "saved.default",
  };

  assert.deepEqual(runtimeConfigurationInvokeArgs(request), { request });
});
