import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import {
  REPOSITORY_ROOT,
  SCRIPT_PATH,
  assertExists,
  assertRemoved,
  captureCleanupOutput,
  collectInventory,
  createFixture,
  git,
  gitFailurePath,
  make,
  outputOf,
  removeEligible,
  run,
  snapshotTree,
  withUnlinkFailure,
  writeFile,
} from "./worktree-cleanup-test-fixtures.mjs";

test("removeEligible reports an unlink failure and leaves the Cargo candidate intact", (t) => {
  const { root } = createFixture(t);
  const candidate =
    "crates/emuchef-rust-backend/target/debug/deps/libunlink-error-0123456789abcdef.rlib";
  writeFile(root, candidate);
  const inventory = collectInventory(root, false);

  const removed = withUnlinkFailure(path.join(root, candidate), () => removeEligible(inventory));

  assert.deepEqual(removed, []);
  assertExists(root, candidate);
  assert.ok(inventory.errors.some((error) => error.includes("cannot remove file (EACCES)")));
});


test("removeEligible reports a candidate whose type changes before deletion", (t) => {
  const { root } = createFixture(t);
  const candidate =
    "crates/emuchef-rust-backend/target/debug/deps/libchanged-0123456789abcdef.rlib";
  const candidatePath = writeFile(root, candidate);
  const inventory = collectInventory(root, false);

  fs.rmSync(candidatePath);
  fs.mkdirSync(candidatePath);

  assert.deepEqual(removeEligible(inventory), []);
  assert.deepEqual(inventory.errors, [`${candidate}: candidate changed type before deletion`]);
  assert.ok(fs.lstatSync(candidatePath).isDirectory());
});


test("removeEligible keeps a dependency tree when its install stamp cannot be removed", (t) => {
  const { root } = createFixture(t);
  const stamp = "apps/emuchef-app/node_modules/.emuchef-deps-stamp";
  const packageFile = "apps/emuchef-app/node_modules/pkg/package.json";
  writeFile(root, stamp);
  writeFile(root, packageFile, '{"name":"fixture-package"}\n');
  const inventory = collectInventory(root, true);

  const removed = withUnlinkFailure(path.join(root, stamp), () => removeEligible(inventory));

  assert.deepEqual(removed, []);
  assertExists(root, stamp);
  assertExists(root, packageFile);
  assert.ok(inventory.errors.some((error) => error.includes("cannot remove file (EACCES)")));
});


test("clean recognizes Cargo cross-target output only inside a Cargo profile shape", (t) => {
  const { root } = createFixture(t);
  const targetRoot = "crates/emuchef-rust-backend/target";
  const legitimate = `${targetRoot}/aarch64-unknown-linux-gnu/debug/deps/libtarget-0123456789abcdef.rlib`;
  const operatorNotes = `${targetRoot}/operator-notes-archive/deps/libnotes-0123456789abcdef.rlib`;
  const nestedOperatorNotes = `${targetRoot}/debug/operator-notes-archive/deps/libnotes-1123456789abcdef.rlib`;
  const unrecognizedProfile = `${targetRoot}/wasm32-unknown-unknown/debug/operator-notes.txt`;
  for (const relativePath of [legitimate, operatorNotes, nestedOperatorNotes, unrecognizedProfile]) {
    writeFile(root, relativePath, `cargo fixture: ${relativePath}\n`);
  }

  const result = make(root, "clean");
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  assertRemoved(root, legitimate);
  for (const relativePath of [operatorNotes, nestedOperatorNotes, unrecognizedProfile]) {
    assertExists(root, relativePath);
  }
});


test(
  "clean-deep leaves one dependency root intact when its install stamp cannot be removed",
  { skip: process.platform === "win32" || process.getuid?.() === 0 },
  (t) => {
    const { root } = createFixture(t);
    const appDependencies = "apps/emuchef-app/node_modules";
    const appStamp = `${appDependencies}/.emuchef-deps-stamp`;
    const appPackage = `${appDependencies}/pkg/package.json`;
    const editorStamp = "apps/config-editor/node_modules/.emuchef-deps-stamp";
    const editorPackage = "apps/config-editor/node_modules/pkg/package.json";
    for (const relativePath of [appStamp, appPackage, editorStamp, editorPackage]) {
      writeFile(root, relativePath, `dependency: ${relativePath}\n`);
    }
    fs.chmodSync(path.join(root, appDependencies), 0o555);

    let result;
    try {
      result = make(root, "clean-deep");
    } finally {
      fs.chmodSync(path.join(root, appDependencies), 0o755);
    }
    const output = outputOf(result);

    assert.notEqual(result.status, 0, output);
    assert.match(output, /cannot remove file \((?:EACCES|EPERM)\)/i);
    assertExists(root, appStamp);
    assertExists(root, appPackage);
    assertRemoved(root, editorStamp);
    assertRemoved(root, editorPackage);
  },
);


