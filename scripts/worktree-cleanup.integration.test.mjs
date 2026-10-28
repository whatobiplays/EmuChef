import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import {
  DURABLE_STATE_CANNARIES,
  REPOSITORY_ROOT,
  assertExists,
  assertRemoved,
  createFixture,
  forceTrack,
  git,
  make,
  outputOf,
  run,
  snapshotFiles,
  snapshotTree,
  writeFile,
} from "./worktree-cleanup-test-fixtures.mjs";

test("make clean removes known Cargo and Tauri generated outputs and preserves durable files", (t) => {
  const { sandbox, root } = createFixture(t);
  const generated = [
    "crates/emuchef-rust-backend/target/CACHEDIR.TAG",
    "crates/emuchef-rust-backend/target/.rustc_info.json",
    "crates/emuchef-rust-backend/target/debug/build/backend-0123456789abcdef/out/build-output.txt",
    "crates/emuchef-rust-backend/target/debug/deps/libbackend-0123456789abcdef.rlib",
    "crates/emuchef-rust-backend/target/debug/incremental/backend-3l9cz3al16vr5/s-hn0lkk6b83-0t4n74k-br4pfaeeq8b6q27a5frw5n3u3/query-cache.bin",
    "crates/emuchef-rust-backend/target/debug/incremental/backend-3l9cz3al16vr5/s-hn0lkk6b83-0t4n74k.lock",
    "crates/emuchef-rust-backend/target/debug/.fingerprint/backend-0123456789abcdef/lib-backend.json",
    "crates/emuchef-rust-backend/target/release/build/backend-0123456789abcdef/out/release-build-output.txt",
    "crates/emuchef-rust-backend/target/aarch64-apple-darwin/debug/deps/libcross-0123456789abcdef.rlib",
    "apps/emuchef-app/src-tauri/target/debug/deps/emuchef_app-0123456789abcdef.d",
    "apps/config-editor/src-tauri/target/debug/build/config_editor-0123456789abcdef/out/config.rs",
    "apps/config-editor/src-tauri/target/debug/incremental/config_editor-1q2khge99p6my/s-hn0lkk6b83-0t4n74k-br4pfaeeq8b6q27a5frw5n3u3/query-cache.bin",
    "apps/config-editor/src-tauri/target/debug/incremental/config_editor-1q2khge99p6my/s-hn0lkk6b83-0t4n74k.lock",
    "apps/config-editor/src-tauri/gen/schemas/desktop-schema.json",
  ];
  for (const [index, relativePath] of generated.entries()) {
    writeFile(root, relativePath, `generated fixture ${index}\n`);
  }

  const trackedArtifact =
    "crates/emuchef-rust-backend/target/debug/deps/libtracked-0123456789abcdef.rlib";
  writeFile(root, trackedArtifact, "tracked durable fixture artifact\n");
  forceTrack(root, trackedArtifact);

  const preserved = [
    "crates/emuchef-rust-backend/target/keep-important.json",
    "crates/emuchef-rust-backend/target/.emuchef-cargo-test-source.sha256.pending",
    ...DURABLE_STATE_CANNARIES,
    "crates/emuchef-rust-backend/target/debug/unrecognized.txt",
    "crates/emuchef-rust-backend/target/debug/deps/notes.keep",
    "crates/emuchef-rust-backend/target/release/build/backend-0123456789abcdef/out/release-manifest.json",
    "crates/emuchef-rust-backend/target/release/build/backend-0123456789abcdef/out/bundle/release.dmg",
    "apps/emuchef-app/src-tauri/gen/schemas/desktop-schema.json",
    "apps/emuchef-app/src-tauri/gen/schemas/unrecognized-schema.json",
    "crates/emuchef-rust-backend/target/release/bin/qualified-backend",
    "crates/emuchef-rust-backend/target/release/manifest.json",
    "apps/emuchef-app/src-tauri/target/release/bundle/macos/EmuChef.app/Contents/Info.plist",
    "apps/config-editor/src-tauri/target/debug/bundle/dmg/ConfigEditor.dmg",
    "release-artifacts/EmuChef.dmg",
    "release-artifacts/manifest.json",
    "docs/testing/device-result.json",
    "docs/qualification/candidate.json",
    "docs/release/release-manifest.json",
    "snapshots/ui/baseline.png",
    "fixtures/catalog/known-good.json",
    "authored/apps/example.json",
    ".emuchef_cache/catalog/index.json",
    ".emuchef_runtime/executions/result.json",
    ".chatgpt/PHASE_A_FEASIBILITY.md",
    ".chatgpt/history/RESULT.json",
    ".chatgpt/reviews/previous-review.md",
  ];
  for (const relativePath of preserved) writeFile(root, relativePath, `durable: ${relativePath}\n`);
  forceTrack(root, "apps/emuchef-app/src-tauri/gen/schemas/desktop-schema.json");

  const preservedBefore = snapshotFiles(root, [...preserved, trackedArtifact]);

  const externalTarget = path.join(sandbox, "external-cargo-target");
  writeFile(externalTarget, "debug/deps/libexternal-0123456789abcdef.rlib", "external target\n");
  const externalBefore = snapshotTree(externalTarget);
  const result = make(root, "clean", { CARGO_TARGET_DIR: externalTarget });
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  for (const relativePath of generated) assertRemoved(root, relativePath);
  assert.deepEqual(snapshotFiles(root, [...preserved, trackedArtifact]), preservedBefore);
  assert.match(output, /estimated/i);
  assert.match(output, /removed/i);
  assert.match(output, /SKIP.*unrecognized\.txt|unrecognized\.txt.*SKIP/i);
  assert.deepEqual(snapshotTree(externalTarget), externalBefore);
});


