import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  canonicalFullBytes,
  canonicalUnsignedBytes,
  finalizeManifest,
  parseExactManifest,
  prepareUnsignedManifest,
  rejectPathLeakage,
  validateDmgPathPrefix,
  validateDmgUrl,
  validateManifest,
  validateProductionTrust,
  verifyMetadataSignature,
} from "./macos-update-manifest-policy.mjs";

const appDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const trust = JSON.parse(fs.readFileSync(path.join(appDir, "tests/fixtures/update-trust.json"), "utf8"));
const seed = Buffer.from(Array.from({ length: 32 }, (_, index) => index + 1));
const privateKey = crypto.createPrivateKey({
  key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]),
  format: "der",
  type: "pkcs8",
});
const TEST_NOW = Date.parse("2026-07-15T00:00:00Z");

function prepareInput(overrides = {}) {
  return {
    version: "9.0.0",
    publishedAt: "2026-07-14T20:00:00Z",
    expiresAt: "2026-07-20T20:00:00Z",
    notes: "Quote \" slash \\\\ controls\n\t café 雪",
    dmgUrl: "https://downloads.example.test/emuchef/stable/EmuChef-9.0.0.dmg",
    dmgSizeBytes: 123456,
    dmgSha256: "a".repeat(64),
    minimumMacosVersion: "14.0.0",
    ...overrides,
  };
}

function unsigned(overrides = {}) {
  return prepareUnsignedManifest(prepareInput(overrides), trust, { now: TEST_NOW });
}

function signed(value = unsigned()) {
  const signature = crypto.sign(null, canonicalUnsignedBytes(value), privateKey).toString("hex");
  return { ...value, metadataSignature: signature };
}

test("Node canonical bytes match the Rust-consumed golden exactly", () => {
  const fixture = fs.readFileSync(path.join(appDir, "tests/fixtures/update-manifest-canonical.hex"), "utf8").trim();
  const manifest = { ...unsigned(), metadataSignature: "0".repeat(128) };
  assert.equal(canonicalFullBytes(manifest).toString("hex"), fixture);
  assert.equal(canonicalFullBytes(manifest).at(-1), "}".charCodeAt(0));
  assert.equal(canonicalFullBytes(manifest).subarray(0, 3).equals(Buffer.from([0xef, 0xbb, 0xbf])), false);
  const omitted = unsigned({ minimumMacosVersion: undefined, notes: "" });
  assert.equal(canonicalUnsignedBytes(omitted).includes(Buffer.from("minimumMacosVersion")), false);
});

test("external Ed25519 signatures finalize and verify", () => {
  const value = unsigned();
  const signature = crypto.sign(null, canonicalUnsignedBytes(value), privateKey).toString("hex");
  const bytes = finalizeManifest(value, `${signature}\n`, trust);
  const manifest = parseExactManifest(bytes);
  assert.deepEqual(verifyMetadataSignature(manifest, trust), manifest);
  assert.throws(() => finalizeManifest({ ...value, notes: "tampered" }, signature, trust), /verification failed/);
  assert.throws(() => finalizeManifest(value, signature.toUpperCase(), trust), /lowercase hex/);
});

test("fixed JSON rejects byte-different equivalents, unknowns, duplicates, escaped names, and unsafe numbers", () => {
  const exact = canonicalFullBytes(signed());
  assert.equal(parseExactManifest(exact).version, "9.0.0");
  for (const bytes of [
    Buffer.concat([exact, Buffer.from("\n")]),
    Buffer.from(exact.toString("utf8").replace(":1,", ": 1,")),
    Buffer.from('{"schemaVersion":1,"schemaVersion":1}'),
    Buffer.from('{"schema\\u0056ersion":1}'),
    Buffer.from('{"schemaVersion":1.0}'),
    Buffer.from('{"schemaVersion":1e0}'),
    Buffer.from('{"schemaVersion":-0}'),
    Buffer.from('{"schemaVersion":01}'),
    Buffer.from('{"schemaVersion":9007199254740992}'),
    Buffer.from([0xff]),
  ]) assert.throws(() => parseExactManifest(bytes));
  assert.throws(() => parseExactManifest(Buffer.from(`${exact.toString("utf8").slice(0, -1)},"unknown":1}`)));
});