test(
  "cleanup reports directory-removal permission failures",
  { skip: process.platform === "win32" || process.getuid?.() === 0 },
  (t) => {
    const { root } = createFixture(t);
    const profileRoot = "crates/emuchef-rust-backend/target/debug";
    const emptyCargoDirectory = `${profileRoot}/deps`;
    fs.mkdirSync(path.join(root, emptyCargoDirectory), { recursive: true });
    fs.chmodSync(path.join(root, profileRoot), 0o555);

    let result;
    try {
      result = make(root, "clean");
    } finally {
      fs.chmodSync(path.join(root, profileRoot), 0o755);
    }
    const output = outputOf(result);

    assert.notEqual(result.status, 0, output);
    assert.match(output, /cannot remove empty directory \((?:EACCES|EPERM)\)/i);
    assert.ok(fs.statSync(path.join(root, emptyCargoDirectory)).isDirectory());
  },
);


test("default cleanup skips symlink roots and candidates without following them", (t) => {
  const { sandbox, root } = createFixture(t);
  const external = path.join(sandbox, "outside-target");
  writeFile(external, "debug/build/canary-0123456789abcdef/out/keep.txt", "outside data\n");
  const externalLeaf = writeFile(external, "debug/deps/libexternal-0123456789abcdef.rlib", "external leaf\n");
  const symlinkedTarget = path.join(root, "apps/config-editor/src-tauri/target");
  fs.mkdirSync(path.dirname(symlinkedTarget), { recursive: true });
  fs.symlinkSync(external, symlinkedTarget, "dir");
  writeFile(root, "crates/emuchef-rust-backend/target/debug/deps/libsafe-0123456789abcdef.rlib");
  const symlinkedArtifact = path.join(
    root,
    "apps/emuchef-app/src-tauri/target/debug/deps/emuchef-0123456789abcdef.d",
  );
  fs.mkdirSync(path.dirname(symlinkedArtifact), { recursive: true });
  fs.symlinkSync(externalLeaf, symlinkedArtifact);
  const outsideBefore = snapshotTree(external);

  const result = make(root, "clean");
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  assert.ok(output.includes("SKIP apps/config-editor/src-tauri/target: symbolic link cleanup candidate skipped"));
  assert.ok(
    output.includes(
      "SKIP apps/emuchef-app/src-tauri/target/debug/deps/emuchef-0123456789abcdef.d: symbolic link cleanup candidate skipped",
    ),
  );
  assertRemoved(root, "crates/emuchef-rust-backend/target/debug/deps/libsafe-0123456789abcdef.rlib");
  assert.equal(fs.lstatSync(symlinkedTarget).isSymbolicLink(), true);
  assert.equal(fs.lstatSync(symlinkedArtifact).isSymbolicLink(), true);
  assert.deepEqual(snapshotTree(external), outsideBefore);
});


test("path traversal arguments, wrong-root invocation, and a symlinked worktree root are rejected", (t) => {
  const { sandbox, root } = createFixture(t);
  writeFile(root, "crates/emuchef-rust-backend/target/debug/deps/libguard-0123456789abcdef.rlib");
  const before = snapshotTree(root);

  const traversal = run(process.execPath, [SCRIPT_PATH, "default", "../../outside"], root);
  assert.notEqual(traversal.status, 0);
  assert.deepEqual(snapshotTree(root), before);

  const wrongRoot = path.join(sandbox, "wrong-root");
  fs.mkdirSync(wrongRoot);
  git(wrongRoot, ["init", "-q"]);
  const wrongRootBefore = snapshotTree(wrongRoot);
  const wrongRootResult = run("make", ["-f", path.join(REPOSITORY_ROOT, "Makefile"), "clean"], wrongRoot);
  assert.notEqual(wrongRootResult.status, 0, outputOf(wrongRootResult));
  assert.match(outputOf(wrongRootResult), /worktree root|Git worktree/i);
  assert.deepEqual(snapshotTree(root), before);
  assert.deepEqual(snapshotTree(wrongRoot), wrongRootBefore);

  const symlinkRoot = path.join(sandbox, "checkout-link");
  fs.symlinkSync(root, symlinkRoot, "dir");
  const symlinkResult = run("make", ["-C", symlinkRoot, "clean"], symlinkRoot);
  assert.notEqual(symlinkResult.status, 0, outputOf(symlinkResult));
  assert.deepEqual(snapshotTree(root), before);
});