test("default cleanup removes the obsolete freshness digest and preserves pending until backend-test-fresh cleans Cargo", (t) => {
  const { root } = createFixture(t);
  const backendDir = "crates/emuchef-rust-backend";
  const stampPath = `${backendDir}/target/.emuchef-cargo-test-source.sha256`;
  const pendingPath = `${stampPath}.pending`;
  writeFile(root, `${backendDir}/Cargo.toml`, "[package]\nname = \"emuchef-rust-backend\"\nversion = \"0.1.0\"\n");
  writeFile(root, `${backendDir}/src/lib.rs`, "pub fn fixture() {}\n");
  writeFile(root, stampPath, "obsolete digest\n");
  writeFile(root, pendingPath, "pending invalidation\n");

  const toolsDirectory = path.join(root, "tools");
  fs.mkdirSync(toolsDirectory, { recursive: true });
  fs.copyFileSync(
    path.join(REPOSITORY_ROOT, "tools", "cargo-test-freshness.mjs"),
    path.join(toolsDirectory, "cargo-test-freshness.mjs"),
  );

  const cleanup = make(root, "clean");
  assert.equal(cleanup.status, 0, outputOf(cleanup));
  assertRemoved(root, stampPath);
  assertExists(root, pendingPath);

  const shimDirectory = path.join(root, "test-shims");
  fs.mkdirSync(shimDirectory, { recursive: true });
  const cargoLog = path.join(root, "cargo-clean.log");
  const cargoShim = path.join(shimDirectory, "cargo");
  fs.writeFileSync(
    cargoShim,
    '#!/bin/sh\nprintf \'%s\\n\' "$@" > "$CARGO_FIXTURE_LOG"\n',
  );
  fs.chmodSync(cargoShim, 0o755);

  const freshness = make(root, "backend-test-fresh", {
    PATH: `${shimDirectory}${path.delimiter}${process.env.PATH}`,
    CARGO_FIXTURE_LOG: cargoLog,
  });
  assert.equal(freshness.status, 0, outputOf(freshness));
  assert.deepEqual(fs.readFileSync(cargoLog, "utf8").trim().split("\n"), [
    "clean",
    "--manifest-path",
    `${backendDir}/Cargo.toml`,
    "-p",
    "emuchef-rust-backend",
  ]);
  assert.ok(
    fs.existsSync(path.join(root, stampPath)),
    "backend-test-fresh should recreate the digest stamp",
  );
  assert.equal(
    fs.existsSync(path.join(root, pendingPath)),
    false,
    "pending clears only after Cargo clean succeeds",
  );
});


