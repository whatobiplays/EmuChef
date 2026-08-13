import assert from "node:assert/strict";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import * as fixtureTool from "./phase-6c-fixture.mjs";

const {
  fixtureMetadataFromToolOutput,
  normalizeSha256,
  validateFixtureMetadata,
} = fixtureTool;

const EXPECTED_METADATA = {
  packageName: "com.emuchef.fixture",
  versionCode: 1,
  versionName: "1.0.0",
  minSdkVersion: 30,
  targetSdkVersion: 35,
  launcherActivity: "com.emuchef.fixture.MainActivity",
  declaredPermissions: ["android.permission.CAMERA"],
  signingCertificateSha256:
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
};

const FIXTURE_METADATA_PATH = new URL(
  "../tests/fixtures/phase-6c/non-root/android-fixture/fixture-metadata.json",
  import.meta.url,
);
const FIXTURE_BUILD_SCRIPT_PATH = new URL(
  "../scripts/build-phase-6c-android-fixture.sh",
  import.meta.url,
);
const QUALIFICATION_CONTRACT_PATH = new URL(
  "../tests/fixtures/phase-6c/non-root/qualification-contract.json",
  import.meta.url,
);
const FIXTURE_TOOL_PATH = new URL("./phase-6c-fixture.mjs", import.meta.url);
const FEATURE_MATRIX_WORKFLOW_PATH = new URL(
  "../.github/workflows/emuchef-execution-feature-matrix.yml",
  import.meta.url,
);
test("normalizes a fixture checksum to lowercase hexadecimal", () => {
  assert.equal(
    normalizeSha256(" 0123456789ABCDEF0123456789abcdef0123456789ABCDEF0123456789abcdef "),
    EXPECTED_METADATA.signingCertificateSha256,
  );
  assert.throws(() => normalizeSha256("not-a-digest"), /64-character SHA-256/);
});

test("parses the semantic Android fixture contract from SDK tool output", () => {
  const metadata = fixtureMetadataFromToolOutput({
    badging: [
      "package: name='com.emuchef.fixture' versionCode='1' versionName='1.0.0'",
      "minSdkVersion:'30'",
      "targetSdkVersion:'35'",
      "launchable-activity: name='com.emuchef.fixture.MainActivity'  label='EmuChef Fixture' icon=''",
      "uses-permission: name='android.permission.CAMERA'",
    ].join("\n"),
    certificate: "Certificate SHA-256 digest: 01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF\n",
  });

  assert.deepEqual(metadata, EXPECTED_METADATA);
});

test("rejects missing or malformed SDK metadata from Android tools", () => {
  const certificate =
    "Certificate SHA-256 digest: 01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF\n";
  const baseBadging = [
    "package: name='com.emuchef.fixture' versionCode='1' versionName='1.0.0'",
    "launchable-activity: name='com.emuchef.fixture.MainActivity'",
    "uses-permission: name='android.permission.CAMERA'",
  ];

  assert.throws(
    () => fixtureMetadataFromToolOutput({ badging: baseBadging.join("\n"), certificate }),
    /minimum and target SDK metadata/,
  );
  assert.throws(
    () =>
      fixtureMetadataFromToolOutput({
        badging: [...baseBadging, "minSdkVersion:'thirty'", "targetSdkVersion:'35'"].join("\n"),
        certificate,
      }),
    /minimum and target SDK metadata/,
  );
});

test("rejects missing or malformed signing metadata from Android tools", () => {
  const badging = [
    "package: name='com.emuchef.fixture' versionCode='1' versionName='1.0.0'",
    "minSdkVersion:'30'",
    "targetSdkVersion:'35'",
    "launchable-activity: name='com.emuchef.fixture.MainActivity'",
    "uses-permission: name='android.permission.CAMERA'",
  ].join("\n");

  assert.throws(
    () => fixtureMetadataFromToolOutput({ badging, certificate: "" }),
    /signing metadata/,
  );
  assert.throws(
    () =>
      fixtureMetadataFromToolOutput({
        badging,
        certificate: "Certificate SHA-256 digest: not-a-digest",
      }),
    /64-character SHA-256/,
  );
});

test("rejects an APK metadata contract that omits the declared camera permission", () => {
  assert.throws(
    () =>
      validateFixtureMetadata(
        { ...EXPECTED_METADATA, declaredPermissions: [] },
        EXPECTED_METADATA,
      ),
    /declaredPermissions/,
  );
});

test("rejects SDK and signing digest mismatches", () => {
  for (const actual of [
    { ...EXPECTED_METADATA, minSdkVersion: 29 },
    { ...EXPECTED_METADATA, targetSdkVersion: 34 },
    { ...EXPECTED_METADATA, signingCertificateSha256: "f".repeat(64) },
  ]) {
    assert.throws(() => validateFixtureMetadata(actual, EXPECTED_METADATA), /metadata mismatch/);
  }
});