test("partial filesystem errors are reported after other eligible files are removed", (t) => {
  const { root } = createFixture(t);
  const malformed = "crates/emuchef-rust-backend/target/.rustc_info.json";
  fs.mkdirSync(path.join(root, malformed), { recursive: true });
  writeFile(root, `${malformed}/unexpected-child.json`, "unexpected node type\n");
  const eligible =
    "apps/emuchef-app/src-tauri/target/debug/deps/libpartial-0123456789abcdef.rlib";
  writeFile(root, eligible, "eligible output\n");

  const result = make(root, "clean");
  const output = outputOf(result);

  assert.notEqual(result.status, 0, output);
  assertRemoved(root, eligible);
  assertExists(root, `${malformed}/unexpected-child.json`);
  assert.match(output, /ERROR/i);
  assert.match(output, /partial|failed/i);
});


test("clean-preview reports inventory errors without changing a malformed generated root", (t) => {
  const { root } = createFixture(t);
  const malformedRoot = "apps/emuchef-app/src-tauri/gen";
  const eligibleArtifact =
    "crates/emuchef-rust-backend/target/debug/deps/libpartial-preview-0123456789abcdef.rlib";
  writeFile(root, malformedRoot, "generated root must be a directory\n");
  writeFile(root, eligibleArtifact, "eligible but preview-only\n");
  const before = snapshotTree(root);

  const result = make(root, "clean-preview");
  const output = outputOf(result);

  assert.notEqual(result.status, 0, output);
  assert.match(output, /ERROR .*expected a directory/i);
  assert.match(output, /libpartial-preview-0123456789abcdef\.rlib/);
  assert.match(output, /no files changed/i);
  assert.deepEqual(snapshotTree(root), before);
});


test("clean fails closed when tracked-path inventory is unavailable", (t) => {
  const { root } = createFixture(t);
  const eligible =
    "crates/emuchef-rust-backend/target/debug/deps/libtracked-error-0123456789abcdef.rlib";
  writeFile(root, eligible, "must remain when tracked paths cannot be read\n");
  const before = snapshotTree(root);

  const result = make(root, "clean", { PATH: gitFailurePath(root) });
  const output = outputOf(result);

  assert.notEqual(result.status, 0, output);
  assert.match(output, /cannot read tracked paths/i);
  assert.doesNotMatch(output, /libtracked-error-0123456789abcdef\.rlib/);
  assert.match(output, /no files changed/i);
  assert.deepEqual(snapshotTree(root), before);
});


test("clean-preview fails when tracked-path inventory is unavailable without changing files", (t) => {
  const { root } = createFixture(t);
  const eligible =
    "crates/emuchef-rust-backend/target/debug/deps/libtracked-preview-0123456789abcdef.rlib";
  writeFile(root, eligible, "preview requires a complete tracked inventory\n");
  const before = snapshotTree(root);

  const result = make(root, "clean-preview", { PATH: gitFailurePath(root) });
  const output = outputOf(result);

  assert.notEqual(result.status, 0, output);
  assert.match(output, /cannot read tracked paths/i);
  assert.doesNotMatch(output, /libtracked-preview-0123456789abcdef\.rlib/);
  assert.match(output, /no files changed/i);
  assert.deepEqual(snapshotTree(root), before);
});


test("cleanup rejects a PWD value that does not match its working directory", (t) => {
  const { sandbox, root } = createFixture(t);
  writeFile(root, "crates/emuchef-rust-backend/target/debug/deps/libpwd-0123456789abcdef.rlib");
  const mismatchedDirectory = path.join(sandbox, "ordinary-mismatched-directory");
  fs.mkdirSync(mismatchedDirectory);
  assert.equal(fs.lstatSync(mismatchedDirectory).isSymbolicLink(), false);
  const before = snapshotTree(root);

  const result = run(
    process.execPath,
    [SCRIPT_PATH, "preview"],
    root,
    { PWD: mismatchedDirectory },
  );
  const output = outputOf(result);

  assert.notEqual(result.status, 0, output);
  assert.match(output, /PWD.*working directory|working directory.*PWD/i);
  assert.match(output, /no files changed/i);
  assert.deepEqual(snapshotTree(root), before);
});