test("backend-test-fresh preserves its pending marker when cargo clean fails", (t) => {
  const { root } = createFixture(t);
  const backendDir = "crates/emuchef-rust-backend";
  const stampPath = `${backendDir}/target/.emuchef-cargo-test-source.sha256`;
  const pendingPath = `${stampPath}.pending`;
  writeFile(root, `${backendDir}/Cargo.toml`, "[package]\nname = \"emuchef-rust-backend\"\nversion = \"0.1.0\"\n");
  writeFile(root, `${backendDir}/src/lib.rs`, "pub fn fixture() {}\n");
  writeFile(root, pendingPath, "pending invalidation\n");

  const toolsDirectory = path.join(root, "tools");
  fs.mkdirSync(toolsDirectory, { recursive: true });
  fs.copyFileSync(
    path.join(REPOSITORY_ROOT, "tools", "cargo-test-freshness.mjs"),
    path.join(toolsDirectory, "cargo-test-freshness.mjs"),
  );

  const shimDirectory = path.join(root, "test-shims");
  fs.mkdirSync(shimDirectory, { recursive: true });
  const cargoLog = path.join(root, "cargo-clean.log");
  const cargoShim = path.join(shimDirectory, "cargo");
  fs.writeFileSync(
    cargoShim,
    '#!/bin/sh\nprintf \'%s\\n\' "$@" > "$CARGO_FIXTURE_LOG"\nexit 23\n',
  );
  fs.chmodSync(cargoShim, 0o755);
  for (const command of ["npm", "pnpm", "curl", "wget", "adb", "rustup"]) {
    const shimPath = path.join(shimDirectory, command);
    fs.writeFileSync(shimPath, "#!/bin/sh\nexit 97\n");
    fs.chmodSync(shimPath, 0o755);
  }

  const result = make(root, "backend-test-fresh", {
    PATH: `${shimDirectory}${path.delimiter}${process.env.PATH}`,
    CARGO_FIXTURE_LOG: cargoLog,
  });

  assert.notEqual(result.status, 0, outputOf(result));
  assert.deepEqual(fs.readFileSync(cargoLog, "utf8").trim().split("\n"), [
    "clean",
    "--manifest-path",
    `${backendDir}/Cargo.toml`,
    "-p",
    "emuchef-rust-backend",
  ]);
  assertExists(root, pendingPath);
});


test("clean-deep reports node_modules .gitignore removal and preserves ignore files elsewhere", (t) => {
  const { root } = createFixture(t);
  const dependencyIgnore = "apps/emuchef-app/node_modules/pkg/.gitignore";
  const distIgnore = "apps/emuchef-app/dist/.gitignore";
  const unknownDistFile = "apps/emuchef-app/dist/assets/unknown.keep";
  writeFile(root, dependencyIgnore, "ignored dependency output\n");
  writeFile(root, distIgnore, "frontend ignore rules\n");
  writeFile(root, unknownDistFile, "unknown frontend output\n");

  const result = make(root, "clean-deep");
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  assert.match(output, /apps\/emuchef-app\/node_modules\/pkg\/\.gitignore/);
  assert.match(
    output,
    /PRESERVE tracked files; \.gitignore and unknown content outside node_modules; deep mode removes untracked regular files under node_modules; symlinks are skipped without following targets; nested \.git metadata is preserved/i,
  );
  assertRemoved(root, dependencyIgnore);
  assertExists(root, distIgnore);
  assertExists(root, unknownDistFile);
  assertExists(root, "apps/emuchef-app/src-tauri/binaries/.gitignore");
});


