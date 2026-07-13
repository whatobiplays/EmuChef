import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  appTreeDigest,
  generateMacosReleaseManifest,
  parseManifestArguments,
  resolveBuildCommit,
  sha256File,
} from "./generate-macos-release-manifest.mjs";

const SHA = "93F816FC1EA59CD034A40432E4E2A269E11EEAD7";

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-manifest-"));
  const appPath = path.join(root, "Example.app");
  const macos = path.join(appPath, "Contents", "MacOS");
  fs.mkdirSync(macos, { recursive: true });
  fs.writeFileSync(path.join(appPath, "Contents", "Info.plist"), "plist-test-data");
  fs.writeFileSync(path.join(macos, "example-main"), "main-bytes");
  fs.writeFileSync(path.join(macos, "emuchef"), "sidecar-bytes");
  const dmgPath = path.join(root, "Example.dmg");
  fs.writeFileSync(dmgPath, "dmg-bytes");
  const outputPath = path.join(root, "release-artifacts", "manifest.json");
  const run = (command, args) => {
    if (command === "git" && args[0] === "cat-file") {
      return { status: 0, stdout: "" };
    }
    if (command === "plutil") {
      return {
        status: 0,
        stdout: JSON.stringify({
          CFBundleDisplayName: "Example",
          CFBundleExecutable: "example-main",
          CFBundleIdentifier: "com.example.app",
          CFBundleShortVersionString: "1.2.3",
        }),
      };
    }
    if (command === "uname") {
      return { status: 0, stdout: "arm64\n" };
    }
    throw new Error(`unexpected command ${command}`);
  };
  const verify = () => ({
    app: { signed: true, notarized: true, stapled: true, gatekeeperAccepted: true },
    dmg: { signed: true, notarized: true, stapled: true, gatekeeperAccepted: true },
  });
  return { appPath, buildCommit: SHA, dmgPath, outputPath, root, run, verify };
}

test("parses positional paths and a full build SHA", () => {
  const parsed = parseManifestArguments([
    "Example.app",
    "Example.dmg",
    "manifest.json",
    "--build-commit",
    SHA,
  ]);
  assert.equal(parsed.buildCommit, SHA);
  assert.throws(() => parseManifestArguments(["a", "b", "c", "--build-commit", "abc"]), /40/);
  assert.throws(() => parseManifestArguments(["a", "b"]), /usage/);
  assert.throws(() => parseManifestArguments(["a", "b", "c", "--unknown"]), /unknown/);
});

test("requires overrides to resolve locally with git cat-file", () => {
  const calls = [];
  const run = (command, args) => {
    calls.push([command, ...args]);
    return { status: 0, stdout: "" };
  };
  assert.equal(
    resolveBuildCommit({ buildCommit: SHA, repositoryRoot: "/repo", run }),
    SHA.toLowerCase(),
  );
  assert.deepEqual(calls[0], ["git", "cat-file", "-e", `${SHA}^{commit}`]);
  assert.throws(
    () =>
      resolveBuildCommit({
        buildCommit: SHA,
        repositoryRoot: "/repo",
        run: () => ({ status: 1, stderr: "not found" }),
      }),
    /local commit/,
  );
});

test("uses full HEAD only for a clean tracked worktree", () => {
  const cleanRun = (_command, args) => {
    if (args[0] === "status") return { status: 0, stdout: "" };
    return { status: 0, stdout: `${SHA}\n` };
  };
  assert.equal(
    resolveBuildCommit({ repositoryRoot: "/repo", run: cleanRun }),
    SHA.toLowerCase(),
  );
  assert.throws(
    () =>
      resolveBuildCommit({
        repositoryRoot: "/repo",
        run: (_command, args) =>
          args[0] === "status" ? { status: 0, stdout: " M tracked" } : { status: 0 },
      }),
    /clean tracked worktree/,
  );
});

