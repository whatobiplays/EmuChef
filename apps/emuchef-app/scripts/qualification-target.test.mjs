import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { selectConfigCreateQualificationTarget } from "./qualification-target.mjs";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const TARGET_PATH = path.join(REPO_ROOT, "docs/qualification/retroarch-config-create/target.json");
const PROFILE_PATH = path.join(REPO_ROOT, "authored/device_profiles/konkr.pocket_advance.yaml");

test("configuration-create target preserves the owner identity and conflicting Android observations", () => {
  const target = JSON.parse(readFileSync(TARGET_PATH, "utf8"));

  assert.equal(target.qualificationTargetId, "konkr-pocket-advance-retroarch-config-create");
  assert.equal(target.selectionPolicy, "explicit_operator_selection");
  assert.equal(target.automaticProfileMatching, false);
  assert.equal(
    target.deviceSessionBinding,
    "blocked_by_existing_canonical_selection_contract",
  );
  assert.equal(target.status, "pending_physical_qualification");
  assert.equal(target.profileId, "konkr.pocket_advance");
  assert.notEqual(target.profileId, "ayaneo.pocket_air_mini");
  assert.equal(target.retailIdentity.value, "KONKR Pocket Advance");
  assert.equal(target.observations.manufacturer.value, "ARBOR");
  assert.equal(target.observations.model.value, "GT78-VN");
  assert.equal(target.observations.androidVersion.value, "12");
  assert.equal(target.observations.androidApi.value, 31);
  assert.match(target.observations.firmwareFingerprint.value, /:11\/RP1A\.200720\.011\//);
  assert.equal(target.capabilities.rootShell, "verified");
  assert.equal(target.capabilities.appDataWrite, "not_verified");
  assert.equal(Object.hasOwn(target, "serial"), false);

  const profile = readFileSync(PROFILE_PATH, "utf8");
  assert.match(profile, /selection_policy: explicit_operator_selection/);
  assert.match(profile, /automatic_profile_matching: false/);
  assert.match(profile, /android_version:\n\s+min: 12/);
  assert.match(profile, /root_shell: false/);
  assert.match(profile, /app_data_write: false/);
  const matchCriteria = profile.split("match:\n")[1].split("capability_defaults:\n")[0];
  assert.doesNotMatch(matchCriteria, /root/i);
});

test("configuration-create target resolution requires its exact operator-selected ID", () => {
  assert.throws(() => selectConfigCreateQualificationTarget(undefined), /explicit configuration-create target ID/);
  assert.throws(() => selectConfigCreateQualificationTarget("GT78-VN"), /explicit configuration-create target ID/);
  assert.throws(() => selectConfigCreateQualificationTarget("device-target-sha256:unknown"), /explicit configuration-create target ID/);
  assert.equal(
    selectConfigCreateQualificationTarget("konkr-pocket-advance-retroarch-config-create").qualificationTargetId,
    "konkr-pocket-advance-retroarch-config-create",
  );
});
