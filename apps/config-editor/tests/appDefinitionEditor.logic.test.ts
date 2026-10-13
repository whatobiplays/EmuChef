import assert from "node:assert/strict";
import test from "node:test";

import type { AppDefinitionV1Dto, AppPermissionSetDto } from "../src/api/types.js";
import {
  appArtifactFromDraft,
  appPermissionSetFromDraft,
  appTargetFromDraft,
  createAppDefinitionEditorDraft,
  isAppDefinitionEditorDraftDirty,
  isReservedAppMetadataKey,
  parseAndroidApiBound,
  parseAppMetadataValue,
  rebasePendingAppDefinitionEditorDraft,
  reconcileAppDefinitionEditorDraft,
} from "../src/components/appDefinitionEditor.logic.js";

const invalidDefinition: AppDefinitionV1Dto = {
  schema_version: 1,
  kind: "app_definition",
  id: "Invalid ID",
  name: "",
  package_id: "",
  artifacts: {
    bad: {
      kind: "apk",
      source: { strategy: "direct_url", url: "http://example.test/app.apk" },
    },
  },
};

function definitionWithPermissionSet(
  name: "baseline" | "elevated",
  permissionSet: AppPermissionSetDto,
): AppDefinitionV1Dto {
  return {
    ...invalidDefinition,
    permission_sets: {
      baseline: name === "baseline" ? permissionSet : undefined,
      elevated: name === "elevated" ? permissionSet : undefined,
    },
  };
}

test("the form projection preserves semantic errors and starts clean", () => {
  const original = structuredClone(invalidDefinition);
  const draft = createAppDefinitionEditorDraft(invalidDefinition);

  assert.equal(draft.identity.id, "Invalid ID");
  assert.equal(draft.presentation.name, "");
  assert.equal(draft.artifacts[0]?.url, "http://example.test/app.apk");
  assert.equal(isAppDefinitionEditorDraftDirty(draft, invalidDefinition), false);
  assert.deepEqual(invalidDefinition, original);
});

test("dirty state includes unsubmitted and incomplete local input", () => {
  const draft = createAppDefinitionEditorDraft(invalidDefinition);
  const edited = {
    ...draft,
    artifacts: draft.artifacts.map((item) => ({ ...item, url: "https://example.test/app.apk" })),
  };

  assert.equal(isAppDefinitionEditorDraftDirty(edited, invalidDefinition), true);
});

test("rebasing preserves only locally changed fields and refreshes accepted collection fields", () => {
  const previous: AppDefinitionV1Dto = {
    ...invalidDefinition,
    id: "demo-app",
    name: "Initial",
    package_id: "com.example.initial",
    artifacts: {
      alpha: { kind: "file", name: "Initial artifact", source: { strategy: "user_provided" } },
    },
  };
  const draft = createAppDefinitionEditorDraft(previous);
  const localDraft = {
    ...draft,
    presentation: { ...draft.presentation, name: "Pending local name" },
    artifacts: draft.artifacts.map((artifact) => ({ ...artifact, name: "Pending artifact name" })),
  };
  const next: AppDefinitionV1Dto = {
    ...previous,
    name: "Accepted name",
    package_id: "com.example.accepted",
    description: "Accepted description",
    artifacts: {
      alpha: { kind: "file", name: "Accepted artifact name", description: "Accepted artifact description", source: { strategy: "user_provided" } },
    },
  };

  const rebased = rebasePendingAppDefinitionEditorDraft(localDraft, previous, next);

  assert.equal(rebased.presentation.name, "Pending local name");
  assert.equal(rebased.presentation.description, "Accepted description");
  assert.equal(rebased.identity.packageId, "com.example.accepted");
  assert.equal(rebased.artifacts[0]?.name, "Pending artifact name");
  assert.equal(rebased.artifacts[0]?.description, "Accepted artifact description");
});