test("make clean-preview reports eligible estimates and preserved categories without changing files", (t) => {
  const { root } = createFixture(t);
  const previewArtifact = "preview artifact\n";
  writeFile(
    root,
    "crates/emuchef-rust-backend/target/debug/deps/libpreview-0123456789abcdef.rlib",
    previewArtifact,
  );
  writeFile(root, "apps/emuchef-app/dist/index.html", "built frontend\n");
  writeFile(root, "apps/emuchef-app/node_modules/pkg/index.js", "installed dependency\n");
  writeFile(
    root,
    "apps/emuchef-app/src-tauri/target/release/bundle/macos/app.dmg",
    "release bundle\n",
  );
  writeFile(root, ".emuchef_runtime/executions/result.json", "runtime state\n");
  const before = snapshotTree(root);

  const result = make(root, "clean-preview");
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  assert.match(output, /clean-preview/i);
  assert.match(output, /libpreview-0123456789abcdef\.rlib/);
  const estimate = Buffer.byteLength(previewArtifact);
  assert.ok(
    output.includes(
      `clean-preview: ELIGIBLE [cargo-intermediates] crates/emuchef-rust-backend/target/debug/deps (1 files; estimated ${estimate} bytes)`,
    ),
  );
  assert.ok(output.includes(`clean-preview: eligible estimated bytes: ${estimate}`));
  assert.match(
    output,
    /clean-preview: SKIP deep-only categories: both apps' node_modules and dist, and validated Tauri sidecar copies/,
  );
  assert.match(output, /clean-preview: PRESERVE release outputs:/);
  assert.match(output, /clean-preview: PRESERVE application\/runtime\/developer state:/);
  assert.match(output, /node_modules/);
  assert.match(output, /dist/);
  assert.match(output, /\.emuchef_runtime/);
  assert.match(output, /no files changed/i);
  assert.deepEqual(snapshotTree(root), before);
});


test("default cleanup retains deep outputs while clean-deep removes only its allowlisted categories", (t) => {
  const { root } = createFixture(t);
  writeFile(root, "crates/emuchef-rust-backend/target/debug/deps/libdefault-0123456789abcdef.rlib");
  const deepOutputs = [
    "apps/emuchef-app/node_modules/pkg/package.json",
    "apps/emuchef-app/node_modules/.emuchef-deps-stamp",
    "apps/config-editor/node_modules/pkg/package.json",
    "apps/config-editor/node_modules/.emuchef-deps-stamp",
    "apps/emuchef-app/dist/assets/app.js",
    "apps/config-editor/dist/assets/app.js",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin",
  "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin.metadata.json",
    "apps/config-editor/src-tauri/binaries/emuchef-x86_64-pc-windows-msvc.exe",
    "apps/config-editor/src-tauri/binaries/emuchef-x86_64-pc-windows-msvc.exe.metadata.json",
  ];
  for (const relativePath of deepOutputs) writeFile(root, relativePath, "deep output\n");
  const unknownSidecarContent = "apps/emuchef-app/src-tauri/binaries/operator-notes.txt";
  writeFile(root, unknownSidecarContent, "keep this unknown file\n");
  const unknownSidecarMetadata =
    "apps/emuchef-app/src-tauri/binaries/emuchef-linux.metadata.json";
  writeFile(root, unknownSidecarMetadata, "no generated metadata candidate exists\n");

  const normal = make(root, "clean");
  assert.equal(normal.status, 0, outputOf(normal));
  assertRemoved(root, "crates/emuchef-rust-backend/target/debug/deps/libdefault-0123456789abcdef.rlib");
  for (const relativePath of deepOutputs) assertExists(root, relativePath);

  const deep = make(root, "clean-deep");
  const output = outputOf(deep);
  assert.equal(deep.status, 0, output);
  assert.ok(output.indexOf("WARNING") < output.indexOf("PREVIEW"));
  assert.ok(output.indexOf("PREVIEW") < output.indexOf("REMOVED"));
  for (const relativePath of deepOutputs) assertRemoved(root, relativePath);
  for (const relativePath of [
    "apps/emuchef-app/src-tauri/binaries/.gitignore",
    "apps/config-editor/src-tauri/binaries/.gitignore",
    unknownSidecarContent,
    unknownSidecarMetadata,
  ]) {
    assertExists(root, relativePath);
  }
  assert.match(output, /reinstall|regenerat/i);
  assert.match(output, /operator-notes\.txt/);
});


