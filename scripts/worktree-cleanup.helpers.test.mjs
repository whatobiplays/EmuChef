import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  assertExists,
  assertRemoved,
  captureCleanupOutput,
  collectInventory,
  createFixture,
  forceTrack,
  invokeCleanupMain,
  isProtectedReleaseArtifact,
  printInventory,
  printRemovalSummary,
  removeEligible,
  validRelativePath,
  validSidecarFilename,
  validSidecarMetadataFilename,
  writeFile,
} from "./worktree-cleanup-test-fixtures.mjs";

const CARGO_TARGET_ROOT = "crates/emuchef-rust-backend/target";
const APP_ROOT = "apps/emuchef-app";
const CONFIG_APP_ROOT = "apps/config-editor";
test("fixture teardown removes read-only nested directories without following symlinks", async (t) => {
  const externalRoot = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-cleanup-teardown-external-"));
  const externalTarget = writeFile(externalRoot, "outside/keep.txt", "external fixture data\n");
  t.after(() => fs.rmSync(externalRoot, { recursive: true, force: true }));

  let sandbox;
  let root;
  await t.test("read-only fixture directories", (fixtureTest) => {
    ({ sandbox, root } = createFixture(fixtureTest));
    const nestedDirectory = path.join(root, "readonly/first/second");
    writeFile(root, "readonly/first/second/keep.txt");
    fs.symlinkSync(externalTarget, path.join(nestedDirectory, "external-link"));
    fs.chmodSync(path.join(root, "readonly"), 0o555);
    fs.chmodSync(path.join(root, "readonly/first"), 0o555);
    fs.chmodSync(nestedDirectory, 0o555);
  });

  assert.equal(fs.existsSync(sandbox), false);
  assert.equal(fs.existsSync(root), false);
  assert.equal(fs.readFileSync(externalTarget, "utf8"), "external fixture data\n");
});