test("rebasing removed artifacts and targets keeps pending values as add rows", () => {
  const previous: AppDefinitionV1Dto = {
    ...invalidDefinition,
    id: "demo-app",
    name: "Demo App",
    package_id: "com.example.demo",
    artifacts: {
      installer: { kind: "apk", source: { strategy: "user_provided" } },
    },
    targets: {
      settings: { kind: "file", location: "app_data", path: "config/settings.json" },
    },
  };
  const draft = createAppDefinitionEditorDraft(previous);
  const localDraft = {
    ...draft,
    artifacts: draft.artifacts.map((artifact) => ({ ...artifact, name: "Pending artifact name" })),
    targets: draft.targets.map((target) => ({ ...target, path: "pending/settings.json" })),
  };
  const next: AppDefinitionV1Dto = { ...previous, artifacts: {}, targets: {} };

  const rebased = rebasePendingAppDefinitionEditorDraft(localDraft, previous, next);

  assert.equal(rebased.artifacts[0]?.originalId, undefined);
  assert.equal(rebased.artifacts[0]?.id, "installer");
  assert.equal(rebased.artifacts[0]?.name, "Pending artifact name");
  assert.equal(rebased.targets[0]?.originalId, undefined);
  assert.equal(rebased.targets[0]?.id, "settings");
  assert.equal(rebased.targets[0]?.path, "pending/settings.json");
});

test("artifact conversion supports each source strategy and rejects invalid active policies", () => {
  const draft = createAppDefinitionEditorDraft(invalidDefinition).artifacts[0];
  assert.ok(draft);
  assert.deepEqual(appArtifactFromDraft({ ...draft, sourceStrategy: "user_provided" }), {
    kind: "apk",
    name: undefined,
    description: undefined,
    source: { strategy: "user_provided" },
  });
  assert.deepEqual(appArtifactFromDraft({ ...draft, url: "https://example.test/app.apk" }).source, {
    strategy: "direct_url",
    url: "https://example.test/app.apk",
    sha256: undefined,
  });
  assert.throws(() => appArtifactFromDraft(draft), /public HTTPS URL/);

  const latestRelease = appArtifactFromDraft({
    ...draft,
    sourceStrategy: "latest_release",
    baseUrl: "https://github.com",
    repository: "owner/project",
    assetPattern: "\\.apk$",
    invertAssetPattern: true,
    prerelease: true,
  });
  assert.deepEqual(latestRelease.source, {
    strategy: "latest_release",
    provider: "github",
    base_url: "https://github.com",
    repository: "owner/project",
    asset_pattern: "\\.apk$",
    invert_asset_pattern: true,
    prerelease: true,
  });
  assert.throws(() => appArtifactFromDraft({
    ...draft,
    sourceStrategy: "latest_release",
    baseUrl: "https://github.com/path",
    repository: "owner/project",
  }), /service origin/);
});

test("latest-release regex syntax is passed through for backend validation", () => {
  const draft = createAppDefinitionEditorDraft(invalidDefinition).artifacts[0];
  assert.ok(draft);
  const pattern = "(?i)\\.apk$";

  const artifact = appArtifactFromDraft({
    ...draft,
    sourceStrategy: "latest_release",
    baseUrl: "https://github.com",
    repository: "owner/project",
    assetPattern: pattern,
  });

  assert.equal(artifact.source.strategy, "latest_release");
  if (artifact.source.strategy === "latest_release") assert.equal(artifact.source.asset_pattern, pattern);
  assert.throws(() => appArtifactFromDraft({
    ...draft,
    sourceStrategy: "latest_release",
    baseUrl: "https://github.com",
    repository: "owner/project",
    assetPattern: "   ",
  }), /non-empty regular expression/);
});

test("permission conversion maps authored API-bound keys and rejects invalid ranges", () => {
  assert.deepEqual(appPermissionSetFromDraft({
    runtime: [{ localId: "r1", permission: "android.permission.CAMERA", androidApiMin: "", androidApiMax: "34" }],
    appOps: [{ localId: "o1", op: "CAMERA", androidApiMin: "28", androidApiMax: "34" }],
  }), {
    runtime: [{ permission: "android.permission.CAMERA", android_api_min: undefined, android_api_max: 34 }],
    app_ops: [{ op: "CAMERA", mode: "allow", android_api_min: 28, android_api_max: 34 }],
  });
  assert.throws(() => parseAndroidApiBound("0", "Minimum Android API"), /positive whole number/);
  assert.throws(() => parseAndroidApiBound("2.5", "Minimum Android API"), /positive whole number/);
  assert.throws(() => appPermissionSetFromDraft({
    runtime: [{ localId: "r1", permission: "CAMERA", androidApiMin: "35", androidApiMax: "34" }],
  }), /must not exceed/);
});

