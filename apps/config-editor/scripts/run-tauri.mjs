#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const DEV_CONFIG_PATH = "src-tauri/tauri.dev.conf.json";

export function tauriArgs(argv) {
  if (argv[0] !== "dev" || argv.some((arg) => arg === "--config" || arg.startsWith("--config="))) {
    return [...argv];
  }
  return [...argv, "--config", DEV_CONFIG_PATH];
}

export function tauriInvocation(argv) {
  return {
    command: process.platform === "win32" ? "npm.cmd" : "npm",
    args: ["exec", "--", "tauri", ...tauriArgs(argv)],
  };
}

function main() {
  const { command, args } = tauriInvocation(process.argv.slice(2));
  const result = spawnSync(command, args, {
    stdio: "inherit",
  });
  if (result.error) {
    console.error(`run-tauri: failed to run ${command}: ${result.error.message}`);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