test("direct planning, classification, and report helpers cover cleanup inventory paths", (t) => {
  const { root } = createFixture(t);
  const cargoPath = "crates/emuchef-rust-backend/target/debug/deps/libdirect-0123456789abcdef.rlib";
  const generatedPath = "apps/emuchef-app/src-tauri/gen/schemas/desktop-schema.json";
  const dependencyPath = "apps/emuchef-app/node_modules/pkg/index.js";
  const distPath = "apps/emuchef-app/dist/index.html";
  const sidecarPath = "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin";
  const metadataPath = `${sidecarPath}.metadata.json`;
  const cargoContents = "direct cargo output\n";
  const generatedContents = "generated schema\n";
  writeFile(root, cargoPath, cargoContents);
  writeFile(root, generatedPath, generatedContents);
  writeFile(root, dependencyPath, "dependency output\n");
  writeFile(root, distPath, "frontend output\n");
  writeFile(root, sidecarPath, "sidecar output\n");
  writeFile(root, metadataPath, "sidecar metadata\n");
  writeFile(root, "apps/emuchef-app/dist/unknown.bin", "preserve unknown output\n");

  assert.equal(validRelativePath("apps/emuchef-app/src-tauri/gen/schemas/desktop-schema.json"), true);
  assert.equal(validRelativePath("../outside"), false);
  assert.equal(validRelativePath("/absolute/path"), false);
  assert.equal(validRelativePath("apps\\outside"), false);
  assert.equal(isProtectedReleaseArtifact("apps/app/target/release/bundle/App.app"), true);
  assert.equal(isProtectedReleaseArtifact("apps/app/target/debug/deps/libapp.rlib"), false);
  assert.equal(validSidecarFilename("emuchef-aarch64-apple-darwin"), true);
  assert.equal(validSidecarFilename("emuchef-aarch64-apple-darwin.exe"), false);
  assert.equal(validSidecarFilename("emuchef-x86_64-pc-windows-msvc"), false);
  assert.equal(validSidecarFilename("emuchef-x86_64-pc-windows-msvc.exe"), true);
  for (const targetTriple of ["x86_64-apple-darwin", "x86_64-unknown-linux-gnu"]) {
    assert.equal(validSidecarFilename(`emuchef-${targetTriple}`), true);
    assert.equal(validSidecarMetadataFilename(`emuchef-${targetTriple}.metadata.json`), true);
  }
  for (const nearMatch of [
    "emuchef-aarch64-apple-darwin.backup",
    "emuchef-aarch64-apple-darwin.old",
    "emuchef-aarch64-apple-darwin-extra",
  ]) {
    assert.equal(validSidecarFilename(nearMatch), false, nearMatch);
  }
  assert.equal(validSidecarFilename("emuchef-aarch64-apple-darwin.metadata.json"), false);
  assert.equal(validSidecarMetadataFilename("emuchef-aarch64-apple-darwin.metadata.json"), true);
  assert.equal(
    validSidecarMetadataFilename("emuchef-aarch64-apple-darwin.exe.metadata.json"),
    false,
  );
  assert.equal(
    validSidecarMetadataFilename("emuchef-x86_64-pc-windows-msvc.metadata.json"),
    false,
  );
  assert.equal(
    validSidecarMetadataFilename("emuchef-x86_64-pc-windows-msvc.exe.metadata.json"),
    true,
  );
  assert.equal(
    validSidecarMetadataFilename("emuchef-aarch64-apple-darwin.extra.metadata.json"),
    false,
  );
  assert.equal(
    validSidecarMetadataFilename("emuchef-aarch64-apple-darwin-extra.metadata.json"),
    false,
  );
  assert.equal(validSidecarMetadataFilename("emuchef-invalid.metadata.json"), false);
  for (const { args, message } of [
    { args: ["unsupported"], message: /expected preview, default, or deep mode/i },
    { args: [], message: /expected one mode and no path arguments/i },
    { args: ["preview", "extra"], message: /expected one mode and no path arguments/i },
  ]) {
    const result = invokeCleanupMain(args);
    assert.equal(result.value, 2);
    assert.match(result.stderr, message);
  }

  const defaultInventory = collectInventory(root, false);
  const deepInventory = collectInventory(root, true);
  assert.deepEqual(defaultInventory.errors, []);
  assert.ok(defaultInventory.eligible.some((item) => item.relativePath === cargoPath));
  assert.ok(defaultInventory.eligible.some((item) => item.relativePath === generatedPath));
  assert.ok(!defaultInventory.eligible.some((item) => item.relativePath === dependencyPath));
  assert.ok(!defaultInventory.eligible.some((item) => item.relativePath === distPath));
  assert.ok(deepInventory.eligible.some((item) => item.relativePath === dependencyPath));
  assert.ok(deepInventory.eligible.some((item) => item.relativePath === distPath));
  assert.ok(deepInventory.eligible.some((item) => item.relativePath === sidecarPath));
  assert.ok(deepInventory.eligible.some((item) => item.relativePath === metadataPath));
  assert.ok(deepInventory.skipped.some((item) => item.includes("unknown frontend output preserved")));

  const preview = captureCleanupOutput(() => printInventory("direct:", defaultInventory, false));
  const expectedDefaultBytes = Buffer.byteLength(cargoContents) + Buffer.byteLength(generatedContents);
  assert.match(preview.stdout, new RegExp(`eligible estimated bytes: ${expectedDefaultBytes}`));
  assert.match(preview.stdout, /SKIP deep-only categories:/);
  const removal = captureCleanupOutput(() =>
    printRemovalSummary("direct:", deepInventory, deepInventory.eligible),
  );
  const expectedDeepBytes = deepInventory.eligible.reduce((total, item) => total + item.size, 0);
  assert.match(removal.stdout, new RegExp(`removed estimated bytes: ${expectedDeepBytes}`));

  const removed = removeEligible(deepInventory);
  assert.equal(removed.length, deepInventory.eligible.length);
  for (const relativePath of [cargoPath, generatedPath, dependencyPath, distPath, sidecarPath, metadataPath]) {
    assertRemoved(root, relativePath);
  }
  assertExists(root, "apps/emuchef-app/dist/unknown.bin");
});

