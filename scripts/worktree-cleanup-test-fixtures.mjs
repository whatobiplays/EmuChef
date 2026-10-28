import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  collectInventory,
  isProtectedReleaseArtifact,
  main as cleanupMain,
  printInventory,
  printRemovalSummary,
  removeEligible,
  validRelativePath,
  validSidecarFilename,
  validSidecarMetadataFilename,
} from "./worktree-cleanup.mjs";

const REPOSITORY_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const SCRIPT_PATH = path.join(REPOSITORY_ROOT, "scripts", "worktree-cleanup.mjs");
const FIXTURE_GITIGNORE = [
  "/crates/emuchef-rust-backend/target/",
  "/apps/emuchef-app/src-tauri/target/",
  "/apps/config-editor/src-tauri/target/",
  "/apps/emuchef-app/node_modules/",
  "/apps/config-editor/node_modules/",
  "/apps/emuchef-app/dist/",
  "/apps/config-editor/dist/",
  "/apps/emuchef-app/src-tauri/binaries/*",
  "!/apps/emuchef-app/src-tauri/binaries/.gitignore",
  "/apps/config-editor/src-tauri/binaries/*",
  "!/apps/config-editor/src-tauri/binaries/.gitignore",
].join("\n");
const DURABLE_STATE_CANNARIES = [
  ".env",
  ".env.local",
  "apps/emuchef-app/.env.production.local",
  "config/local.toml",
  ".config/emuchef/settings.json",
  "data/development.sqlite",
  ".emuchef_runtime/state.json",
];

function run(command, args, cwd, env = {}) {
  const result = spawnSync(command, args, {
    cwd,
    env: { ...process.env, PWD: cwd, ...env },
    encoding: "utf8",
  });
  if (result.error) throw result.error;
  return result;
}

function git(root, args) {
  const result = run("git", ["-C", root, ...args], root);
  assert.equal(result.status, 0, result.stderr || result.stdout);
  return result.stdout.trim();
}

