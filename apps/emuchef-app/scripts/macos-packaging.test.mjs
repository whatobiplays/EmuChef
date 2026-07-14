import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  APPLE_VARIABLE_ALLOWLIST,
  appTreeDigest,
  assertSafeManifest,
  canonicalJson,
  catalogDigest,
  compareReleaseManifests,
  createReleaseManifest,
  developerIdBuildEnvironment,
  discoverArtifacts,
  forbiddenBinaryReason,
  forbiddenBundleReason,
  localBuildEnvironment,
  normalizedBuildEnvironment,
  normalizedContentManifest,
  parseOptions,
  qualificationPolicy,
  requireThinArm64,
  semanticDigest,
  validateDeveloperIdEnvironment,
  validatePackagingConfiguration,
  validateQualificationProbe,
} from "./macos-packaging.mjs";
import {
  buildNormalizedContent,
  classifySigningState,
  selectArtifacts,
  validateDeveloperIdMetadata,
  validateInfoPlist,
  validatePackagedPolicy,
} from "./macos-packaging-policy.mjs";

const goodConfig = {
  packageJson: { version: "0.1.0" },
  cargoToml: '[package]\nname = "emuchef-app"\nversion = "0.1.0"\n',
  targetTriple: "aarch64-apple-darwin",
  tauriConfig: {
    productName: "EmuChef",
    version: "0.1.0",
    identifier: "com.emuchef.desktop",
    build: {},
    app: { security: { csp: "default-src 'self'" } },
    bundle: {
      icon: ["icons/icon.icns"],
      externalBin: ["binaries/emuchef"],
      resources: {
        a: "catalog/apps",
        b: "catalog/device_plans",
        c: "catalog/device_profiles",
        d: "catalog/recipes",
        e: "qualification/qualification-policy.json",
      },
      macOS: { signingIdentity: "-", hardenedRuntime: true, minimumSystemVersion: "11.0" },
    },
  },
};

test("local mode ignores and neutralizes the fixed Apple variable allowlist", () => {
  const secret = "DO-NOT-PRINT-THIS";
  const populated = Object.fromEntries(APPLE_VARIABLE_ALLOWLIST.map((name) => [name, `${secret}-${name}`]));
  const withCredentials = localBuildEnvironment({ KEEP: "yes", ...populated });
  const withoutCredentials = localBuildEnvironment({ KEEP: "yes" });
  assert.equal(withCredentials.KEEP, "yes");
  assert.equal(withCredentials.APPLE_SIGNING_IDENTITY, "-");
  assert.deepEqual(withCredentials, withoutCredentials);
  assert.doesNotMatch(JSON.stringify(withCredentials), new RegExp(secret));
});

test("developer-id mode reads only the allowlisted credential sets and returns no values", () => {
  const api = {
    APPLE_SIGNING_IDENTITY: "Developer ID Application: Example",
    APPLE_API_ISSUER: "issuer-secret",
    APPLE_API_KEY: "key-secret",
    APPLE_API_KEY_PATH: "/private/key.p8",
    UNRELATED_SECRET: "untouched",
  };
  assert.deepEqual(validateDeveloperIdEnvironment(api), { authentication: "app-store-connect-api" });
  assert.deepEqual(developerIdBuildEnvironment(api).summary, { authentication: "app-store-connect-api" });
  assert.deepEqual(
    validateDeveloperIdEnvironment({
      ...api,
      APPLE_ID: "id",
      APPLE_PASSWORD: "password",
      APPLE_TEAM_ID: "team",
    }),
    { authentication: "app-store-connect-api" },
  );
  assert.deepEqual(
    validateDeveloperIdEnvironment({
      APPLE_SIGNING_IDENTITY: "Developer ID Application: Example",
      APPLE_ID: "id",
      APPLE_PASSWORD: "password",
      APPLE_TEAM_ID: "team",
    }),
    { authentication: "apple-id" },
  );
  assert.throws(() => validateDeveloperIdEnvironment({}), /APPLE_SIGNING_IDENTITY/);
  assert.throws(
    () => validateDeveloperIdEnvironment({ APPLE_SIGNING_IDENTITY: "Apple Development: Example" }),
    /Developer ID Application/,
  );
  assert.throws(
    () => validateDeveloperIdEnvironment({ APPLE_SIGNING_IDENTITY: "Developer ID Application: Example" }),
    /requires either/,
  );
  assert.throws(
    () =>
      validateDeveloperIdEnvironment({
        APPLE_SIGNING_IDENTITY: "Developer ID Application: Example",
        APPLE_API_ISSUER: "issuer",
        APPLE_API_KEY: "   ",
        APPLE_API_KEY_PATH: 7,
      }),
    /requires either/,
  );
  for (const errorFactory of [
    () => validateDeveloperIdEnvironment({}),
    () => validateDeveloperIdEnvironment({ APPLE_SIGNING_IDENTITY: "Developer ID Application: Example" }),
  ]) {
    try {
      errorFactory();
    } catch (error) {
      assert.doesNotMatch(error.message, /issuer-secret|key-secret|password/);
    }
  }
});