test("clean-deep removes canonical sidecars for other host triples and preserves near matches", (t) => {
  const { root } = createFixture(t);
  const eligible = [
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin.metadata.json",
    "apps/emuchef-app/src-tauri/binaries/emuchef-x86_64-apple-darwin",
    "apps/emuchef-app/src-tauri/binaries/emuchef-x86_64-apple-darwin.metadata.json",
    "apps/config-editor/src-tauri/binaries/emuchef-x86_64-unknown-linux-gnu",
    "apps/config-editor/src-tauri/binaries/emuchef-x86_64-unknown-linux-gnu.metadata.json",
    "apps/config-editor/src-tauri/binaries/emuchef-x86_64-pc-windows-msvc.exe",
    "apps/config-editor/src-tauri/binaries/emuchef-x86_64-pc-windows-msvc.exe.metadata.json",
  ];
  const nearMatches = [
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin.backup",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin.old",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin-extra",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin.extra.metadata.json",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin-extra.metadata.json",
  ];
  for (const relativePath of [...eligible, ...nearMatches]) {
    writeFile(root, relativePath, `sidecar fixture: ${relativePath}\n`);
  }
  const nearMatchesBefore = snapshotFiles(root, nearMatches);

  const result = make(root, "clean-deep");
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  const previewStart = output.indexOf("PREVIEW");
  const deletionStart = output.indexOf("REMOVED");
  assert.ok(previewStart >= 0 && previewStart < deletionStart, output);
  const preview = output.slice(previewStart, deletionStart);
  for (const relativePath of nearMatches) {
    assert.ok(
      preview.includes(
        `SKIP ${relativePath}: unknown Tauri binary-directory content preserved`,
      ),
      `expected ${relativePath} to be skipped during deep preview\n${output}`,
    );
  }
  for (const relativePath of eligible) assertRemoved(root, relativePath);
  assert.deepEqual(snapshotFiles(root, nearMatches), nearMatchesBefore);
});


test("clean-deep removes untracked npm regular files and dependency-install stamps while preserving symlinks safely", (t) => {
  const { sandbox, root } = createFixture(t);
  const generated = [
    "apps/emuchef-app/node_modules/@emuchef/cli/package.json",
    "apps/emuchef-app/node_modules/@emuchef/cli/bin/emuchef.js",
    "apps/emuchef-app/node_modules/@emuchef/cli/README.md",
    "apps/emuchef-app/node_modules/@emuchef/cli/node_modules/transitive/package.json",
    "apps/emuchef-app/node_modules/.emuchef-deps-stamp",
    "apps/emuchef-app/node_modules/@emuchef/native/bin/emuchef-helper-darwin-arm64",
    "apps/config-editor/node_modules/typescript/lib/tsc.js",
    "apps/config-editor/node_modules/typescript/package.json",
    "apps/config-editor/node_modules/.emuchef-deps-stamp",
    "apps/config-editor/node_modules/@emuchef/native/bin/emuchef-helper-linux-x64",
    "apps/emuchef-app/node_modules/@emuchef/native/.gitignore",
  ];
  for (const relativePath of generated) writeFile(root, relativePath, `dependency:${relativePath}\n`);
  const nestedGitConfig = "apps/emuchef-app/node_modules/@emuchef/native/.git/config";
  writeFile(root, nestedGitConfig, "[core]\nrepositoryformatversion = 0\n");
  for (const relativePath of [
    "apps/emuchef-app/node_modules/@emuchef/native/bin/emuchef-helper-darwin-arm64",
    "apps/config-editor/node_modules/@emuchef/native/bin/emuchef-helper-linux-x64",
  ]) {
    fs.chmodSync(path.join(root, relativePath), 0o755);
  }

  const trackedDependency = "apps/emuchef-app/node_modules/@emuchef/cli/tracked-fixture.txt";
  writeFile(root, trackedDependency, "tracked dependency fixture\n");
  forceTrack(root, trackedDependency);

  const externalTarget = writeFile(sandbox, "external-bin/emuchef-helper.js", "external executable\n");
  const binLink = "apps/emuchef-app/node_modules/.bin/emuchef-helper";
  fs.mkdirSync(path.dirname(path.join(root, binLink)), { recursive: true });
  fs.symlinkSync(externalTarget, path.join(root, binLink));
  const externalBefore = crypto.createHash("sha256").update(fs.readFileSync(externalTarget)).digest("hex");

  const result = make(root, "clean-deep");
  const output = outputOf(result);

  assert.equal(result.status, 0, output);
  for (const relativePath of generated) assertRemoved(root, relativePath);
  assert.equal(fs.lstatSync(path.join(root, binLink)).isSymbolicLink(), true);
  assert.equal(fs.readlinkSync(path.join(root, binLink)), externalTarget);
  assert.ok(
    output.includes(`SKIP ${binLink}: symbolic link cleanup candidate skipped`),
    output,
  );
  assertExists(root, trackedDependency);
  assert.equal(
    crypto.createHash("sha256").update(fs.readFileSync(externalTarget)).digest("hex"),
    externalBefore,
  );
  assertExists(root, nestedGitConfig);
  assert.match(output, /tracked-fixture\.txt.*tracked file preserved|tracked file preserved.*tracked-fixture\.txt/);
});


