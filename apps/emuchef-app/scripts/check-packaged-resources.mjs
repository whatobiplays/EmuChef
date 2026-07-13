#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const appDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(fs.readFileSync(path.join(appDir, "src-tauri/tauri.conf.json"), "utf8"));
const resources = config.bundle?.resources ?? {};
for (const directory of ["apps", "device_plans", "device_profiles", "recipes"]) {
  if (!Object.values(resources).includes(`catalog/${directory}`)) {
    throw new Error(`Bundled catalog resource '${directory}' is missing`);
  }
}
const serialized = JSON.stringify(config);
if (/platform-tools|platform_tools|latest-darwin\.zip/i.test(serialized)) {
  throw new Error("Tauri bundle must not contain Platform-Tools resources");
}

const bundleIndex = process.argv.indexOf("--bundle");
if (bundleIndex >= 0) {
  const bundle = process.argv[bundleIndex + 1];
  if (!bundle) throw new Error("--bundle requires an .app path");
  const contents = path.join(path.resolve(bundle), "Contents");
  const catalog = path.join(contents, "Resources", "catalog");
  const catalogRoot = fs.realpathSync(catalog);
  const files = [];
  for (const directory of ["apps", "device_plans", "device_profiles", "recipes"]) {
    const directoryPath = path.join(catalog, directory);
    const metadata = fs.lstatSync(directoryPath);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error(`Packaged catalog '${directory}' is not a real directory`);
    }
    for (const name of fs.readdirSync(directoryPath).sort()) {
      const entry = path.join(directoryPath, name);
      const entryMetadata = fs.lstatSync(entry);
      if (name === ".gitkeep" && entryMetadata.isFile() && !entryMetadata.isSymbolicLink()) {
        continue;
      }
      if (!entryMetadata.isFile() || entryMetadata.isSymbolicLink() || !/\.ya?ml$/.test(name)) {
        throw new Error(`Packaged catalog '${directory}' contains an unsupported entry`);
      }
      const resolved = fs.realpathSync(entry);
      if (!resolved.startsWith(`${catalogRoot}${path.sep}`)) {
        throw new Error(`Packaged catalog '${directory}' escaped its resource root`);
      }
      files.push([`${directory}/${name}`, fs.readFileSync(entry)]);
    }
  }
  if (files.length === 0) throw new Error("Packaged catalog is empty");
  const hasher = crypto.createHash("sha256");
  for (const [relative, bytes] of files.sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))) {
    hasher.update(`${Buffer.byteLength(relative, "utf8")}:${relative}${bytes.length}:`);
    hasher.update(bytes);
  }
  const digest = hasher.digest("hex");
  if (!/^[a-f0-9]{64}$/.test(digest)) throw new Error("Packaged catalog digest is invalid");
  const sidecar = path.join(contents, "MacOS", "emuchef");
  if (!fs.statSync(sidecar).isFile()) throw new Error("Packaged Rust sidecar is missing");
  const packagedNames = [];
  for (const base of [path.join(contents, "MacOS"), path.join(contents, "Resources")]) {
    const visit = (directory) => {
      for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        packagedNames.push(entry.name);
        if (entry.isDirectory()) visit(path.join(directory, entry.name));
      }
    };
    visit(base);
  }
  if (packagedNames.some((name) => /platform[-_]tools|latest-darwin\.zip/i.test(name))) {
    throw new Error("Packaged app contains a forbidden Platform-Tools artifact");
  }
  console.log(`Packaged Rust sidecar and catalog snapshot verified (${files.length} files, sha256 ${digest}).`);
} else {
  console.log("Packaged catalog resources are declared and Platform-Tools is absent.");
}
