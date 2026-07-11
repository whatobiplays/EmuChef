#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const REQUIRED_PRODUCTION_DIRECTIVES = {
  "base-uri": ["'none'"],
  "connect-src": ["ipc:", "http://ipc.localhost"],
  "default-src": ["'self'"],
  "font-src": ["'self'"],
  "form-action": ["'self'"],
  "frame-ancestors": ["'none'"],
  "img-src": ["'self'", "data:"],
  "object-src": ["'none'"],
  "script-src": ["'self'"],
  "style-src": ["'self'"],
};

export function parseCsp(value) {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error("CSP must be a non-empty string");
  }
  const directives = new Map();
  for (const rawDirective of value.split(";")) {
    const tokens = rawDirective.trim().split(/\s+/).filter(Boolean);
    if (tokens.length === 0) {
      continue;
    }
    const [name, ...sources] = tokens;
    if (directives.has(name)) {
      throw new Error(`CSP directive '${name}' is duplicated`);
    }
    directives.set(name, sources);
  }
  return directives;
}

function sameSources(actual, expected) {
  return actual.length === expected.length && expected.every((source) => actual.includes(source));
}

export function validateProductionCsp(value) {
  const directives = parseCsp(value);
  for (const [name, expectedSources] of Object.entries(REQUIRED_PRODUCTION_DIRECTIVES)) {
    const actualSources = directives.get(name);
    if (!actualSources || !sameSources(actualSources, expectedSources)) {
      throw new Error(
        `production CSP directive '${name}' must contain only ${expectedSources.join(" ")}`,
      );
    }
  }
  for (const [name, sources] of directives) {
    for (const source of sources) {
      if (source === "*" || source.includes("*")) {
        throw new Error(`production CSP directive '${name}' contains a wildcard source`);
      }
      if (source === "'unsafe-eval'" || source === "'unsafe-inline'") {
        throw new Error(`production CSP directive '${name}' contains ${source}`);
      }
    }
  }
  return directives;
}

export function validateDevelopmentConfig(config) {
  if (config.build?.devUrl !== "http://localhost:5173") {
    throw new Error("development config must use the maintained Vite URL");
  }
  if (config.build?.beforeDevCommand !== "npm run sidecar:dev && npm run dev") {
    throw new Error("development config must prepare the sidecar and start Vite");
  }
  if (config.app?.security?.csp !== undefined) {
    throw new Error("development config must not override the production CSP");
  }
  const directives = parseCsp(config.app?.security?.devCsp);
  const allSources = [...directives.values()].flat();
  if (allSources.includes("*") || allSources.includes("'unsafe-eval'")) {
    throw new Error("development CSP must not contain wildcard or unsafe-eval sources");
  }
  const styleSources = directives.get("style-src") ?? [];
  if (!sameSources(styleSources, ["'self'", "'unsafe-inline'"])) {
    throw new Error("development inline allowance must be limited to styles");
  }
  const scriptSources = directives.get("script-src") ?? [];
  if (!sameSources(scriptSources, ["'self'"])) {
    throw new Error("development script-src must remain self-only");
  }
  const connectSources = directives.get("connect-src") ?? [];
  if (
    !sameSources(connectSources, [
      "'self'",
      "ipc:",
      "http://ipc.localhost",
      "ws://localhost:5173",
    ])
  ) {
    throw new Error("development connect-src must be limited to self, Tauri IPC, and Vite HMR");
  }
}

export function validateConfigFiles(productionConfig, developmentConfig) {
  if (productionConfig.build?.devUrl !== undefined || productionConfig.build?.beforeDevCommand !== undefined) {
    throw new Error("production Tauri config must not contain development build settings");
  }
  if (productionConfig.app?.security?.devCsp !== undefined) {
    throw new Error("production Tauri config must not contain a development CSP");
  }
  if (/localhost:5173|127\.0\.0\.1:[0-9]+/.test(JSON.stringify(productionConfig))) {
    throw new Error("production Tauri config contains a development-server URL");
  }
  validateProductionCsp(productionConfig.app?.security?.csp);
  validateDevelopmentConfig(developmentConfig);
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function main() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const tauriDir = path.resolve(scriptDir, "../src-tauri");
  validateConfigFiles(
    readJson(path.join(tauriDir, "tauri.conf.json")),
    readJson(path.join(tauriDir, "tauri.dev.conf.json")),
  );
  console.log("check-tauri-csp: production and development CSP boundaries passed.");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`check-tauri-csp: ${error.message}`);
    process.exit(1);
  }
}