test("manifest policy binds product, stable target, validity, size, hash, and URL", () => {
  const valid = signed();
  assert.equal(validateManifest(valid, { trust, now: Date.parse("2026-07-15T00:00:00Z") }), valid);
  for (const changed of [
    { product: "other" },
    { channel: "beta" },
    { architecture: "x86_64" },
    { version: "9.0.0-beta.1" },
    { dmgSizeBytes: 0 },
    { dmgSizeBytes: 512 * 1024 * 1024 + 1 },
    { dmgSha256: "A".repeat(64) },
    { dmgUrl: "https://evil.test/file.dmg" },
    { minimumMacosVersion: "" },
  ]) assert.throws(() => validateManifest({ ...valid, ...changed }, { trust, now: Date.parse("2026-07-15T00:00:00Z") }));
});

test("production trust is minimal when disabled and rejects fixture authority", () => {
  assert.equal(validateProductionTrust({ schemaVersion: 1, configured: false }, [trust]), null);
  assert.throws(() => validateProductionTrust({ schemaVersion: 1, configured: false, manifestUrl: "" }, [trust]));
  assert.throws(() => validateProductionTrust(trust, [trust]), /fixture key IDs/);
  assert.throws(() => validateProductionTrust({ ...trust, metadataKeyId: "production-1" }, [trust]), /fixture public keys/);
});

test("release metadata rejects local path leakage", () => {
  assert.doesNotThrow(() => rejectPathLeakage(unsigned()));
  assert.throws(() => rejectPathLeakage({ notes: "see /Users/operator/build" }));
  assert.throws(() => rejectPathLeakage({ notes: "file:///tmp/release" }));
  assert.throws(() => rejectPathLeakage({ notes: "secret-root" }, ["secret-root"]));
});

test("string, integer, timestamp, and validity edges fail closed", () => {
  const valid = signed();
  const now = Date.parse("2026-07-15T00:00:00Z");
  for (const changed of [
    { schemaVersion: "1" },
    { schemaVersion: -1 },
    { schemaVersion: Number.MAX_SAFE_INTEGER + 1 },
    { product: "bad\ud800" },
    { metadataSignature: "0" },
    { notes: "x".repeat(16 * 1024 + 1) },
    { publishedAt: "2026-07-14T20:00:00.000Z" },
    { publishedAt: "not-a-date" },
    { publishedAt: "2026-07-16T00:00:01Z" },
    { expiresAt: "2026-07-14T19:00:00Z" },
    { expiresAt: "2026-08-20T20:00:00Z" },
  ]) assert.throws(() => validateManifest({ ...valid, ...changed }, { trust, now }));
  const missing = { ...valid };
  delete missing.notes;
  assert.throws(() => validateManifest(missing, { trust, now }));
});

test("every URL authority component is fixed by source trust", () => {
  const valid = signed();
  const now = Date.parse("2026-07-15T00:00:00Z");
  for (const dmgUrl of [
    "http://downloads.example.test/emuchef/stable/file.dmg",
    "https://user@downloads.example.test/emuchef/stable/file.dmg",
    "https://downloads.example.test/other/file.dmg",
    "https://downloads.example.test/emuchef/stable/file.zip",
    "https://downloads.example.test/emuchef/stable/file.dmg?token=x",
    "https://downloads.example.test/emuchef/stable/file.dmg#fragment",
    "https://downloads.example.test/emuchef/stable/%2e%2e/file.dmg",
    "not a URL",
  ]) assert.throws(() => validateManifest({ ...valid, dmgUrl }, { trust, now }));
});

test("DMG path prefixes and candidate matching are segment-boundary safe", () => {
  assert.equal(validateDmgPathPrefix("/emuchef/"), "/emuchef/");
  assert.equal(validateDmgPathPrefix("/"), "/");
  assert.doesNotThrow(() => validateDmgUrl(
    "https://downloads.example.test/emuchef/nested/EmuChef-9.0.0.dmg",
    { ...trust, dmgPathPrefix: "/emuchef/" },
  ));
  for (const prefix of [
    null,
    "emuchef/",
    "/emuchef",
    "/emuchef/../stable/",
    "/emuchef/./stable/",
    "/emuchef/%2e%2e/",
    "/emuchef/%2Fstable/",
    "/emuchef/%5cstable/",
    "/emuchef/%252e%252e/",
    "/emuchef?channel=stable/",
    "/emuchef/#stable/",
    "/emuchef//stable/",
    "/emuchef\\stable/",
    "/emu\u0001chef/",
  ]) assert.throws(() => validateDmgPathPrefix(prefix));
  for (const candidate of [
    "https://downloads.example.test/emuchef-evil/file.dmg",
    "https://downloads.example.test/emuchef2/file.dmg",
    "https://downloads.example.test/emuchef",
    "https://downloads.example.test/emuchef/%2e%2e/file.dmg",
    "https://downloads.example.test/emuchef/nested/../file.dmg",
    "https://downloads.example.test/emuchef%2ffile.dmg",
    "https://downloads.example.test/emuchef/%252e%252e/file.dmg",
    "https://downloads.example.test/emuchef//file.dmg",
    "https://downloads.example.test/emuchef/./file.dmg",
    "https://downloads.example.test/emuchef\\file.dmg",
    "https://downloads.example.test",
  ]) assert.throws(() => validateDmgUrl(candidate, { ...trust, dmgPathPrefix: "/emuchef/" }));
});

