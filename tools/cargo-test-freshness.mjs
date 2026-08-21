#!/usr/bin/env node
"use strict";

// Freshness gate for Cargo test artifacts.
//
// Cargo decides whether a local crate's test executable is up to date from
// file mtimes (its fingerprint uses CheckDepInfo with checksum disabled), so a
// checkout or restore that preserves old source mtimes can leave `cargo test`
// executing a stale binary even when the source content changed. This script
// compares a content digest of the crate's build inputs against a recorded
// stamp instead of relying on mtimes.
//
// Protocol (fail-safe against interruption):
//   1. If the digest differs from the stamp, or a `.pending` marker already
//      exists, the caller must invalidate the crate's Cargo artifacts.
//   2. The `.pending` marker is written before the stamp is replaced, so no
//      interruption can leave a changed source state looking fresh.
//   3. The caller clears the marker only after the package-scoped
//      `cargo clean` succeeds.
//
// Output: prints "invalidate" when the caller must clean, otherwise "fresh".
// Exit status is non-zero (fail closed) when inputs cannot be read.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

function usage() {
  console.error(
    "usage: node cargo-test-freshness.mjs <crate-dir> <stamp-file>",
  );
  process.exit(2);
}

function collectBuildInputs(crateDir) {
  const inputs = [];

  const addRegularFile = (relative) => {
    const full = path.join(crateDir, relative);
    if (fs.statSync(full).isFile()) {
      inputs.push(relative);
    }
  };

  for (const name of ["Cargo.toml", "Cargo.lock", "build.rs"]) {
    if (fs.existsSync(path.join(crateDir, name))) {
      addRegularFile(name);
    }
  }

  for (const sourceDir of ["src", "tests"]) {
    const base = path.join(crateDir, sourceDir);
    if (!fs.existsSync(base)) {
      continue;
    }
    const walk = (current, prefix) => {
      for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
        const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
        const full = path.join(current, entry.name);
        if (entry.isDirectory()) {
          walk(full, relative);
        } else if (entry.isFile()) {
          inputs.push(relative);
        }
      }
    };
    walk(base, sourceDir);
  }

  inputs.sort();
  return inputs;
}

function computeDigest(crateDir) {
  const hash = crypto.createHash("sha256");
  for (const relative of collectBuildInputs(crateDir)) {
    const content = fs.readFileSync(path.join(crateDir, relative));
    hash.update(relative);
    hash.update("\0");
    hash.update(content);
    hash.update("\0");
  }
  return hash.digest("hex");
}

function atomicWrite(file, content) {
  const directory = path.dirname(file);
  fs.mkdirSync(directory, { recursive: true });
  const temporary = path.join(
    directory,
    `.${path.basename(file)}.tmp-${process.pid}-${Date.now()}`,
  );
  fs.writeFileSync(temporary, content);
  fs.renameSync(temporary, file);
}

function main() {
  const [crateDir, stampFile] = process.argv.slice(2);
  if (!crateDir || !stampFile) {
    usage();
  }

  const digest = computeDigest(crateDir);
  const pendingFile = `${stampFile}.pending`;
  const hasPending = fs.existsSync(pendingFile);
  const previous = fs.existsSync(stampFile)
    ? fs.readFileSync(stampFile, "utf8").trim()
    : "";

  if (hasPending || previous !== digest) {
    atomicWrite(pendingFile, `${digest}\n`);
    atomicWrite(stampFile, `${digest}\n`);
    process.stdout.write("invalidate\n");
  } else {
    process.stdout.write("fresh\n");
  }
}

try {
  main();
} catch (error) {
  console.error(`cargo-test-freshness: ${error.message}`);
  process.exit(1);
}