test("credentialed CLI failures do not echo allowlisted values", () => {
  const sentinel = "CREDENTIAL-VALUE-MUST-NOT-APPEAR";
  const result = spawnSync(
    process.execPath,
    [path.join(import.meta.dirname, "macos-package.mjs"), "build", "--mode", "developer-id"],
    {
      cwd: path.resolve(import.meta.dirname, ".."),
      env: {
        ...process.env,
        APPLE_SIGNING_IDENTITY: `Developer ID Application: ${sentinel}`,
        APPLE_API_ISSUER: sentinel,
      },
      encoding: "utf8",
    },
  );
  assert.notEqual(result.status, 0);
  assert.doesNotMatch(`${result.stdout}\n${result.stderr}`, new RegExp(sentinel));
  assert.match(result.stderr, /requires either/);
});

test("normalized build environment replaces caller-dependent flags", () => {
  const result = normalizedBuildEnvironment(
    { RUSTFLAGS: "secret-path", CARGO_BUILD_RUSTFLAGS: "other", TAURI_ENV_DEBUG: "true" },
    { repoRoot: "/repo", homeDir: "/home/example", mode: "local" },
  );
  assert.equal(result.APPLE_SIGNING_IDENTITY, "-");
  assert.equal(result.RUSTFLAGS, undefined);
  assert.equal(result.CARGO_BUILD_RUSTFLAGS, undefined);
  assert.equal(result.TAURI_ENV_DEBUG, undefined);
  assert.match(result.CARGO_ENCODED_RUSTFLAGS, /emuchef-source/);
  assert.doesNotMatch(result.CARGO_ENCODED_RUSTFLAGS, /secret-path/);
  const developer = normalizedBuildEnvironment(
    {
      APPLE_SIGNING_IDENTITY: "Developer ID Application: Example",
      APPLE_API_ISSUER: "issuer",
      APPLE_API_KEY: "key",
      APPLE_API_KEY_PATH: "key.p8",
    },
    { repoRoot: "/repo", homeDir: "/home/example", mode: "developer-id" },
  );
  assert.equal(developer.APPLE_API_KEY, "key");
});

test("parses bounded packaging options", () => {
  assert.deepEqual(parseOptions([]), { mode: "local", positional: [] });
  assert.deepEqual(parseOptions(["--mode", "developer-id", "one", "two"]), {
    mode: "developer-id",
    positional: ["one", "two"],
  });
  assert.equal(parseOptions(["--app", "Example.app"]).app, "Example.app");
  assert.throws(() => parseOptions(["--mode"]), /requires a value/);
  assert.throws(() => parseOptions(["--mode", "wrong"]), /local or developer-id/);
  assert.throws(() => parseOptions(["--unknown"]), /unknown option/);
});

test("validates the complete qualified packaging configuration", () => {
  assert.deepEqual(validatePackagingConfiguration(goodConfig), {
    appVersion: "0.1.0",
    architecture: "arm64",
    targetTriple: "aarch64-apple-darwin",
  });
  const cases = [
    [{ cargoToml: '[package]\nname = "emuchef-app"\n' }, /package version/],
    [{ packageJson: { version: "0.2.0" } }, /versions must match/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, identifier: "wrong" } }, /product identity/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, productName: "wrong" } }, /product identity/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, build: { devUrl: "http://localhost" } } }, /devUrl/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, app: { security: { csp: "unsafe-eval" } } } }, /CSP/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, app: { security: { csp: "http://127.0.0.1:5174" } } } }, /CSP/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, bundle: { ...goodConfig.tauriConfig.bundle, externalBin: [] } } }, /externalBin/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, bundle: { ...goodConfig.tauriConfig.bundle, resources: { e: "qualification/qualification-policy.json" } } } }, /omit catalog/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, bundle: { ...goodConfig.tauriConfig.bundle, resources: { a: "catalog/apps", b: "catalog/device_plans", c: "catalog/device_profiles", d: "catalog/recipes" } } } }, /qualification policy/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, bundle: { ...goodConfig.tauriConfig.bundle, macOS: { signingIdentity: "Developer ID", hardenedRuntime: true, minimumSystemVersion: "11.0" } } } }, /ad-hoc default/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, bundle: { ...goodConfig.tauriConfig.bundle, macOS: { signingIdentity: "-", hardenedRuntime: false, minimumSystemVersion: "11.0" } } } }, /ad-hoc default/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, bundle: { ...goodConfig.tauriConfig.bundle, macOS: { signingIdentity: "-", hardenedRuntime: true, minimumSystemVersion: "12.0" } } } }, /ad-hoc default/],
    [{ tauriConfig: { ...goodConfig.tauriConfig, bundle: { ...goodConfig.tauriConfig.bundle, icon: [] } } }, /icon/],
    [{ targetTriple: "x86_64-apple-darwin" }, /qualifies only/],
  ];
  for (const [override, pattern] of cases) {
    assert.throws(() => validatePackagingConfiguration({ ...goodConfig, ...override }), pattern);
  }
});