test("direct Cargo inventory removes recognized build, fingerprint, incremental, and cross-target outputs", (t) => {
  const { root } = createFixture(t);
  const targetRoot = "crates/emuchef-rust-backend/target";
  const eligiblePaths = [
    `${targetRoot}/debug/build/backend-0123456789abcdef/out/generated.rs`,
    `${targetRoot}/debug/.fingerprint/backend-0123456789abcdef/lib-backend.json`,
    `${targetRoot}/debug/incremental/backend-3l9cz3al16vr5/s-hn0lkk6b83-0t4n74k-br4pfaeeq8b6q27a5frw5n3u3/query-cache.bin`,
    `${targetRoot}/aarch64-unknown-linux-gnu/debug/deps/libtarget-0123456789abcdef.rlib`,
  ];
  for (const relativePath of eligiblePaths) writeFile(root, relativePath, `cargo output: ${relativePath}\n`);

  const inventory = collectInventory(root, false);

  assert.deepEqual(inventory.errors, []);
  assert.deepEqual(
    inventory.eligible.map((item) => item.relativePath).sort(),
    [...eligiblePaths].sort(),
  );
  assert.deepEqual(removeEligible(inventory).map((item) => item.relativePath).sort(), eligiblePaths.sort());
  for (const relativePath of eligiblePaths) assertRemoved(root, relativePath);
});

test("direct Cargo inventory preserves unknown, wrong-path, and cross-target content", (t) => {
  const { root } = createFixture(t);
  const targetRoot = "crates/emuchef-rust-backend/target";
  const preservedPaths = [
    `${targetRoot}/keep-important.json`,
    `${targetRoot}/debug/build/backend-0123456789abcdef/unrecognized/keep.txt`,
    `${targetRoot}/debug/build/backend-0123456789abcdef/notes.keep`,
    `${targetRoot}/debug/.fingerprint/backend-0123456789abcdef/unrecognized/keep.json`,
    `${targetRoot}/debug/.fingerprint/backend-0123456789abcdef/notes.keep`,
    `${targetRoot}/debug/incremental/operator-notes-archive/keep.bin`,
    `${targetRoot}/debug/deps/notes.keep`,
    `${targetRoot}/debug/operator-notes-archive/deps/libnotes-0123456789abcdef.rlib`,
    `${targetRoot}/wasm32-unknown-unknown/debug/operator-notes.txt`,
    `${targetRoot}/operator-notes-archive/deps/libnotes-0123456789abcdef.rlib`,
  ];
  for (const relativePath of preservedPaths) writeFile(root, relativePath, `preserved: ${relativePath}\n`);

  const inventory = collectInventory(root, false);

  assert.deepEqual(inventory.errors, []);
  assert.deepEqual(inventory.eligible, []);
  assert.deepEqual(removeEligible(inventory), []);
  for (const relativePath of preservedPaths) assertExists(root, relativePath);
});

test("Cargo inventory preserves unknown hyphenated target directories with Cargo-shaped profiles", (t) => {
  const { root } = createFixture(t);
  const targetRoot = "crates/emuchef-rust-backend/target";
  const unknownTargets = [
    `${targetRoot}/operator-notes-archive`,
    `${targetRoot}/aarch64-notes-darwin`,
  ];
  const preservedPaths = unknownTargets.map(
    (unknownTarget) => `${unknownTarget}/debug/deps/libnotes-0123456789abcdef.rlib`,
  );
  for (const preservedPath of preservedPaths) {
    writeFile(root, preservedPath, "operator data that resembles Cargo output\n");
  }

  const inventory = collectInventory(root, false);

  assert.deepEqual(inventory.errors, []);
  assert.deepEqual(inventory.eligible, []);
  for (const unknownTarget of unknownTargets) {
    assert.ok(
      inventory.skipped.includes(
        `${unknownTarget}: unrecognized Cargo target-triple directory preserved`,
      ),
    );
  }
  assert.deepEqual(removeEligible(inventory), []);
  for (const preservedPath of preservedPaths) assertExists(root, preservedPath);
});

