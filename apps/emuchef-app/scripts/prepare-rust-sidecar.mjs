#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  targetExecutableExtension,
  validateTargetTriple,
} from "../../../scripts/sidecar-target-contract.mjs";
import { BINARY_BASENAME, externalBinArtifactName } from "./sidecar-packaging.mjs";

const profileIndex = process.argv.indexOf("--profile");
const profile = profileIndex >= 0 ? process.argv[profileIndex + 1] : "debug";
if (!new Set(["debug", "release"]).has(profile)) throw new Error("--profile must be debug or release");

const appDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(appDir, "../..");
const manifest = path.join(repoRoot, "crates/emuchef-rust-backend/Cargo.toml");
const targetResult = spawnSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" });
if (targetResult.status !== 0) throw new Error("rustc could not report its host target");
const target = validateTargetTriple(targetResult.stdout.trim());
const args = ["build", "--manifest-path", manifest, "--bin", BINARY_BASENAME];
if (profile === "release") args.push("--release");
const build = spawnSync("cargo", args, { stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

const extension = targetExecutableExtension(target);
const source = path.join(repoRoot, "crates/emuchef-rust-backend/target", profile, `${BINARY_BASENAME}${extension}`);
const destination = path.join(appDir, "src-tauri/binaries", externalBinArtifactName(target));
fs.mkdirSync(path.dirname(destination), { recursive: true });
fs.copyFileSync(source, destination);
if (extension !== ".exe") fs.chmodSync(destination, 0o755);
console.log(`Prepared ${profile} Rust sidecar: ${destination}`);