test("qualification metadata is explicit and real execution remains disabled", () => {
  assert.deepEqual(
    qualificationPolicy({ appVersion: "1.2.3", architecture: "arm64", targetTriple: "aarch64-apple-darwin" }),
    {
      schemaVersion: 1,
      qualificationPolicyVersion: 1,
      appVersion: "1.2.3",
      targetTriple: "aarch64-apple-darwin",
      architecture: "arm64",
      buildMode: "release",
      realExecutionEnabled: false,
    },
  );
});

test("canonical semantic digests ignore object key order and detect content changes", () => {
  assert.equal(canonicalJson({ b: 2, a: 1 }), canonicalJson({ a: 1, b: 2 }));
  assert.equal(semanticDigest({ b: 2, a: 1 }), semanticDigest({ a: 1, b: 2 }));
  assert.notEqual(semanticDigest({ a: 1 }), semanticDigest({ a: 2 }));
});

test("normalized comparison ignores raw artifact volatility but detects real content changes", () => {
  const content = {
    product: { appVersion: "1" },
    target: { architecture: "arm64" },
    executables: { main: { unsignedContentSha256: "a" }, sidecar: { unsignedContentSha256: "b" } },
    infoPlist: { CFBundleIdentifier: "com.example" },
    catalog: { sha256: "c", fileCount: 1 },
    qualificationPolicy: { realExecutionEnabled: false },
    capabilities: { semanticSha256: "d" },
    tauriSecurity: { csp: "self" },
  };
  const normalized = normalizedContentManifest(content);
  const left = createReleaseManifest({
    normalized,
    rawArtifacts: { app: { treeSha256: "volatile-one" } },
    provenance: { toolchain: {} },
    signingState: "ad-hoc",
  });
  const right = structuredClone(left);
  right.rawArtifactIdentities.app.treeSha256 = "volatile-two";
  assert.deepEqual(compareReleaseManifests(left, right), {
    normalizedContentMatches: true,
    rawArtifactHashesMayDiffer: true,
    result: "equivalent-normalized-content",
  });
  right.normalizedContent = normalizedContentManifest({ ...content, catalog: { sha256: "changed", fileCount: 1 } });
  assert.equal(compareReleaseManifests(left, right).normalizedContentMatches, false);
  assert.equal(left.reproducibility.byteIdenticalSignedArtifactsClaimed, false);
  assert.equal(left.reproducibility.rawHashesArePerBuildIdentities, true);
  assert.equal(compareReleaseManifests(null, null).normalizedContentMatches, true);
  assert.equal(compareReleaseManifests(left, null).normalizedContentMatches, false);
});

test("manifest sanitizer rejects credentials and caller paths", () => {
  assert.equal(assertSafeManifest({ safe: true }), true);
  assert.throws(() => assertSafeManifest({ path: "/Users/example/build" }), /forbidden/);
  assert.throws(() => assertSafeManifest({ APPLE_PASSWORD: "value" }), /forbidden/);
  assert.throws(() => assertSafeManifest({ deviceSerial: "value" }), /forbidden/);
  assert.equal(assertSafeManifest({ safe: true }, { forbiddenValues: [""] }), true);
  assert.throws(() => assertSafeManifest({ safe: "hidden-value" }, { forbiddenValues: ["hidden-value"] }), /credential value/);
});