for (const setName of ["baseline", "elevated"] as const) {
  test(`${setName} runtime action edits follow permission names through removal undo and redo`, () => {
    const camera = { permission: "CAMERA" };
    const location = { permission: "LOCATION" };
    const beforeRemoval = definitionWithPermissionSet(setName, { runtime: [camera, location] });
    const afterRemoval = definitionWithPermissionSet(setName, { runtime: [location] });
    const acceptedDraft = createAppDefinitionEditorDraft(afterRemoval);
    const currentSet = acceptedDraft.permissionSets[setName];
    assert.ok(currentSet?.runtime);
    const localDraft = {
      ...acceptedDraft,
      permissionSets: {
        ...acceptedDraft.permissionSets,
        [setName]: {
          ...currentSet,
          runtime: currentSet.runtime.map((row) => ({ ...row, androidApiMin: "29" })),
        },
      },
    };

    const afterUndo = rebasePendingAppDefinitionEditorDraft(localDraft, afterRemoval, beforeRemoval);
    const undoneRows = afterUndo.permissionSets[setName]?.runtime ?? [];
    assert.deepEqual(undoneRows.map((row) => row.permission), ["CAMERA", "LOCATION"]);
    assert.equal(undoneRows[0]?.androidApiMin, undefined);
    assert.equal(undoneRows[1]?.androidApiMin, "29");

    const afterRedo = rebasePendingAppDefinitionEditorDraft(afterUndo, beforeRemoval, afterRemoval);
    const redoneRows = afterRedo.permissionSets[setName]?.runtime ?? [];
    assert.deepEqual(redoneRows.map((row) => row.permission), ["LOCATION"]);
    assert.equal(redoneRows[0]?.androidApiMin, "29");
  });

  test(`${setName} app-op edits follow action names through reorder undo and redo`, () => {
    const camera = { op: "CAMERA", mode: "allow" as const };
    const location = { op: "LOCATION", mode: "allow" as const };
    const beforeReorder = definitionWithPermissionSet(setName, { app_ops: [camera, location] });
    const afterReorder = definitionWithPermissionSet(setName, { app_ops: [location, camera] });
    const acceptedDraft = createAppDefinitionEditorDraft(afterReorder);
    const currentSet = acceptedDraft.permissionSets[setName];
    assert.ok(currentSet?.appOps);
    const localDraft = {
      ...acceptedDraft,
      permissionSets: {
        ...acceptedDraft.permissionSets,
        [setName]: {
          ...currentSet,
          appOps: currentSet.appOps.map((row) => row.op === "CAMERA" ? { ...row, androidApiMin: "28" } : row),
        },
      },
    };

    const afterUndo = rebasePendingAppDefinitionEditorDraft(localDraft, afterReorder, beforeReorder);
    const undoneRows = afterUndo.permissionSets[setName]?.appOps ?? [];
    assert.deepEqual(undoneRows.map((row) => row.op), ["CAMERA", "LOCATION"]);
    assert.equal(undoneRows[0]?.androidApiMin, "28");
    assert.equal(undoneRows[1]?.androidApiMin, undefined);

    const afterRedo = rebasePendingAppDefinitionEditorDraft(afterUndo, beforeReorder, afterReorder);
    const redoneRows = afterRedo.permissionSets[setName]?.appOps ?? [];
    assert.deepEqual(redoneRows.map((row) => row.op), ["LOCATION", "CAMERA"]);
    assert.equal(redoneRows[1]?.androidApiMin, "28");
  });
}