test("direct Cargo inventory preserves protected release-like entries", (t) => {
  const { root } = createFixture(t);
  const targetRoot = "crates/emuchef-rust-backend/target";
  const buildOutputRoot = `${targetRoot}/debug/build/backend-0123456789abcdef/out`;
  const protectedPaths = [
    `${buildOutputRoot}/.git`,
    `${buildOutputRoot}/bundle`,
    `${buildOutputRoot}/EmuChef.app`,
    `${buildOutputRoot}/EmuChef.dmg`,
    `${buildOutputRoot}/release-manifest.json`,
  ];
  for (const relativePath of protectedPaths) writeFile(root, relativePath);

  const inventory = collectInventory(root, false);

  assert.deepEqual(inventory.errors, []);
  assert.deepEqual(inventory.eligible, []);
  assert.deepEqual(
    inventory.skipped.filter((item) => item.endsWith(": release bundle or manifest preserved")),
    protectedPaths
      .map((relativePath) => `${relativePath}: release bundle or manifest preserved`)
      .sort(),
  );
  assert.deepEqual(removeEligible(inventory), []);
  for (const relativePath of protectedPaths) assertExists(root, relativePath);
});

test("direct default inventory preserves profile bundles and unknown generated files", (t) => {
  const { root } = createFixture(t);
  const profileBundle = "crates/emuchef-rust-backend/target/debug/bundle/notes.txt";
  const generatedRoot = "apps/emuchef-app/src-tauri/gen";
  const knownSchema = `${generatedRoot}/schemas/desktop-schema.json`;
  const unknownSchema = `${generatedRoot}/schemas/operator-notes.json`;
  const unknownGeneratedFile = `${generatedRoot}/operator-notes.json`;

  writeFile(root, profileBundle, "preserve profile bundle\n");
  writeFile(root, knownSchema, "known generated schema\n");
  writeFile(root, unknownSchema, "preserve unknown schema\n");
  writeFile(root, unknownGeneratedFile, "preserve unknown generated file\n");

  const inventory = collectInventory(root, false);
  assert.deepEqual(inventory.errors, []);
  assert.deepEqual(
    inventory.eligible.map((item) => item.relativePath),
    [knownSchema],
  );
  assert.ok(
    inventory.skipped.includes(
      "crates/emuchef-rust-backend/target/debug/bundle: release or debug bundle subtree is preserved",
    ),
  );
  assert.ok(
    inventory.skipped.includes(
      `${unknownGeneratedFile}: unrecognized Tauri generated content preserved`,
    ),
  );
  assert.ok(
    inventory.skipped.includes(`${unknownSchema}: unrecognized Tauri generated content preserved`),
  );

  assert.deepEqual(
    removeEligible(inventory).map((item) => item.relativePath),
    [knownSchema],
  );
  assertRemoved(root, knownSchema);
  assertExists(root, profileBundle);
  assertExists(root, unknownSchema);
  assertExists(root, unknownGeneratedFile);
});

test("direct Cargo inventory handles cache and freshness markers once", (t) => {
  const { root } = createFixture(t);
  const targetRoot = "crates/emuchef-rust-backend/target";
  const markerPaths = [
    `${targetRoot}/CACHEDIR.TAG`,
    `${targetRoot}/.rustc_info.json`,
    `${targetRoot}/.emuchef-cargo-test-source.sha256`,
  ];
  const pendingMarker = `${targetRoot}/.emuchef-cargo-test-source.sha256.pending`;
  const cargoOutput = `${targetRoot}/debug/deps/libmarker-0123456789abcdef.rlib`;
  for (const relativePath of [...markerPaths, pendingMarker, cargoOutput]) {
    writeFile(root, relativePath);
  }

  const inventory = collectInventory(root, false);
  const expectedRemoved = [...markerPaths, cargoOutput].sort();

  assert.deepEqual(inventory.errors, []);
  assert.deepEqual(
    inventory.eligible.map((item) => item.relativePath).sort(),
    expectedRemoved,
  );
  assert.deepEqual(inventory.skipped, [
    `${pendingMarker}: backend-test-fresh pending marker preserved`,
  ]);
  assert.deepEqual(
    removeEligible(inventory).map((item) => item.relativePath).sort(),
    expectedRemoved,
  );
  for (const relativePath of expectedRemoved) assertRemoved(root, relativePath);
  assertExists(root, pendingMarker);
});