test("binary path-leak detection ignores its own guard tokens but rejects concrete paths", () => {
  assert.equal(forbiddenBinaryReason("/Users//home/"), false);
  assert.equal(forbiddenBinaryReason("/System/Library/Frameworks/WebKit.framework/WebKit"), false);
  assert.equal(forbiddenBinaryReason("/Users/developer/Projects/other/lib.dylib"), true);
  assert.equal(forbiddenBinaryReason("/home/builder/work/lib.so"), true);
  assert.equal(forbiddenBinaryReason("http://localhost:5174"), true);
});

test("bundle safety checks reject development content and non-qualified architectures", () => {
  assert.equal(forbiddenBundleReason("Contents/Resources/assets/index.js"), false);
  assert.equal(forbiddenBundleReason("Contents/Resources/node_modules/module.js"), true);
  assert.equal(forbiddenBundleReason("Contents/Resources/assets/index.js.map"), true);
  assert.doesNotThrow(() => requireThinArm64("sidecar", "Mach-O 64-bit executable arm64"));
  assert.throws(() => requireThinArm64("sidecar", "Mach-O 64-bit executable x86_64"), /thin arm64/);
  assert.throws(
    () => requireThinArm64("sidecar", "Mach-O universal binary with 2 architectures: arm64 x86_64"),
    /thin arm64/,
  );
});

test("qualification probe requires all safe runtime facts", () => {
  const report = {
    kind: "macos_packaged_app_qualification",
    status: "passed",
    runtimeReady: true,
    catalogLoaded: true,
    readOnlyCatalogOperation: true,
    realExecutionEnabled: false,
  };
  assert.equal(validateQualificationProbe(report), report);
  for (const [field, value] of [
    ["kind", "wrong"],
    ["status", "failed"],
    ["runtimeReady", false],
    ["catalogLoaded", false],
    ["readOnlyCatalogOperation", false],
    ["realExecutionEnabled", true],
  ]) {
    assert.throws(() => validateQualificationProbe({ ...report, [field]: value }), /did not prove/);
  }
});

test("pure bundle metadata policy validates every security-relevant field", () => {
  const info = {
    CFBundleIdentifier: "com.emuchef.desktop",
    CFBundleDisplayName: "EmuChef",
    CFBundleShortVersionString: "0.1.0",
    CFBundleVersion: "0.1.0",
    LSMinimumSystemVersion: "11.0",
    CFBundlePackageType: "APPL",
    CFBundleExecutable: "emuchef-app",
  };
  assert.equal(validateInfoPlist(info, goodConfig.tauriConfig), info);
  for (const field of [
    "CFBundleIdentifier",
    "CFBundleDisplayName",
    "CFBundleShortVersionString",
    "CFBundleVersion",
    "LSMinimumSystemVersion",
    "CFBundlePackageType",
  ]) {
    assert.throws(() => validateInfoPlist({ ...info, [field]: "wrong" }, goodConfig.tauriConfig), new RegExp(field));
  }

  const policy = qualificationPolicy({
    appVersion: "0.1.0",
    architecture: "arm64",
    targetTriple: "aarch64-apple-darwin",
  });
  assert.equal(validatePackagedPolicy(policy, "0.1.0"), policy);
  for (const [field, value] of [
    ["qualificationPolicyVersion", 2],
    ["realExecutionEnabled", true],
    ["targetTriple", "x86_64-apple-darwin"],
    ["appVersion", "9.9.9"],
  ]) {
    assert.throws(() => validatePackagedPolicy({ ...policy, [field]: value }, "0.1.0"), /inconsistent/);
  }
});

test("pure signing and artifact policies fail closed", () => {
  assert.equal(classifySigningState(0, "Signature=adhoc"), "ad-hoc");
  assert.equal(
    classifySigningState(0, "Authority=Developer ID Application: Example"),
    "developer-id",
  );
  assert.throws(() => classifySigningState(1, "Signature=adhoc"), /neither valid/);
  assert.throws(() => classifySigningState(0, "unsigned"), /neither valid/);
  assert.deepEqual(selectArtifacts(["EmuChef.app"], ["EmuChef.dmg"]), {
    appPath: "EmuChef.app",
    dmgPath: "EmuChef.dmg",
  });
  assert.throws(() => selectArtifacts([], ["EmuChef.dmg"]), /exactly one/);
  assert.throws(() => selectArtifacts(["EmuChef.app"], []), /exactly one/);

  const metadata = "Authority=Developer ID Application: Example\nTimestamp=2026-07-14\nflags=0x10000(runtime)";
  assert.equal(validateDeveloperIdMetadata(metadata), true);
  for (const invalid of [
    "Timestamp=2026-07-14\nflags=runtime",
    "Authority=Developer ID Application: Example\nflags=runtime",
    "Authority=Developer ID Application: Example\nTimestamp=none\nflags=runtime",
    "Authority=Developer ID Application: Example\nTimestamp=2026-07-14",
  ]) {
    assert.throws(() => validateDeveloperIdMetadata(invalid), /required Developer ID metadata/);
  }
});