test("duplicate invalid permission action names keep pending rows from being matched ambiguously", () => {
  const invalidDuplicates = definitionWithPermissionSet("baseline", {
    runtime: [
      { permission: "CAMERA", android_api_min: 21 },
      { permission: "CAMERA", android_api_min: 30 },
      { permission: "LOCATION" },
    ],
  });
  const reorderedDuplicates = definitionWithPermissionSet("baseline", {
    runtime: [
      { permission: "LOCATION" },
      { permission: "CAMERA", android_api_min: 30 },
      { permission: "CAMERA", android_api_min: 21 },
    ],
  });
  const acceptedDraft = createAppDefinitionEditorDraft(invalidDuplicates);
  const currentSet = acceptedDraft.permissionSets.baseline;
  assert.ok(currentSet?.runtime);
  const localDraft = {
    ...acceptedDraft,
    permissionSets: {
      ...acceptedDraft.permissionSets,
      baseline: {
        ...currentSet,
        runtime: currentSet.runtime.map((row, index) =>
          index === 0 ? { ...row, androidApiMax: "25" } : row,
        ),
      },
    },
  };

  const rebased = rebasePendingAppDefinitionEditorDraft(localDraft, invalidDuplicates, reorderedDuplicates);
  const rows = rebased.permissionSets.baseline?.runtime ?? [];
  assert.deepEqual(rows.map((row) => row.permission), ["CAMERA", "CAMERA", "LOCATION"]);
  assert.equal(rows[0]?.androidApiMin, "21");
  assert.equal(rows[0]?.androidApiMax, "25");
  assert.equal(rows[1]?.androidApiMin, "30");
});

test("duplicate invalid app-op names keep pending rows from being matched ambiguously", () => {
  const invalidDuplicates = definitionWithPermissionSet("elevated", {
    app_ops: [
      { op: "CAMERA", mode: "allow", android_api_min: 21 },
      { op: "CAMERA", mode: "allow", android_api_min: 30 },
      { op: "LOCATION", mode: "allow" },
    ],
  });
  const reorderedDuplicates = definitionWithPermissionSet("elevated", {
    app_ops: [
      { op: "LOCATION", mode: "allow" },
      { op: "CAMERA", mode: "allow", android_api_min: 30 },
      { op: "CAMERA", mode: "allow", android_api_min: 21 },
    ],
  });
  const acceptedDraft = createAppDefinitionEditorDraft(invalidDuplicates);
  const currentSet = acceptedDraft.permissionSets.elevated;
  assert.ok(currentSet?.appOps);
  const localDraft = {
    ...acceptedDraft,
    permissionSets: {
      ...acceptedDraft.permissionSets,
      elevated: {
        ...currentSet,
        appOps: currentSet.appOps.map((row, index) =>
          index === 0 ? { ...row, androidApiMax: "25" } : row,
        ),
      },
    },
  };

  const rebased = rebasePendingAppDefinitionEditorDraft(localDraft, invalidDuplicates, reorderedDuplicates);
  const rows = rebased.permissionSets.elevated?.appOps ?? [];
  assert.deepEqual(rows.map((row) => row.op), ["CAMERA", "CAMERA", "LOCATION"]);
  assert.equal(rows[0]?.androidApiMin, "21");
  assert.equal(rows[0]?.androidApiMax, "25");
  assert.equal(rows[1]?.androidApiMin, "30");
});

test("a unique pending runtime permission reattaches to the accepted action across undo and redo", () => {
  const beforeRemoval = definitionWithPermissionSet("baseline", {
    runtime: [{ permission: "CAMERA", android_api_min: 22 }, { permission: "LOCATION" }],
  });
  const afterRemoval = definitionWithPermissionSet("baseline", { runtime: [{ permission: "LOCATION" }] });
  const acceptedDraft = createAppDefinitionEditorDraft(afterRemoval);
  const currentSet = acceptedDraft.permissionSets.baseline;
  assert.ok(currentSet?.runtime);
  const localDraft = {
    ...acceptedDraft,
    permissionSets: {
      ...acceptedDraft.permissionSets,
      baseline: {
        ...currentSet,
        runtime: [...currentSet.runtime, {
          localId: "pending-camera",
          permission: "CAMERA",
          androidApiMin: "30",
          androidApiMax: "34",
        }],
      },
    },
  };

  const afterUndo = rebasePendingAppDefinitionEditorDraft(localDraft, afterRemoval, beforeRemoval);
  const cameraAfterUndo = (afterUndo.permissionSets.baseline?.runtime ?? [])
    .filter((row) => row.permission === "CAMERA");
  assert.equal(cameraAfterUndo.length, 1);
  assert.equal(cameraAfterUndo[0]?.originalPermission, "CAMERA");
  assert.equal(cameraAfterUndo[0]?.androidApiMin, "30");
  assert.equal(cameraAfterUndo[0]?.androidApiMax, "34");

  const afterRedo = rebasePendingAppDefinitionEditorDraft(afterUndo, beforeRemoval, afterRemoval);
  const cameraAfterRedo = (afterRedo.permissionSets.baseline?.runtime ?? [])
    .filter((row) => row.permission === "CAMERA");
  assert.equal(cameraAfterRedo.length, 1);
  assert.equal(cameraAfterRedo[0]?.originalPermission, undefined);
  assert.equal(cameraAfterRedo[0]?.androidApiMin, "30");
  assert.equal(cameraAfterRedo[0]?.androidApiMax, "34");

  const afterUndoAgain = rebasePendingAppDefinitionEditorDraft(afterRedo, afterRemoval, beforeRemoval);
  const cameraAfterUndoAgain = (afterUndoAgain.permissionSets.baseline?.runtime ?? [])
    .filter((row) => row.permission === "CAMERA");
  assert.equal(cameraAfterUndoAgain.length, 1);
  assert.equal(cameraAfterUndoAgain[0]?.originalPermission, "CAMERA");
  assert.equal(cameraAfterUndoAgain[0]?.androidApiMin, "30");
});