test("direct Cargo inventory reports malformed profile and target-triple roots", (t) => {
  const { root } = createFixture(t);
  const targetRoot = "crates/emuchef-rust-backend/target";
  const malformedPaths = [
    `${targetRoot}/aarch64-unknown-linux-gnu/debug`,
    `${targetRoot}/debug`,
    `${targetRoot}/x86_64-unknown-linux-gnu`,
  ];
  for (const relativePath of malformedPaths) writeFile(root, relativePath);

  const inventory = collectInventory(root, false);

  assert.deepEqual(inventory.eligible, []);
  assert.deepEqual(inventory.errors, [
    `${malformedPaths[0]}: Cargo profile is not a directory`,
    `${malformedPaths[1]}: Cargo profile is not a directory`,
    `${malformedPaths[2]}: target-triple output root is not a directory`,
  ]);
  assert.deepEqual(removeEligible(inventory), []);
  for (const relativePath of malformedPaths) assertExists(root, relativePath);
});

test(
  "direct Cargo inventory reports unsupported non-file entries",
  { skip: process.platform === "win32" },
  (t) => {
    const { root } = createFixture(t);
    const fifoRelativePath =
      "crates/emuchef-rust-backend/target/debug/incremental/backend-3l9cz3al16vr5/s-hn0lkk6b83-0t4n74k-br4pfaeeq8b6q27a5frw5n3u3/query-cache.bin";
    const fifoPath = path.join(root, fifoRelativePath);
    fs.mkdirSync(path.dirname(fifoPath), { recursive: true });
    const fifo = spawnSync("mkfifo", [fifoPath], { encoding: "utf8" });
    assert.equal(fifo.status, 0, fifo.stderr || fifo.error?.message);

    const inventory = collectInventory(root, false);

    assert.deepEqual(inventory.eligible, []);
    assert.deepEqual(inventory.errors, [
      `${fifoRelativePath}: unsupported filesystem entry in Cargo incremental output`,
    ]);
    assert.deepEqual(removeEligible(inventory), []);
    assert.ok(fs.lstatSync(fifoPath).isFIFO());
  },
);

test("direct Cargo inventory preserves known target roots without deletable outputs", (t) => {
  const { root } = createFixture(t);
  const profileWithoutOutputs = `${CARGO_TARGET_ROOT}/aarch64-unknown-linux-gnu/debug`;
  const unknownTargetProfile = `${CARGO_TARGET_ROOT}/x86_64-apple-darwin/operator-notes.txt`;
  writeFile(root, `${profileWithoutOutputs}/operator-notes.txt`, "preserve profile notes\n");
  writeFile(root, unknownTargetProfile, "preserve unknown target profile\n");

  const inventory = collectInventory(root, false);

  assert.deepEqual(inventory.errors, []);
  assert.deepEqual(inventory.eligible, []);
  assert.deepEqual(inventory.skipped, [
    `${CARGO_TARGET_ROOT}/.emuchef-cargo-test-source.sha256.pending: backend-test-fresh pending marker preserved`,
    `${profileWithoutOutputs}: target-triple profile lacks recognized Cargo output`,
    `${unknownTargetProfile}: outside the target-triple profile allowlist`,
  ].sort());
  assert.deepEqual(removeEligible(inventory), []);
  assertExists(root, `${profileWithoutOutputs}/operator-notes.txt`);
  assertExists(root, unknownTargetProfile);
});