test("normalized content assembly covers executable, resource, and security semantics", () => {
  const verification = {
    info: {
      CFBundleDisplayName: "EmuChef",
      CFBundleExecutable: "emuchef-app",
      CFBundleIdentifier: "com.emuchef.desktop",
      CFBundlePackageType: "APPL",
      CFBundleShortVersionString: "0.1.0",
      CFBundleVersion: "0.1.0",
      LSMinimumSystemVersion: "11.0",
    },
    policy: qualificationPolicy({
      appVersion: "0.1.0",
      architecture: "arm64",
      targetTriple: "aarch64-apple-darwin",
    }),
    catalog: { fileCount: 18, sha256: "catalog" },
  };
  const normalized = buildNormalizedContent(
    verification,
    { identifier: "default", permissions: ["core:default"] },
    goodConfig.tauriConfig,
    { main: "main-hash", sidecar: "sidecar-hash" },
  );
  assert.equal(normalized.content.executables.main.unsignedContentSha256, "main-hash");
  assert.equal(normalized.content.executables.sidecar.unsignedContentSha256, "sidecar-hash");
  assert.equal(normalized.content.qualificationPolicy.realExecutionEnabled, false);
  assert.equal(normalized.content.capabilities.value.permissions[0], "core:default");
  assert.match(normalized.normalizedContentSha256, /^[a-f0-9]{64}$/);
});

test("tree and catalog digests ignore mtimes but detect content changes", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-digest-test-"));
  try {
    fs.mkdirSync(path.join(root, "app"));
    fs.writeFileSync(path.join(root, "app", "one"), "one");
    const first = appTreeDigest(path.join(root, "app"));
    fs.utimesSync(path.join(root, "app", "one"), new Date(0), new Date());
    assert.equal(appTreeDigest(path.join(root, "app")), first);
    fs.writeFileSync(path.join(root, "app", "one"), "two");
    assert.notEqual(appTreeDigest(path.join(root, "app")), first);

    const catalog = path.join(root, "catalog");
    for (const directory of ["apps", "device_plans", "device_profiles", "recipes"]) {
      fs.mkdirSync(path.join(catalog, directory), { recursive: true });
      fs.writeFileSync(path.join(catalog, directory, "one.yaml"), directory);
    }
    const catalogFirst = catalogDigest(catalog);
    assert.equal(catalogFirst.fileCount, 4);
    fs.utimesSync(path.join(catalog, "apps", "one.yaml"), new Date(), new Date());
    assert.deepEqual(catalogDigest(catalog), catalogFirst);
    fs.writeFileSync(path.join(catalog, "apps", "one.yaml"), "changed");
    assert.notEqual(catalogDigest(catalog).sha256, catalogFirst.sha256);
    fs.symlinkSync("one.yaml", path.join(catalog, "apps", "linked.yaml"));
    assert.throws(() => catalogDigest(catalog), /symlink/);
    fs.unlinkSync(path.join(catalog, "apps", "linked.yaml"));
    fs.writeFileSync(path.join(catalog, "apps", "unexpected.txt"), "unsupported");
    assert.throws(() => catalogDigest(catalog), /unsupported content/);
    fs.unlinkSync(path.join(catalog, "apps", "unexpected.txt"));
    fs.rmSync(path.join(catalog, "recipes"), { recursive: true });
    assert.throws(() => catalogDigest(catalog), /catalog\/recipes is missing/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("artifact discovery rejects missing or ambiguous outputs", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-artifact-test-"));
  try {
    fs.mkdirSync(path.join(root, "macos"));
    fs.mkdirSync(path.join(root, "dmg"));
    assert.throws(() => discoverArtifacts(root), /exactly one/);
    fs.mkdirSync(path.join(root, "macos", "EmuChef.app"));
    fs.writeFileSync(path.join(root, "dmg", "EmuChef.dmg"), "dmg");
    assert.deepEqual(discoverArtifacts(root), {
      appPath: path.join(root, "macos", "EmuChef.app"),
      dmgPath: path.join(root, "dmg", "EmuChef.dmg"),
    });
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