function writeFile(root, relativePath, contents = "fixture data\n") {
  const absolutePath = path.join(root, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  fs.writeFileSync(absolutePath, contents);
  return absolutePath;
}

function makeFixtureDirectoriesRemovable(directory) {
  const stat = fs.lstatSync(directory);
  if (stat.isSymbolicLink() || !stat.isDirectory()) return;

  fs.chmodSync(directory, stat.mode | 0o700);
  for (const name of fs.readdirSync(directory)) {
    makeFixtureDirectoriesRemovable(path.join(directory, name));
  }
}

function removeFixtureTree(directory) {
  try {
    makeFixtureDirectoriesRemovable(directory);
  } catch (error) {
    if (error.code === "ENOENT") return;
    throw error;
  }
  fs.rmSync(directory, {
    recursive: true,
    force: true,
    maxRetries: 5,
    retryDelay: 100,
  });
}

function createFixture(t) {
  const sandboxParent = fs.realpathSync(os.tmpdir());
  const sandbox = fs.mkdtempSync(path.join(sandboxParent, "emuchef-cleanup-fixture-"));
  t.after(() => removeFixtureTree(sandbox));
  const root = path.join(sandbox, "checkout");
  fs.mkdirSync(root);
  fs.mkdirSync(path.join(root, "scripts"));
  fs.copyFileSync(path.join(REPOSITORY_ROOT, "Makefile"), path.join(root, "Makefile"));
  fs.copyFileSync(SCRIPT_PATH, path.join(root, "scripts", "worktree-cleanup.mjs"));
  fs.copyFileSync(
    path.join(REPOSITORY_ROOT, "scripts", "sidecar-target-contract.mjs"),
    path.join(root, "scripts", "sidecar-target-contract.mjs"),
  );
  fs.mkdirSync(path.join(root, "tools"), { recursive: true });
  fs.copyFileSync(
    path.join(REPOSITORY_ROOT, "tools", "cargo-test-freshness.mjs"),
    path.join(root, "tools", "cargo-test-freshness.mjs"),
  );
  writeFile(root, ".gitignore", `${FIXTURE_GITIGNORE}\n`);
  writeFile(
    root,
    "apps/emuchef-app/src-tauri/binaries/.gitignore",
    "*\n!.gitignore\n",
  );
  writeFile(
    root,
    "apps/config-editor/src-tauri/binaries/.gitignore",
    "*\n!.gitignore\n",
  );

  git(root, ["init", "-q"]);
  git(root, ["config", "user.name", "Cleanup Fixture"]);
  git(root, ["config", "user.email", "cleanup-fixture@example.invalid"]);
  git(root, ["add", "Makefile", ".gitignore", "scripts", "apps"]);
  git(root, ["commit", "-qm", "Create disposable cleanup fixture"]);
  return { sandbox, root };
}

function forceTrack(root, relativePath) {
  git(root, ["add", "-f", "--", relativePath]);
  git(root, ["commit", "-qm", "Track fixture preservation canary"]);
}

function make(root, target, env = {}) {
  return run("make", [target], root, env);
}

function gitFailurePath(root) {
  const realGit = process.env.PATH.split(path.delimiter)
    .map((directory) => path.join(directory, "git"))
    .find((candidate) => {
      try {
        return fs.statSync(candidate).isFile() && (fs.statSync(candidate).mode & 0o111) !== 0;
      } catch {
        return false;
      }
    });
  assert.ok(realGit, "a real Git executable should be available to the fixture");

  const shimDirectory = path.join(path.dirname(root), "test-shims");
  fs.mkdirSync(shimDirectory, { recursive: true });
  const shimPath = path.join(shimDirectory, "git");
  const quotedGitPath = `'${realGit.replaceAll("'", "'\\''")}'`;
  fs.writeFileSync(
    shimPath,
    `#!/bin/sh
case " $* " in
  *" rev-parse "*) exec ${quotedGitPath} "$@" ;;
  *) printf '%s\\n' 'simulated tracked inventory failure' >&2; exit 73 ;;
esac
`,
  );
  fs.chmodSync(shimPath, 0o755);
  return `${shimDirectory}${path.delimiter}${process.env.PATH}`;
}

function outputOf(result) {
  return `${result.stdout}\n${result.stderr}`;
}

function captureCleanupOutput(callback) {
  let stdout = "";
  let stderr = "";
  const originalStdoutWrite = process.stdout.write;
  const originalStderrWrite = process.stderr.write;
  process.stdout.write = (chunk) => {
    stdout += String(chunk);
    return true;
  };
  process.stderr.write = (chunk) => {
    stderr += String(chunk);
    return true;
  };
  try {
    return { value: callback(), stdout, stderr };
  } finally {
    process.stdout.write = originalStdoutWrite;
    process.stderr.write = originalStderrWrite;
  }
}

function invokeCleanupMain(args) {
  const originalArgv = process.argv;
  try {
    process.argv = [originalArgv[0], SCRIPT_PATH, ...args];
    return captureCleanupOutput(cleanupMain);
  } finally {
    process.argv = originalArgv;
  }
}

function snapshotTree(root) {
  const entries = [];
  const visit = (directory, relativeDirectory = "") => {
    for (const name of fs.readdirSync(directory).sort()) {
      if (!relativeDirectory && name === ".git") continue;
      const absolutePath = path.join(directory, name);
      const relativePath = path.posix.join(relativeDirectory, name);
      const stat = fs.lstatSync(absolutePath);
      if (stat.isSymbolicLink()) {
        entries.push(`${relativePath}:symlink:${fs.readlinkSync(absolutePath)}`);
      } else if (stat.isDirectory()) {
        entries.push(`${relativePath}:directory`);
        visit(absolutePath, relativePath);
      } else if (stat.isFile()) {
        const digest = crypto
          .createHash("sha256")
          .update(fs.readFileSync(absolutePath))
          .digest("hex");
        entries.push(`${relativePath}:file:${digest}`);
      } else {
        entries.push(`${relativePath}:other`);
      }
    }
  };
  visit(root);
  return entries.sort();
}

function assertExists(root, relativePath) {
  assert.ok(fs.existsSync(path.join(root, relativePath)), `${relativePath} should remain`);
}

function assertRemoved(root, relativePath) {
  assert.equal(fs.existsSync(path.join(root, relativePath)), false, `${relativePath} should be removed`);
}

/** Runs an action while unlinking one fixture file fails with an access error. */
function withUnlinkFailure(targetPath, action) {
  const originalUnlinkSync = fs.unlinkSync;
  fs.unlinkSync = (candidatePath, ...args) => {
    if (path.resolve(candidatePath) === path.resolve(targetPath)) {
      const error = new Error("fixture unlink denied");
      error.code = "EACCES";
      throw error;
    }
    return originalUnlinkSync(candidatePath, ...args);
  };
  try {
    return action();
  } finally {
    fs.unlinkSync = originalUnlinkSync;
  }
}

function snapshotFiles(root, relativePaths) {
  return relativePaths.map((relativePath) => {
    const absolutePath = path.join(root, relativePath);
    const stat = fs.lstatSync(absolutePath);
    assert.ok(stat.isFile(), `${relativePath} should remain a regular file`);
    const digest = crypto
      .createHash("sha256")
      .update(fs.readFileSync(absolutePath))
      .digest("hex");
    return [relativePath, digest];
  });
}


export {
  DURABLE_STATE_CANNARIES,
  FIXTURE_GITIGNORE,
  REPOSITORY_ROOT,
  SCRIPT_PATH,
  assertExists,
  assertRemoved,
  captureCleanupOutput,
  createFixture,
  forceTrack,
  git,
  gitFailurePath,
  invokeCleanupMain,
  make,
  outputOf,
  run,
  snapshotFiles,
  snapshotTree,
  withUnlinkFailure,
  writeFile,
  collectInventory,
  isProtectedReleaseArtifact,
  cleanupMain,
  printInventory,
  printRemovalSummary,
  removeEligible,
  validRelativePath,
  validSidecarFilename,
  validSidecarMetadataFilename,
};