test("direct generated inventory preserves tracked and symbolic schema content", (t) => {
  const { sandbox, root } = createFixture(t);
  const trackedSchema = `${APP_ROOT}/src-tauri/gen/schemas/desktop-schema.json`;
  const schemaLink = `${CONFIG_APP_ROOT}/src-tauri/gen/schemas/desktop-schema.json`;
  writeFile(root, trackedSchema, "tracked generated schema\n");
  forceTrack(root, trackedSchema);

  const externalSchema = writeFile(sandbox, "external/desktop-schema.json", "external schema\n");
  fs.mkdirSync(path.dirname(path.join(root, schemaLink)), { recursive: true });
  fs.symlinkSync(externalSchema, path.join(root, schemaLink));

  const inventory = collectInventory(root, false);

  assert.deepEqual(inventory.errors, []);
  assert.deepEqual(inventory.eligible, []);
  assert.ok(inventory.skipped.includes(`${trackedSchema}: tracked file preserved`));
  assert.ok(
    inventory.skipped.includes(
      `${schemaLink}: symbolic link cleanup candidate skipped`,
    ),
  );
  assert.deepEqual(removeEligible(inventory), []);
  assertExists(root, trackedSchema);
  assert.equal(fs.readFileSync(externalSchema, "utf8"), "external schema\n");
  assert.equal(fs.lstatSync(path.join(root, schemaLink)).isSymbolicLink(), true);
});

test("direct generated inventory reports a schema path that is not a regular file", (t) => {
  const { root } = createFixture(t);
  const schemaPath = `${APP_ROOT}/src-tauri/gen/schemas/desktop-schema.json`;
  fs.mkdirSync(path.join(root, schemaPath), { recursive: true });

  const inventory = collectInventory(root, false);

  assert.deepEqual(inventory.eligible, []);
  assert.deepEqual(inventory.errors, [
    `${schemaPath}: expected a regular generated file`,
  ]);
  assert.deepEqual(removeEligible(inventory), []);
  assert.equal(fs.lstatSync(path.join(root, schemaPath)).isDirectory(), true);
});

test("deep inventory fails closed for unsafe dependency roots", async (t) => {
  await t.test("dependency root is a regular file", (subtest) => {
    const { root } = createFixture(subtest);
    const dependencyRoot = `${APP_ROOT}/node_modules`;
    writeFile(root, dependencyRoot, "preserve malformed dependency root\n");

    const inventory = collectInventory(root, true);

    assert.ok(
      inventory.errors.includes(
        `${dependencyRoot}: expected a directory for deep cleanup`,
      ),
    );
    assert.deepEqual(removeEligible(inventory), []);
    assert.equal(
      fs.readFileSync(path.join(root, dependencyRoot), "utf8"),
      "preserve malformed dependency root\n",
    );
  });

  await t.test("dependency install stamp is tracked", (subtest) => {
    const { root } = createFixture(subtest);
    const dependencyRoot = `${APP_ROOT}/node_modules`;
    const stampPath = `${dependencyRoot}/.emuchef-deps-stamp`;
    const packagePath = `${dependencyRoot}/pkg/index.js`;
    writeFile(root, stampPath, "tracked dependency stamp\n");
    writeFile(root, packagePath, "preserve tracked-stamp dependency\n");
    forceTrack(root, stampPath);

    const inventory = collectInventory(root, true);

    assert.ok(
      inventory.errors.includes(
        `${stampPath}: tracked dependency-install stamp prevents safe deep cleanup`,
      ),
    );
    assert.deepEqual(removeEligible(inventory), []);
    assertExists(root, stampPath);
    assertExists(root, packagePath);
  });

  await t.test("dependency install stamp is a symbolic link", (subtest) => {
    const { root } = createFixture(subtest);
    const dependencyRoot = `${APP_ROOT}/node_modules`;
    const stampPath = `${dependencyRoot}/.emuchef-deps-stamp`;
    const packagePath = `${dependencyRoot}/pkg/index.js`;
    const externalStamp = writeFile(root, "apps/emuchef-app/external-deps-stamp", "external stamp\n");
    fs.mkdirSync(path.join(root, dependencyRoot), { recursive: true });
    fs.symlinkSync(externalStamp, path.join(root, stampPath));
    writeFile(root, packagePath, "preserve symlink-stamp dependency\n");

    const inventory = collectInventory(root, true);

    assert.ok(
      inventory.skipped.includes(
        `${stampPath}: symbolic link cleanup candidate skipped; dependency root cleanup skipped`,
      ),
    );
    assert.deepEqual(removeEligible(inventory), []);
    assertExists(root, packagePath);
    assert.equal(fs.readFileSync(externalStamp, "utf8"), "external stamp\n");
    assert.equal(fs.lstatSync(path.join(root, stampPath)).isSymbolicLink(), true);
  });

  await t.test("dependency install stamp is a directory", (subtest) => {
    const { root } = createFixture(subtest);
    const dependencyRoot = `${APP_ROOT}/node_modules`;
    const stampPath = `${dependencyRoot}/.emuchef-deps-stamp`;
    const packagePath = `${dependencyRoot}/pkg/index.js`;
    fs.mkdirSync(path.join(root, stampPath), { recursive: true });
    writeFile(root, packagePath, "preserve directory-stamp dependency\n");

    const inventory = collectInventory(root, true);

    assert.ok(
      inventory.errors.includes(
        `${stampPath}: dependency-install stamp is not a regular file`,
      ),
    );
    assert.deepEqual(removeEligible(inventory), []);
    assertExists(root, packagePath);
    assert.equal(fs.lstatSync(path.join(root, stampPath)).isDirectory(), true);
  });
});