test("missing outputs are successful no-ops and repeated cleanup is idempotent", (t) => {
  const { root } = createFixture(t);
  for (const invocation of [1, 2]) {
    const result = make(root, "clean");
    assert.equal(result.status, 0, `invocation ${invocation}: ${outputOf(result)}`);
  }
});


test("cleanup in a linked worktree leaves the primary synthetic checkout untouched", (t) => {
  const { sandbox, root } = createFixture(t);
  const linkedRoot = path.join(sandbox, "linked-checkout");
  git(root, ["worktree", "add", "-q", "-b", "fixture-linked", linkedRoot]);
  const artifact = "crates/emuchef-rust-backend/target/debug/deps/liblinked-0123456789abcdef.rlib";
  writeFile(root, artifact, "primary worktree artifact\n");
  writeFile(linkedRoot, artifact, "linked worktree artifact\n");
  const primaryBefore = snapshotTree(root);
  t.after(() => {
    if (fs.existsSync(path.join(root, ".git"))) {
      run("git", ["-C", root, "worktree", "remove", "--force", linkedRoot], root);
    }
  });

  const result = make(linkedRoot, "clean");

  assert.equal(result.status, 0, outputOf(result));
  assertExists(root, artifact);
  assertRemoved(linkedRoot, artifact);
  assert.equal(snapshotTree(root).includes(`${artifact}:file:${crypto.createHash("sha256").update("primary worktree artifact\n").digest("hex")}`), true);
  assert.ok(primaryBefore.length > 0);
});


test("clean-deep directly removes known outputs and preserves hashed evidence and unknown files", (t) => {
  const { root } = createFixture(t);
  const generated = [
    "crates/emuchef-rust-backend/target/debug/deps/libdeep-0123456789abcdef.rlib",
    "apps/emuchef-app/src-tauri/gen/schemas/desktop-schema.json",
    "apps/emuchef-app/node_modules/pkg/index.js",
    "apps/emuchef-app/node_modules/pkg/unknown.keep",
    "apps/emuchef-app/dist/index.html",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin",
    "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin.metadata.json",
  ];
  for (const relativePath of generated) writeFile(root, relativePath, `generated:${relativePath}\n`);

  const preserved = [
    "crates/emuchef-rust-backend/target/.emuchef-cargo-test-source.sha256.pending",
    ...DURABLE_STATE_CANNARIES,
    "crates/emuchef-rust-backend/target/debug/unknown.keep",
    "apps/emuchef-app/dist/assets/unknown.keep",
    "apps/emuchef-app/src-tauri/binaries/emuchef-bad.metadata.json",
    "apps/emuchef-app/src-tauri/target/release/bundle/macos/App.app/Info.plist",
    "apps/config-editor/src-tauri/target/release/manifest.json",
    "release-artifacts/release-manifest.json",
    "docs/testing/device-result.json",
    "docs/qualification/candidate.json",
    "docs/release/release-manifest.json",
    ".emuchef_runtime/executions/result.json",
    "snapshots/ui/baseline.png",
    "fixtures/catalog/known-good.json",
    "authored/apps/example.json",
    ".chatgpt/state/agent.json",
    "apps/emuchef-app/node_modules/pkg/tracked.js",
    "apps/emuchef-app/dist/assets/tracked.js",
  ];
  for (const relativePath of preserved) writeFile(root, relativePath, `preserved:${relativePath}\n`);
  forceTrack(root, "apps/emuchef-app/node_modules/pkg/tracked.js");
  forceTrack(root, "apps/emuchef-app/dist/assets/tracked.js");
  const preservedBefore = snapshotFiles(root, preserved);

  const first = make(root, "clean-deep");
  const firstOutput = outputOf(first);
  assert.equal(first.status, 0, firstOutput);
  for (const relativePath of generated) assertRemoved(root, relativePath);
  assert.deepEqual(snapshotFiles(root, preserved), preservedBefore);
  assert.match(firstOutput, /emuchef-aarch64-apple-darwin\.metadata\.json/);

  const second = make(root, "clean-deep");
  const secondOutput = outputOf(second);
  assert.equal(second.status, 0, secondOutput);
  assert.match(secondOutput, /REMOVED none \(estimated 0 bytes\)/i);
  assert.match(secondOutput, /removed estimated bytes: 0/i);
});