test("cleanup rejects a non-Git working directory", (t) => {
  const { sandbox } = createFixture(t);
  const nonGitRoot = path.join(sandbox, "not-a-git-checkout");
  fs.mkdirSync(nonGitRoot);
  writeFile(nonGitRoot, "candidate.txt", "must remain\n");
  const before = snapshotTree(nonGitRoot);

  const result = run(process.execPath, [SCRIPT_PATH, "preview"], nonGitRoot);
  const output = outputOf(result);

  assert.notEqual(result.status, 0, output);
  assert.match(output, /Git worktree|Git checkout|worktree root/i);
  assert.match(output, /no files changed/i);
  assert.deepEqual(snapshotTree(nonGitRoot), before);
});


test("cleanup CLI rejects an invalid mode and incorrect argument counts", (t) => {
  const { root } = createFixture(t);
  const before = snapshotTree(root);
  const invalidCalls = [
    { args: ["unsupported"], message: /expected preview, default, or deep mode/i },
    { args: [], message: /expected one mode and no path arguments/i },
    { args: ["preview", "extra"], message: /expected one mode and no path arguments/i },
  ];

  for (const { args, message } of invalidCalls) {
    const result = run(process.execPath, [SCRIPT_PATH, ...args], root);
    const output = outputOf(result);
    assert.notEqual(result.status, 0, output);
    assert.match(output, message);
    assert.deepEqual(snapshotTree(root), before);
  }
});


test("cleanup rejects a symlink in the generated Tauri schema tree", (t) => {
  const { root } = createFixture(t);
  const externalTarget = writeFile(root, "protected/generated-schema.json", "external schema\n");
  const symlink = path.join(root, "apps/emuchef-app/src-tauri/gen/schemas/desktop-schema.json");
  fs.mkdirSync(path.dirname(symlink), { recursive: true });
  fs.symlinkSync(externalTarget, symlink);
  const before = snapshotTree(root);

  const result = make(root, "clean-preview");
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  assert.match(output, /SKIP .*desktop-schema\.json: symbolic link cleanup candidate skipped/i);
  assert.deepEqual(snapshotTree(root), before);
});


test("clean-deep skips package and executable symlinks without following them", (t) => {
  const { sandbox, root } = createFixture(t);
  const externalPackageFile = writeFile(sandbox, "outside-package/package.json", '{"name":"external"}\n');
  const externalPackage = path.dirname(externalPackageFile);
  const externalBin = writeFile(sandbox, "outside-bin/emuchef", "external executable\n");
  const externalData = writeFile(sandbox, "outside-data.txt", "external deep output\n");
  const packageSymlink = "apps/emuchef-app/node_modules/pkg";
  const binSymlink = "apps/emuchef-app/node_modules/.bin/emuchef";
  const otherSymlinks = [
    "apps/emuchef-app/dist/index.html",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin",
  ];
  const symlinks = [
    [packageSymlink, externalPackage, "dir"],
    [binSymlink, externalBin, undefined],
    ...otherSymlinks.map((relativePath) => [relativePath, externalData, undefined]),
  ];
  for (const [relativePath, target, type] of symlinks) {
    const absolutePath = path.join(root, relativePath);
    fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
    fs.symlinkSync(target, absolutePath, type);
  }
  const externalPackageBefore = snapshotTree(externalPackage);
  const externalBinBefore = fs.readFileSync(externalBin);
  const externalDataBefore = fs.readFileSync(externalData);

  const result = make(root, "clean-deep");
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  for (const relativePath of [packageSymlink, binSymlink, ...otherSymlinks]) {
    assert.ok(output.includes(`SKIP ${relativePath}: symbolic link cleanup candidate skipped`));
    assert.equal(fs.lstatSync(path.join(root, relativePath)).isSymbolicLink(), true);
  }
  assert.deepEqual(snapshotTree(externalPackage), externalPackageBefore);
  assert.deepEqual(fs.readFileSync(externalBin), externalBinBefore);
  assert.deepEqual(fs.readFileSync(externalData), externalDataBefore);
});