test("deep inventory deletes known outputs while preserving protected deep entries", (t) => {
  const { root } = createFixture(t);
  const dependencyFile = `${APP_ROOT}/node_modules/pkg/index.js`;
  const gitMetadataRoot = `${APP_ROOT}/node_modules/.git`;
  const gitMetadata = `${gitMetadataRoot}/objects/keep`;
  const dependencyLink = `${APP_ROOT}/node_modules/linked-package`;
  const externalPackage = writeFile(root, "apps/emuchef-app/external-package/package.json", "external package\n");
  const unknownSidecar = `${APP_ROOT}/src-tauri/binaries/operator-notes.bin`;
  const knownSidecar = `${APP_ROOT}/src-tauri/binaries/emuchef-aarch64-apple-darwin`;
  const ignoredDistFile = `${CONFIG_APP_ROOT}/dist/.gitignore`;

  writeFile(root, dependencyFile, "known dependency output\n");
  writeFile(root, gitMetadata, "preserve nested Git metadata\n");
  fs.symlinkSync(externalPackage, path.join(root, dependencyLink));
  writeFile(root, unknownSidecar, "preserve unknown sidecar\n");
  writeFile(root, knownSidecar, "known sidecar\n");
  writeFile(root, ignoredDistFile, "preserve dist ignore file\n");

  const inventory = collectInventory(root, true);
  const eligiblePaths = inventory.eligible.map((item) => item.relativePath);

  assert.deepEqual(inventory.errors, []);
  assert.ok(eligiblePaths.includes(dependencyFile));
  assert.ok(eligiblePaths.includes(knownSidecar));
  assert.ok(inventory.skipped.includes(`${gitMetadataRoot}: Git metadata preserved`));
  assert.ok(
    inventory.skipped.includes(
      `${dependencyLink}: symbolic link cleanup candidate skipped`,
    ),
  );
  assert.ok(
    inventory.skipped.includes(
      `${unknownSidecar}: unknown Tauri binary-directory content preserved`,
    ),
  );
  assert.ok(inventory.skipped.includes(`${ignoredDistFile}: .gitignore preserved`));

  const removedPaths = removeEligible(inventory).map((item) => item.relativePath);

  assert.ok(removedPaths.includes(dependencyFile));
  assert.ok(removedPaths.includes(knownSidecar));
  assertRemoved(root, dependencyFile);
  assertRemoved(root, knownSidecar);
  assertExists(root, gitMetadata);
  assertExists(root, unknownSidecar);
  assertExists(root, ignoredDistFile);
  assert.equal(fs.readFileSync(externalPackage, "utf8"), "external package\n");
  assert.equal(fs.lstatSync(path.join(root, dependencyLink)).isSymbolicLink(), true);
});
