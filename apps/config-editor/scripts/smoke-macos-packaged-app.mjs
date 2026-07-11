#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { parseAppPath } from "./check-macos-bundle.mjs";

const TIMEOUT_MS = 10_000;

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function waitForExit(child, timeoutMs = TIMEOUT_MS) {
  if (child.exitCode !== null) {
    return Promise.resolve({ code: child.exitCode, signal: child.signalCode });
  }
  return Promise.race([
    new Promise((resolve) => child.once("exit", (code, signal) => resolve({ code, signal }))),
    delay(timeoutMs).then(() => {
      throw new Error(`process ${child.pid} did not exit within ${timeoutMs} ms`);
    }),
  ]);
}

export function validateSidecarResponses(lines) {
  if (lines.length !== 2) {
    throw new Error(`expected two sidecar responses, received ${lines.length}`);
  }
  const responses = lines.map((line) => JSON.parse(line));
  for (const [index, expectedId] of ["bundle-hello", "bundle-ping"].entries()) {
    if (responses[index].id !== expectedId || responses[index].ok !== true) {
      throw new Error(`sidecar response '${expectedId}' was not successful`);
    }
  }
  return responses;
}

export function processListContainsSidecar(processList, sidecarPath) {
  return processList
    .split("\n")
    .some((line) => line.includes(sidecarPath) && line.includes("--sidecar"));
}

function plistExecutable(infoPath) {
  const result = spawnSync("plutil", ["-extract", "CFBundleExecutable", "raw", infoPath], {
    encoding: "utf8",
  });
  if (result.error || result.status !== 0) {
    throw new Error(`failed to read CFBundleExecutable: ${(result.stderr || "").trim()}`);
  }
  return result.stdout.trim();
}

export function resolvePackagedExecutables(appPath) {
  const infoPath = path.join(appPath, "Contents", "Info.plist");
  const mainName = plistExecutable(infoPath);
  const result = {
    mainPath: path.join(appPath, "Contents", "MacOS", mainName),
    sidecarPath: path.join(appPath, "Contents", "MacOS", "emuchef"),
  };
  for (const [label, filePath] of Object.entries(result)) {
    if (!fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
      throw new Error(`${label} was not found at '${filePath}'`);
    }
    fs.accessSync(filePath, fs.constants.X_OK);
  }
  return result;
}

async function directSidecarSmoke(sidecarPath) {
  const child = spawn(sidecarPath, ["--sidecar"], { stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  child.stdin.end(
    `${JSON.stringify({ id: "bundle-hello", type: "hello" })}\n${JSON.stringify({ id: "bundle-ping", type: "ping" })}\n`,
  );
  const exit = await waitForExit(child);
  if (exit.code !== 0) {
    throw new Error(`bundled sidecar exited ${exit.code}; stderr was ${stderr.length} bytes`);
  }
  const lines = stdout.split("\n").filter(Boolean);
  validateSidecarResponses(lines);
  return { responseCount: lines.length, stderrPresent: stderr.length > 0 };
}

async function packagedAppLaunchSmoke(mainPath, sidecarPath) {
  const child = spawn(mainPath, [], { stdio: ["ignore", "pipe", "pipe"] });
  let stdoutBytes = 0;
  let stderrBytes = 0;
  child.stdout.on("data", (chunk) => {
    stdoutBytes += chunk.length;
  });
  child.stderr.on("data", (chunk) => {
    stderrBytes += chunk.length;
  });

  try {
    await delay(5_000);
    if (child.exitCode !== null) {
      throw new Error(`packaged application exited early with code ${child.exitCode}`);
    }
    const processList = spawnSync("ps", ["-axo", "pid=,ppid=,command="], {
      encoding: "utf8",
    });
    if (processList.error || processList.status !== 0) {
      throw new Error("failed to inspect packaged application processes");
    }
    if (!processListContainsSidecar(processList.stdout, sidecarPath)) {
      throw new Error("the exact bundled sidecar process was not observed after application launch");
    }
  } finally {
    if (child.exitCode === null) {
      child.kill("SIGTERM");
      try {
        await waitForExit(child, 5_000);
      } catch {
        child.kill("SIGKILL");
        await waitForExit(child, 5_000);
      }
    }
  }

  await delay(500);
  const after = spawnSync("ps", ["-axo", "pid=,ppid=,command="], { encoding: "utf8" });
  if (after.status === 0 && processListContainsSidecar(after.stdout, sidecarPath)) {
    throw new Error("bundled sidecar remained after the packaged application exited");
  }
  return { stderrBytes, stdoutBytes };
}

async function main() {
  const appPath = parseAppPath(process.argv.slice(2));
  const { mainPath, sidecarPath } = resolvePackagedExecutables(appPath);
  const directSidecar = await directSidecarSmoke(sidecarPath);
  const applicationLaunch = await packagedAppLaunchSmoke(mainPath, sidecarPath);
  console.log(
    JSON.stringify(
      {
        kind: "macos_packaged_app_smoke",
        status: "passed",
        appPath,
        applicationLaunch,
        directSidecar,
      },
      null,
      2,
    ),
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`smoke-macos-packaged-app: ${error.message}`);
    process.exit(1);
  });
}
