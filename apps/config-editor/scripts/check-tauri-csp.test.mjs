import assert from "node:assert/strict";
import test from "node:test";

import {
  parseCsp,
  validateConfigFiles,
  validateDevelopmentConfig,
  validateProductionCsp,
} from "./check-tauri-csp.mjs";

const productionCsp =
  "default-src 'self'; base-uri 'none'; connect-src ipc: http://ipc.localhost; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self'; style-src 'self'";
const developmentCsp =
  "default-src 'self'; base-uri 'none'; connect-src 'self' ipc: http://ipc.localhost ws://localhost:5173; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data: blob:; object-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'";

function productionConfig(csp = productionCsp) {
  return {
    build: { beforeBuildCommand: "npm run sidecar:build && npm run build" },
    app: { security: { csp } },
  };
}

function developmentConfig(devCsp = developmentCsp) {
  return {
    build: {
      beforeDevCommand: "npm run sidecar:dev && npm run dev",
      devUrl: "http://localhost:5173",
    },
    app: { security: { devCsp } },
  };
}

test("parses unique CSP directives", () => {
  const directives = parseCsp("default-src 'self'; object-src 'none'");
  assert.deepEqual(directives.get("default-src"), ["'self'"]);
  assert.throws(() => parseCsp(""), /non-empty/);
  assert.throws(() => parseCsp("default-src 'self'; default-src *"), /duplicated/);
});

test("accepts the narrow production policy", () => {
  assert.doesNotThrow(() => validateProductionCsp(productionCsp));
  assert.doesNotThrow(() => validateConfigFiles(productionConfig(), developmentConfig()));
});

test("rejects disabled, wildcard, eval, inline, and broad connect production policies", () => {
  assert.throws(() => validateProductionCsp(null), /non-empty/);
  assert.throws(
    () => validateProductionCsp(productionCsp.replace("default-src 'self'", "default-src *")),
    /default-src/,
  );
  assert.throws(
    () => validateProductionCsp(productionCsp.replace("script-src 'self'", "script-src 'self' 'unsafe-eval'")),
    /script-src/,
  );
  assert.throws(
    () => validateProductionCsp(productionCsp.replace("style-src 'self'", "style-src 'self' 'unsafe-inline'")),
    /style-src/,
  );
  assert.throws(
    () => validateProductionCsp(productionCsp.replace("connect-src ipc: http://ipc.localhost", "connect-src *")),
    /connect-src/,
  );
});

test("keeps Vite allowances development-only and narrowly scoped", () => {
  assert.doesNotThrow(() => validateDevelopmentConfig(developmentConfig()));
  assert.throws(
    () => validateDevelopmentConfig(developmentConfig(developmentCsp.replace("script-src 'self'", "script-src 'self' 'unsafe-eval'"))),
    /unsafe-eval/,
  );
  assert.throws(
    () => validateConfigFiles({ ...productionConfig(), build: { devUrl: "http://localhost:5173" } }, developmentConfig()),
    /development build settings/,
  );
});