test("commits the non-root fixture identity and CAMERA permission contract", () => {
  const metadata = JSON.parse(readFileSync(FIXTURE_METADATA_PATH, "utf8"));
  assert.equal(metadata.packageName, "com.emuchef.fixture");
  assert.equal(metadata.minSdkVersion, 30);
  assert.equal(metadata.targetSdkVersion, 35);
  assert.deepEqual(metadata.declaredPermissions, ["android.permission.CAMERA"]);
  assert.match(metadata.signingKeystore.alias, /^emuchef-fixture-test$/);
  assert.notEqual(metadata.signingCertificateSha256, "0".repeat(64));
});

test("builds fixture resources as a complete application package", () => {
  const buildScript = readFileSync(FIXTURE_BUILD_SCRIPT_PATH, "utf8");
  assert.match(buildScript, /aapt2" link --auto-add-overlay/);
});

test("packages compiled classes before supplying them to d8", () => {
  const buildScript = readFileSync(FIXTURE_BUILD_SCRIPT_PATH, "utf8");
  assert.match(buildScript, /"\$jar" cf "\$work_root\/classes\.jar" -C "\$work_root\/classes" \./);
  assert.match(buildScript, /"\$d8" --min-api 30 --output "\$work_root\/dex" "\$work_root\/classes\.jar"/);
});

test("limits device qualification to manifest-declared test-owned storage", () => {
  const contract = JSON.parse(readFileSync(QUALIFICATION_CONTRACT_PATH, "utf8"));
  assert.equal(
    contract.sharedStorageRoot,
    "/sdcard/EmuChefQualification/com.emuchef.fixture/",
  );
  assert.equal(
    contract.appSpecificExternalStorageRoot,
    "/sdcard/Android/data/com.emuchef.fixture/files/",
  );
  assert.equal(contract.packageName, "com.emuchef.fixture");
});

test("keeps the fixture signing identity out of production Tauri configuration", () => {
  const metadata = JSON.parse(readFileSync(FIXTURE_METADATA_PATH, "utf8"));
  const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
  assert.equal(typeof fixtureTool.findFixtureSigningReferences, "function");
  assert.deepEqual(
    fixtureTool.findFixtureSigningReferences(repositoryRoot, metadata.signingKeystore),
    [],
  );
});

test("detects fixture signing filenames and aliases across production surfaces", () => {
  const repositoryRoot = mkdtempSync(path.join(tmpdir(), "emuchef-signing-scan-"));
  const metadata = JSON.parse(readFileSync(FIXTURE_METADATA_PATH, "utf8"));
  try {
    for (const relative of [
      ".github/workflows",
      "apps/emuchef-app/scripts",
      "apps/emuchef-app/src-tauri",
      "scripts",
    ]) {
      mkdirSync(path.join(repositoryRoot, relative), { recursive: true });
    }
    writeFileSync(
      path.join(repositoryRoot, "apps/emuchef-app/package.json"),
      JSON.stringify({ signing: metadata.signingKeystore.file }),
    );
    writeFileSync(
      path.join(repositoryRoot, ".github/workflows/release.yml"),
      `alias: ${metadata.signingKeystore.alias}\n`,
    );
    writeFileSync(
      path.join(repositoryRoot, "scripts/build-phase-6c-android-fixture.sh"),
      `${metadata.signingKeystore.file} ${metadata.signingKeystore.alias}\n`,
    );

    assert.deepEqual(
      fixtureTool.findFixtureSigningReferences(repositoryRoot, metadata.signingKeystore),
      [
        ".github/workflows/release.yml",
        "apps/emuchef-app/package.json",
      ],
    );
  } finally {
    rmSync(repositoryRoot, { recursive: true, force: true });
  }
});

test("fails closed on symlinks within production signing surfaces", () => {
  const repositoryRoot = mkdtempSync(path.join(tmpdir(), "emuchef-signing-symlink-"));
  const metadata = JSON.parse(readFileSync(FIXTURE_METADATA_PATH, "utf8"));
  try {
    for (const relative of [
      ".github/workflows",
      "apps/emuchef-app/scripts",
      "apps/emuchef-app/src-tauri",
      "scripts",
    ]) {
      mkdirSync(path.join(repositoryRoot, relative), { recursive: true });
    }
    writeFileSync(path.join(repositoryRoot, "apps/emuchef-app/package.json"), "{}\n");
    const target = path.join(repositoryRoot, "symlink-target.yml");
    writeFileSync(target, `alias: ${metadata.signingKeystore.alias}\n`);
    symlinkSync(target, path.join(repositoryRoot, ".github/workflows/release.yml"));

    assert.throws(
      () => fixtureTool.findFixtureSigningReferences(repositoryRoot, metadata.signingKeystore),
      /symbolic link/,
    );
  } finally {
    rmSync(repositoryRoot, { recursive: true, force: true });
  }
});

test("uses a portable CLI entry check and pins the fixture job to Node 22", () => {
  const fixtureToolSource = readFileSync(FIXTURE_TOOL_PATH, "utf8");
  const workflow = readFileSync(FEATURE_MATRIX_WORKFLOW_PATH, "utf8");
  assert.doesNotMatch(fixtureToolSource, /import\.meta\.main/);
  assert.match(fixtureToolSource, /pathToFileURL/);
  assert.match(
    workflow,
    /android-qualification-fixture:[\s\S]*actions\/setup-node@v7[\s\S]*node-version: 22/,
  );
});