test("a unique pending app-op reattaches to the accepted action across undo and redo", () => {
  const beforeRemoval = definitionWithPermissionSet("elevated", {
    app_ops: [{ op: "CAMERA", mode: "allow", android_api_min: 22 }, { op: "LOCATION", mode: "allow" }],
  });
  const afterRemoval = definitionWithPermissionSet("elevated", { app_ops: [{ op: "LOCATION", mode: "allow" }] });
  const acceptedDraft = createAppDefinitionEditorDraft(afterRemoval);
  const currentSet = acceptedDraft.permissionSets.elevated;
  assert.ok(currentSet?.appOps);
  const localDraft = {
    ...acceptedDraft,
    permissionSets: {
      ...acceptedDraft.permissionSets,
      elevated: {
        ...currentSet,
        appOps: [...currentSet.appOps, {
          localId: "pending-camera-op",
          op: "CAMERA",
          androidApiMin: "30",
          androidApiMax: "34",
        }],
      },
    },
  };

  const afterUndo = rebasePendingAppDefinitionEditorDraft(localDraft, afterRemoval, beforeRemoval);
  const cameraAfterUndo = (afterUndo.permissionSets.elevated?.appOps ?? [])
    .filter((row) => row.op === "CAMERA");
  assert.equal(cameraAfterUndo.length, 1);
  assert.equal(cameraAfterUndo[0]?.originalOp, "CAMERA");
  assert.equal(cameraAfterUndo[0]?.androidApiMin, "30");
  assert.equal(cameraAfterUndo[0]?.androidApiMax, "34");

  const afterRedo = rebasePendingAppDefinitionEditorDraft(afterUndo, beforeRemoval, afterRemoval);
  const cameraAfterRedo = (afterRedo.permissionSets.elevated?.appOps ?? [])
    .filter((row) => row.op === "CAMERA");
  assert.equal(cameraAfterRedo.length, 1);
  assert.equal(cameraAfterRedo[0]?.originalOp, undefined);
  assert.equal(cameraAfterRedo[0]?.androidApiMin, "30");
  assert.equal(cameraAfterRedo[0]?.androidApiMax, "34");

  const afterUndoAgain = rebasePendingAppDefinitionEditorDraft(afterRedo, afterRemoval, beforeRemoval);
  const cameraAfterUndoAgain = (afterUndoAgain.permissionSets.elevated?.appOps ?? [])
    .filter((row) => row.op === "CAMERA");
  assert.equal(cameraAfterUndoAgain.length, 1);
  assert.equal(cameraAfterUndoAgain[0]?.originalOp, "CAMERA");
  assert.equal(cameraAfterUndoAgain[0]?.androidApiMin, "30");
});