test("cleanup commands do not invoke build, install, download, network, or device tools", (t) => {
  const { root } = createFixture(t);
  const shimDirectory = path.join(root, "test-shims");
  const markerDirectory = path.join(shimDirectory, "markers");
  fs.mkdirSync(markerDirectory, { recursive: true });
  const commands = ["cargo", "npm", "pnpm", "curl", "wget", "adb"];
  for (const command of commands) {
    const shimPath = path.join(shimDirectory, command);
    const markerPath = path.join(markerDirectory, `${command}.called`);
    fs.writeFileSync(shimPath, `#!/bin/sh\n: > '${markerPath}'\nexit 97\n`);
    fs.chmodSync(shimPath, 0o755);
  }
  writeFile(root, "crates/emuchef-rust-backend/target/debug/deps/libshim-0123456789abcdef.rlib");
  writeFile(root, "apps/emuchef-app/dist/index.html");
  const env = { PATH: `${shimDirectory}:${process.env.PATH}` };

  for (const target of ["clean", "clean-preview", "clean-deep"]) {
    const result = make(root, target, env);
    assert.equal(result.status, 0, outputOf(result));
  }
  for (const command of commands) {
    assert.equal(
      fs.existsSync(path.join(markerDirectory, `${command}.called`)),
      false,
      `${command} must not be invoked`,
    );
  }
});

test("incremental cleanup preserves unknown directories and reclaims Cargo-generated directories", (t) => {
  const { root } = createFixture(t);
  const unknownFile =
    "crates/emuchef-rust-backend/target/debug/incremental/operator-notes-archive/keep.txt";
  const generatedFile =
    "crates/emuchef-rust-backend/target/debug/incremental/backend-3l9cz3al16vr5/s-hn0lkk6b83-0t4n74k-br4pfaeeq8b6q27a5frw5n3u3/query-cache.bin";
  const unknownContents = "operator notes must survive cleanup\n";
  writeFile(root, unknownFile, unknownContents);
  writeFile(root, generatedFile, "Cargo incremental output\n");

  const beforePreview = snapshotTree(root);
  const preview = make(root, "clean-preview");
  const previewOutput = outputOf(preview);
  assert.equal(preview.status, 0, previewOutput);
  assert.match(
    previewOutput,
    /target\/debug\/incremental\/operator-notes-archive: unrecognized Cargo output preserved/,
  );
  assert.match(previewOutput, /backend-3l9cz3al16vr5/);
  assert.match(previewOutput, /no files changed/i);
  assert.deepEqual(snapshotTree(root), beforePreview);

  const clean = make(root, "clean");
  assert.equal(clean.status, 0, outputOf(clean));
  assertRemoved(root, generatedFile);
  assertExists(root, unknownFile);
  assert.equal(fs.readFileSync(path.join(root, unknownFile), "utf8"), unknownContents);

  writeFile(root, generatedFile, "recreated Cargo incremental output\n");
  const deep = make(root, "clean-deep");
  assert.equal(deep.status, 0, outputOf(deep));
  assertRemoved(root, generatedFile);
  assertExists(root, unknownFile);
  assert.equal(fs.readFileSync(path.join(root, unknownFile), "utf8"), unknownContents);
});

test("incremental cleanup preserves operator directories with hash-like suffixes", (t) => {
  const { root } = createFixture(t);
  const operatorFile =
    "crates/emuchef-rust-backend/target/debug/incremental/operator-notes-0123456789abc/keep.txt";
  writeFile(root, operatorFile, "operator content\n");

  const preview = make(root, "clean-preview");
  const previewOutput = outputOf(preview);
  assert.equal(preview.status, 0, previewOutput);
  assert.match(
    previewOutput,
    /incremental\/operator-notes-0123456789abc: unrecognized Cargo output preserved/,
  );

  const clean = make(root, "clean");
  assert.equal(clean.status, 0, outputOf(clean));
  assertExists(root, operatorFile);

  const deep = make(root, "clean-deep");
  assert.equal(deep.status, 0, outputOf(deep));
  assertExists(root, operatorFile);
});