test("hashes file contents and canonical sorted app-tree records", (t) => {
  const value = fixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  const first = appTreeDigest(value.appPath);
  const second = appTreeDigest(value.appPath);
  assert.equal(first, second);
  assert.match(first, /^[0-9a-f]{64}$/);
  assert.notEqual(sha256File(value.dmgPath), first);
});

test("sorts canonical app-tree records by normalized UTF-8 path bytes", () => {
  const appPath = path.join(path.sep, "virtual", "Canonical.app");
  const orderingDirectory = path.join(appPath, "ordering");
  const filenames = ["é", "a", "_", "A", "-"];
  const fileContents = new Map(
    filenames.map((name) => [path.join(orderingDirectory, name), Buffer.from(`contents:${name}`)]),
  );
  const mockFs = {
    readdirSync(directory) {
      if (directory === appPath) {
        return [{ name: "ordering", isDirectory: () => true, isFile: () => false }];
      }
      if (directory === orderingDirectory) {
        return filenames.map((name) => ({
          name,
          isDirectory: () => false,
          isFile: () => true,
        }));
      }
      throw new Error("unexpected mock directory");
    },
    readFileSync(filePath) {
      const contents = fileContents.get(filePath);
      if (contents === undefined) throw new Error("unexpected mock file");
      return contents;
    },
  };

  const orderedRelativePaths = [...fileContents.keys()]
    .map((filePath) => path.relative(appPath, filePath).split(path.sep).join("/"))
    .sort((left, right) => Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8")));
  assert.deepEqual(orderedRelativePaths, [
    "ordering/-",
    "ordering/A",
    "ordering/_",
    "ordering/a",
    "ordering/é",
  ]);

  const canonicalRecords = orderedRelativePaths
    .map((relativePath) => {
      const contentDigest = crypto
        .createHash("sha256")
        .update(fileContents.get(path.join(appPath, relativePath)))
        .digest("hex");
      return `${contentDigest}  ${relativePath}\n`;
    })
    .join("");
  const expectedDigest = crypto.createHash("sha256").update(canonicalRecords).digest("hex");
  assert.equal(appTreeDigest(appPath, mockFs), expectedDigest);
});

test("app-tree ordering does not use locale collation", () => {
  const sourcePath = fileURLToPath(new URL("./generate-macos-release-manifest.mjs", import.meta.url));
  const source = fs.readFileSync(sourcePath, "utf8");
  const start = source.indexOf("export function appTreeDigest");
  const end = source.indexOf("\n}\n\nfunction runSafe", start);
  assert.notEqual(start, -1);
  assert.notEqual(end, -1);
  const implementation = source.slice(start, end);
  assert.doesNotMatch(implementation, /localeCompare/);
  assert.match(implementation, /Buffer\.compare/);
});

test("writes the safe schema only after signed verification passes", (t) => {
  const value = fixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  const manifest = generateMacosReleaseManifest(value, {
    now: () => new Date("2026-07-12T00:00:00.000Z"),
    repositoryRoot: value.root,
    run: value.run,
    verify: value.verify,
  });
  assert.equal(manifest.schemaVersion, 1);
  assert.equal(manifest.buildCommitSha, SHA.toLowerCase());
  assert.equal(manifest.app.name, "Example.app");
  assert.equal(manifest.dmg.name, "Example.dmg");
  assert.equal(manifest.verification.notarizationPassed, true);
  assert.equal(manifest.generatedAtUtc, "2026-07-12T00:00:00.000Z");
  const written = fs.readFileSync(value.outputPath, "utf8");
  assert.doesNotMatch(written, new RegExp(value.root.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.doesNotMatch(written, /TeamIdentifier|APPLE_API_|certificateSubject/);
});

test("does not create output when signed verification fails", (t) => {
  const value = fixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  assert.throws(
    () =>
      generateMacosReleaseManifest(value, {
        repositoryRoot: value.root,
        run: value.run,
        verify: () => {
          throw new Error("signature verification failed");
        },
      }),
    /signature verification failed/,
  );
  assert.equal(fs.existsSync(value.outputPath), false);
});
