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

function main() {
  const command = process.platform === "win32" ? "tauri.cmd" : "tauri";
  const result = spawnSync(command, tauriArgs(process.argv.slice(2)), {
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