test("ambiguous pending runtime permission additions remain separate from a restored action", () => {
  const beforeRemoval = definitionWithPermissionSet("baseline", {
    runtime: [{ permission: "CAMERA" }, { permission: "LOCATION" }],
  });
  const afterRemoval = definitionWithPermissionSet("baseline", { runtime: [{ permission: "LOCATION" }] });
  const acceptedDraft = createAppDefinitionEditorDraft(afterRemoval);
  const currentSet = acceptedDraft.permissionSets.baseline;
  assert.ok(currentSet?.runtime);
  const localDraft = {
    ...acceptedDraft,
    permissionSets: {
      ...acceptedDraft.permissionSets,
      baseline: {
        ...currentSet,
        runtime: [...currentSet.runtime,
          { localId: "pending-camera-one", permission: "CAMERA", androidApiMin: "25" },
          { localId: "pending-camera-two", permission: "CAMERA", androidApiMin: "30" },
        ],
      },
    },
  };

  const rebased = rebasePendingAppDefinitionEditorDraft(localDraft, afterRemoval, beforeRemoval);
  const cameraRows = (rebased.permissionSets.baseline?.runtime ?? [])
    .filter((row) => row.permission === "CAMERA");
  assert.equal(cameraRows.length, 3);
  assert.equal(cameraRows.filter((row) => row.originalPermission === "CAMERA").length, 1);
  assert.deepEqual(cameraRows.filter((row) => row.originalPermission === undefined).map((row) => row.androidApiMin), ["25", "30"]);
});

test("ambiguous pending app-op additions remain separate from a restored action", () => {
  const beforeRemoval = definitionWithPermissionSet("elevated", {
    app_ops: [{ op: "CAMERA", mode: "allow" }, { op: "LOCATION", mode: "allow" }],
  });
  const afterRemoval = definitionWithPermissionSet("elevated", { app_ops: [{ op: "LOCATION", mode: "allow" }] });
  const acceptedDraft = createAppDefinitionEditorDraft(afterRemoval);
  const currentSet = acceptedDraft.permissionSets.elevated;
  assert.ok(currentSet?.appOps);
  const localDraft = {
    ...acceptedDraft,
    permissionSets: {
      ...acceptedDraft.permissionSets,
      elevated: {
        ...currentSet,
        appOps: [...currentSet.appOps,
          { localId: "pending-camera-op-one", op: "CAMERA", androidApiMin: "25" },
          { localId: "pending-camera-op-two", op: "CAMERA", androidApiMin: "30" },
        ],
      },
    },
  };

  const rebased = rebasePendingAppDefinitionEditorDraft(localDraft, afterRemoval, beforeRemoval);
  const cameraRows = (rebased.permissionSets.elevated?.appOps ?? [])
    .filter((row) => row.op === "CAMERA");
  assert.equal(cameraRows.length, 3);
  assert.equal(cameraRows.filter((row) => row.originalOp === "CAMERA").length, 1);
  assert.deepEqual(cameraRows.filter((row) => row.originalOp === undefined).map((row) => row.androidApiMin), ["25", "30"]);
});

test("target conversion validates the identifier and required path", () => {
  const target = { localId: "t1", id: "config", kind: "file" as const, location: "app_data" as const, path: "config/app.json" };
  assert.deepEqual(appTargetFromDraft(target), {
    kind: "file",
    location: "app_data",
    path: "config/app.json",
  });
  assert.throws(() => appTargetFromDraft({ ...target, id: "Invalid ID" }), /Target ID/);
  assert.throws(() => appTargetFromDraft({ ...target, path: "  " }), /path must not be empty/);
});

test("metadata parsing keeps JSON scalars and reserved schema names are exact", () => {
  assert.deepEqual(parseAppMetadataValue('{"enabled":true}'), { enabled: true });
  assert.equal(parseAppMetadataValue("false"), false);
  assert.throws(() => parseAppMetadataValue("{broken"), /valid JSON/);
  assert.equal(isReservedAppMetadataKey("id"), true);
  assert.equal(isReservedAppMetadataKey("release_channel"), false);
});

test("reconciling one presentation command preserves unrelated invalid artifact input", () => {
  const draft = createAppDefinitionEditorDraft(invalidDefinition);
  const next = reconcileAppDefinitionEditorDraft(
    draft,
    { ...invalidDefinition, name: "Corrected" },
    { type: "SetName", name: "Corrected" },
  );

  assert.equal(next.presentation.name, "Corrected");
  assert.equal(next.artifacts[0]?.url, "http://example.test/app.apk");
});

