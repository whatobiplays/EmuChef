import assert from "node:assert/strict";
import test from "node:test";

import {
  architectureMatches,
  forbiddenPathReasons,
  forbiddenStringReasons,
  parseAppPath,
  resolveBundleLayout,
  signingState,
  validateInfoPlist,
} from "./check-macos-bundle.mjs";

test("requires exactly one app path", () => {
  assert.throws(() => parseAppPath([]), /usage/);
  assert.throws(() => parseAppPath(["one", "two"]), /usage/);
  assert.match(parseAppPath(["Example.app"]), /Example\.app$/);
});

test("resolves executable paths from Info.plist", () => {
  const layout = resolveBundleLayout("/tmp/Example.app", {
    CFBundleExecutable: "example-main",
  });
  assert.equal(layout.mainPath, "/tmp/Example.app/Contents/MacOS/example-main");
  assert.equal(layout.sidecarPath, "/tmp/Example.app/Contents/MacOS/emuchef");
  assert.throws(() => resolveBundleLayout("/tmp/Example", {}), /end in .app/);
  assert.throws(
    () => resolveBundleLayout("/tmp/Example.app", {}),
    /CFBundleExecutable/,
  );
});

test("validates stable Info.plist identity", () => {
  const expected = {
    identifier: "com.example.app",
    productName: "Example",
    version: "1.2.3",
  };
  const info = {
    CFBundleDisplayName: "Example",
    CFBundleIdentifier: "com.example.app",
    CFBundlePackageType: "APPL",
    CFBundleShortVersionString: "1.2.3",
    CFBundleVersion: "1.2.3",
  };
  assert.doesNotThrow(() => validateInfoPlist(info, expected));
  assert.throws(
    () => validateInfoPlist({ ...info, CFBundleIdentifier: "wrong" }, expected),
    /CFBundleIdentifier/,
  );
});

test("matches thin and universal host architectures", () => {
  assert.equal(architectureMatches("Mach-O 64-bit executable arm64", "arm64"), true);
  assert.equal(architectureMatches("universal binary x86_64 arm64", "arm64"), true);
  assert.equal(architectureMatches("Mach-O 64-bit executable arm64", "x86_64"), false);
});

test("rejects Python, legacy, shadow, and development-server remnants", () => {
  assert.deepEqual(forbiddenPathReasons(["Contents/MacOS/emuchef"]), []);
  assert.equal(forbiddenPathReasons(["Contents/Resources/tool.py"]).length, 1);
  assert.equal(forbiddenPathReasons(["Contents/Frameworks/Python.framework/x"]).length, 1);
  assert.equal(forbiddenPathReasons(["Contents/MacOS/emuchef-plan-shadow"]).length, 1);
  assert.equal(forbiddenStringReasons("main", "http://localhost:5173").length, 1);
  assert.equal(forbiddenStringReasons("main", "libpython3.12.dylib").length, 1);
  assert.deepEqual(forbiddenStringReasons("main", "http://ipc.localhost"), []);
});

test("accepts only unsigned or ad-hoc local signing", () => {
  assert.equal(signingState("Signature=adhoc", 0), "ad-hoc");
  assert.equal(signingState("code object is not signed at all", 1), "unsigned");
  assert.throws(() => signingState("Authority=Developer ID Application", 0), /unsigned or ad-hoc/);
});