test("configured production trust rejects non-normalized DMG path prefixes", () => {
  const production = { ...trust, metadataKeyId: "production-metadata-2026" };
  assert.doesNotThrow(() => validateProductionTrust(production));
  for (const dmgPathPrefix of ["emuchef/", "/emuchef", "/emuchef/../", "/emuchef/%2e/", "/emuchef?x/"]) {
    assert.throws(() => validateProductionTrust({ ...production, dmgPathPrefix }));
  }
});

test("prepare validates publication and expiry against the injected policy clock", () => {
  const now = Date.parse("2026-07-15T00:00:00Z");
  assert.doesNotThrow(() => prepareUnsignedManifest(prepareInput({
    publishedAt: "2026-07-14T23:59:00Z",
    expiresAt: "2026-07-20T00:00:00Z",
  }), trust, { now }));
  for (const timestamps of [
    { publishedAt: "2026-07-15T00:10:01Z", expiresAt: "2026-07-20T00:00:00Z" },
    { publishedAt: "2026-07-14T00:00:00Z", expiresAt: "2026-07-14T23:59:59Z" },
    { publishedAt: "2026-07-16T00:00:00Z", expiresAt: "2026-07-15T12:00:00Z" },
    { publishedAt: "2026-07-14T00:00:00Z", expiresAt: "2026-08-14T00:00:01Z" },
  ]) assert.throws(() => prepareUnsignedManifest(prepareInput(timestamps), trust, { now }));
});

test("production prepare uses the actual current clock by default", () => {
  const current = Date.now();
  const isoSeconds = (milliseconds) => new Date(Math.floor(milliseconds / 1000) * 1000)
    .toISOString().replace(".000Z", "Z");
  assert.doesNotThrow(() => prepareUnsignedManifest(prepareInput({
    publishedAt: isoSeconds(current - 60_000),
    expiresAt: isoSeconds(current + 24 * 60 * 60 * 1000),
  }), trust));
  assert.throws(() => prepareUnsignedManifest(prepareInput({
    publishedAt: isoSeconds(current + 11 * 60 * 1000),
    expiresAt: isoSeconds(current + 2 * 24 * 60 * 60 * 1000),
  }), trust));
});

test("production trust rejects unknown schema, insecure endpoints, and malformed keys", () => {
  assert.throws(() => validateProductionTrust(null));
  assert.throws(() => validateProductionTrust({ schemaVersion: 2, configured: false }));
  assert.throws(() => validateProductionTrust({ schemaVersion: 1, configured: false, unknown: true }));
  assert.throws(() => validateProductionTrust({ ...trust, metadataKeyId: "production-1", metadataPublicKey: "0".repeat(64), manifestUrl: "http://updates.test/file" }));
  assert.throws(() => validateProductionTrust({ ...trust, metadataKeyId: "production-1", metadataPublicKey: "bad" }));
});

test("wrong metadata keys and malformed signature inputs fail", () => {
  const value = unsigned();
  const signature = crypto.sign(null, canonicalUnsignedBytes(value), privateKey).toString("hex");
  assert.throws(() => finalizeManifest(value, null, trust));
  assert.throws(() => finalizeManifest(value, `${signature}\n\n`, trust));
  assert.throws(() => verifyMetadataSignature({ ...value, metadataSignature: signature }, {
    ...trust,
    metadataPublicKey: "0".repeat(64),
  }));
  assert.throws(() => parseExactManifest(Buffer.alloc(0)));
  assert.throws(() => parseExactManifest(Buffer.from([0xef, 0xbb, 0xbf, 0x7b, 0x7d])));
  assert.throws(() => parseExactManifest(Buffer.from("{")));
});