test("reconciling metadata preserves falsey JSON values and removal drops only the selected row", () => {
  const definition = { ...invalidDefinition, metadata: { enabled: false, retries: 0 } };
  const draft = createAppDefinitionEditorDraft(definition);
  const accepted = { ...definition, metadata: { enabled: false, retries: 0, notes: null } };
  const withEntry = reconcileAppDefinitionEditorDraft(draft, accepted, {
    type: "SetMetadataEntry",
    key: "notes",
    value: null,
  });
  assert.equal(withEntry.metadata.find((row) => row.key === "notes")?.valueJson, "null");

  const removed = reconcileAppDefinitionEditorDraft(withEntry, {
    ...accepted,
    metadata: { enabled: false, retries: 0 },
  }, { type: "RemoveMetadataEntry", key: "notes" });
  assert.deepEqual(removed.metadata.map((row) => row.key), ["enabled", "retries"]);
});

test("metadata reconciliation updates the submitted local row when keys collide", () => {
  const definition = { ...invalidDefinition, metadata: { notes: "accepted value" } };
  const draft = {
    ...createAppDefinitionEditorDraft(definition),
    metadata: [
      { localId: "existing-row", originalKey: "notes", key: "notes", valueJson: '"other pending JSON"' },
      { localId: "submitted-row", key: "notes", valueJson: '"submitted JSON"' },
    ],
  };

  const reconciled = reconcileAppDefinitionEditorDraft(
    draft,
    definition,
    { type: "SetMetadataEntry", key: "notes", value: "accepted value" },
    "submitted-row",
  );

  assert.equal(reconciled.metadata[0]?.valueJson, '"other pending JSON"');
  assert.equal(reconciled.metadata[1]?.valueJson, '"accepted value"');
  assert.equal(reconciled.metadata[1]?.localId, "submitted-row");
});

test("artifact and target undo/redo ignores regenerated UI IDs for unchanged rows", () => {
  const previous: AppDefinitionV1Dto = {
    ...invalidDefinition,
    artifacts: { installer: { kind: "apk", source: { strategy: "user_provided" } } },
    targets: { settings: { kind: "file", location: "app_data", path: "config/settings.json" } },
  };
  const acceptedDraft = createAppDefinitionEditorDraft(previous);
  const localDraft = {
    ...acceptedDraft,
    artifacts: acceptedDraft.artifacts.map((row) => ({ ...row, localId: "regenerated-artifact" })),
    targets: acceptedDraft.targets.map((row) => ({ ...row, localId: "regenerated-target" })),
  };
  const undone: AppDefinitionV1Dto = { ...previous, artifacts: {}, targets: {} };

  const afterUndo = rebasePendingAppDefinitionEditorDraft(localDraft, previous, undone);
  assert.deepEqual(afterUndo.artifacts, []);
  assert.deepEqual(afterUndo.targets, []);

  const afterRedo = rebasePendingAppDefinitionEditorDraft(afterUndo, undone, previous);
  assert.equal(afterRedo.artifacts.length, 1);
  assert.equal(afterRedo.artifacts[0]?.id, "installer");
  assert.equal(afterRedo.targets.length, 1);
  assert.equal(afterRedo.targets[0]?.id, "settings");
});

test("artifact and target undo/redo keeps edited pending rows without duplicate IDs", () => {
  const previous: AppDefinitionV1Dto = {
    ...invalidDefinition,
    artifacts: { installer: { kind: "apk", source: { strategy: "user_provided" } } },
    targets: { settings: { kind: "file", location: "app_data", path: "config/settings.json" } },
  };
  const acceptedDraft = createAppDefinitionEditorDraft(previous);
  const localDraft = {
    ...acceptedDraft,
    artifacts: acceptedDraft.artifacts.map((row) => ({ ...row, localId: "pending-artifact", name: "Pending artifact" })),
    targets: acceptedDraft.targets.map((row) => ({ ...row, localId: "pending-target", path: "pending/settings.json" })),
  };
  const undone: AppDefinitionV1Dto = { ...previous, artifacts: {}, targets: {} };

  const afterUndo = rebasePendingAppDefinitionEditorDraft(localDraft, previous, undone);
  assert.equal(afterUndo.artifacts[0]?.originalId, undefined);
  assert.equal(afterUndo.artifacts[0]?.name, "Pending artifact");
  assert.equal(afterUndo.targets[0]?.originalId, undefined);
  assert.equal(afterUndo.targets[0]?.path, "pending/settings.json");

  const afterRedo = rebasePendingAppDefinitionEditorDraft(afterUndo, undone, previous);
  assert.equal(afterRedo.artifacts.length, 1);
  assert.equal(afterRedo.artifacts[0]?.originalId, "installer");
  assert.equal(afterRedo.artifacts[0]?.name, "Pending artifact");
  assert.equal(afterRedo.targets.length, 1);
  assert.equal(afterRedo.targets[0]?.originalId, "settings");
  assert.equal(afterRedo.targets[0]?.path, "pending/settings.json");
});