test("incremental cleanup preserves unknown descendants of recognized Cargo directories", (t) => {
  const { root } = createFixture(t);
  const crateDirectory = "crates/emuchef-rust-backend/target/debug/incremental/serde_derive-3l9cz3al16vr5";
  const sessionDirectory = `${crateDirectory}/s-hn0lkk6b83-0t4n74k-br4pfaeeq8b6q27a5frw5n3u3`;
  const generatedFiles = [
    `${crateDirectory}/s-hn0lkk6b83-0t4n74k.lock`,
    `${sessionDirectory}/dep-graph.bin`,
    `${sessionDirectory}/query-cache.bin`,
    `${sessionDirectory}/012a403e9odf0sk4sc4l2q8wv.o`,
  ];
  const preservedFiles = [
    `${crateDirectory}/operator-notes.txt`,
    `${sessionDirectory}/operator-notes.txt`,
    `${crateDirectory}/operator-notes-directory/keep.txt`,
  ];
  const preservedDirectory = `${crateDirectory}/operator-notes-directory`;
  for (const relativePath of generatedFiles) writeFile(root, relativePath, "Cargo output\n");
  for (const relativePath of preservedFiles) writeFile(root, relativePath, "operator content\n");

  const beforePreview = snapshotTree(root);
  const preview = make(root, "clean-preview");
  const previewOutput = outputOf(preview);
  assert.equal(preview.status, 0, previewOutput);
  for (const relativePath of [...preservedFiles.slice(0, 2), preservedDirectory]) {
    assert.ok(
      previewOutput.includes(
        `${relativePath}: unrecognized Cargo incremental entry preserved`,
      ),
    );
  }
  assert.match(previewOutput, /\(4 files; estimated \d+ bytes\)/);
  assert.deepEqual(snapshotTree(root), beforePreview);

  const clean = make(root, "clean");
  assert.equal(clean.status, 0, outputOf(clean));
  for (const relativePath of generatedFiles) assertRemoved(root, relativePath);
  for (const relativePath of preservedFiles) assertExists(root, relativePath);
});

test("incremental cleanup reclaims working-session partial graphs and preserves unknown entries", (t) => {
  const { root } = createFixture(t);
  const crateDirectory = "crates/emuchef-rust-backend/target/debug/incremental/backend-3l9cz3al16vr5";
  const workingSession = `${crateDirectory}/s-hn0lkk6b83-0t4n74k-working`;
  const partialGraph = `${workingSession}/dep-graph.part.bin`;
  const unknownSibling = `${crateDirectory}/operator-notes.txt`;
  const unknownNested = `${workingSession}/operator-notes.txt`;
  writeFile(root, partialGraph, "partial Cargo graph\n");
  writeFile(root, unknownSibling, "operator sibling content\n");
  writeFile(root, unknownNested, "operator nested content\n");

  const beforePreview = snapshotTree(root);
  const preview = make(root, "clean-preview");
  const previewOutput = outputOf(preview);
  assert.equal(preview.status, 0, previewOutput);
  assert.ok(previewOutput.includes(partialGraph));
  assert.ok(
    previewOutput.includes(
      `${unknownSibling}: unrecognized Cargo incremental entry preserved`,
    ),
  );
  assert.ok(
    previewOutput.includes(
      `${unknownNested}: unrecognized Cargo incremental entry preserved`,
    ),
  );
  assert.deepEqual(snapshotTree(root), beforePreview);

  const clean = make(root, "clean");
  assert.equal(clean.status, 0, outputOf(clean));
  assertRemoved(root, partialGraph);
  assertExists(root, unknownSibling);
  assertExists(root, unknownNested);

  writeFile(root, partialGraph, "recreated partial Cargo graph\n");
  const deep = make(root, "clean-deep");
  assert.equal(deep.status, 0, outputOf(deep));
  assertRemoved(root, partialGraph);
  assertExists(root, unknownSibling);
  assertExists(root, unknownNested);
});