test("metadata undo/redo matches accepted keys when row local IDs change", () => {
  const previous: AppDefinitionV1Dto = { ...invalidDefinition, metadata: { notes: "accepted value" } };
  const acceptedDraft = createAppDefinitionEditorDraft(previous);
  const localDraft = {
    ...acceptedDraft,
    metadata: acceptedDraft.metadata.map((row) => ({ ...row, localId: "regenerated-metadata" })),
  };
  const undone: AppDefinitionV1Dto = { ...previous, metadata: {} };

  const afterUndo = rebasePendingAppDefinitionEditorDraft(localDraft, previous, undone);
  assert.deepEqual(afterUndo.metadata, []);

  const afterRedo = rebasePendingAppDefinitionEditorDraft(afterUndo, undone, previous);
  assert.equal(afterRedo.metadata.length, 1);
  assert.equal(afterRedo.metadata[0]?.key, "notes");
  assert.equal(afterRedo.metadata[0]?.originalKey, "notes");
});

test("metadata undo/redo reattaches an edited pending key without duplicating the row", () => {
  const previous: AppDefinitionV1Dto = { ...invalidDefinition, metadata: { notes: "accepted value" } };
  const acceptedDraft = createAppDefinitionEditorDraft(previous);
  const localDraft = {
    ...acceptedDraft,
    metadata: acceptedDraft.metadata.map((row) => ({
      ...row,
      localId: "pending-metadata",
      valueJson: '"pending JSON"',
    })),
  };
  const undone: AppDefinitionV1Dto = { ...previous, metadata: {} };

  const afterUndo = rebasePendingAppDefinitionEditorDraft(localDraft, previous, undone);
  assert.equal(afterUndo.metadata.length, 1);
  assert.equal(afterUndo.metadata[0]?.originalKey, undefined);
  assert.equal(afterUndo.metadata[0]?.valueJson, '"pending JSON"');

  const afterRedo = rebasePendingAppDefinitionEditorDraft(afterUndo, undone, previous);
  assert.equal(afterRedo.metadata.length, 1);
  assert.equal(afterRedo.metadata[0]?.originalKey, "notes");
  assert.equal(afterRedo.metadata[0]?.valueJson, '"pending JSON"');
});

test("collection commands reconcile accepted removal and ordering while preserving authored row IDs", () => {
  const definition: AppDefinitionV1Dto = {
    ...invalidDefinition,
    artifacts: {
      alpha: { kind: "file", source: { strategy: "user_provided" } },
      beta: { kind: "apk", source: { strategy: "user_provided" } },
    },
  };
  const draft = createAppDefinitionEditorDraft(definition);
  const reordered = reconcileAppDefinitionEditorDraft({
    ...draft,
    artifacts: [...draft.artifacts].reverse(),
  }, definition, { type: "ReorderArtifact", artifactId: "beta", toIndex: 0 });
  assert.deepEqual(reordered.artifacts.map((row) => row.originalId), ["alpha", "beta"]);

  const beta = definition.artifacts?.beta;
  assert.ok(beta);
  const removed = reconcileAppDefinitionEditorDraft(reordered, {
    ...definition,
    artifacts: { beta },
  }, { type: "RemoveArtifact", artifactId: "alpha" });
  assert.deepEqual(removed.artifacts.map((row) => row.originalId), ["beta"]);
});
