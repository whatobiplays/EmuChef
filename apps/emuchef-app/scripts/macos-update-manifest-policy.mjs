/**
 * Pure policy for the Phase 4B fixed-JSON update manifest.
 *
 * Object insertion order below is the serialization contract. JSON.stringify
 * supplies the single standard JSON string-escaping implementation used by the
 * Node release tooling; Rust serde_json parity is enforced by a shared golden.
 */
import crypto from "node:crypto";

export const MANIFEST_SCHEMA_VERSION = 1;
export const MAX_SAFE_INTEGER = 9_007_199_254_740_991;
export const MAX_DMG_SIZE = 512 * 1024 * 1024;
export const MAX_NOTES_BYTES = 16 * 1024;
export const SIGNED_FIELD_ORDER = [
  "schemaVersion", "product", "channel", "platform", "architecture", "version",
  "publishedAt", "expiresAt", "notes", "dmgUrl", "dmgSizeBytes", "dmgSha256",
  "minimumMacosVersion", "metadataKeyId",
];
export const FULL_FIELD_ORDER = [...SIGNED_FIELD_ORDER, "metadataSignature"];

function fail(message = "update manifest is invalid") {
  throw new Error(message);
}

function exactInteger(value, name) {
  if (!Number.isSafeInteger(value) || value < 0 || value > MAX_SAFE_INTEGER) {
    fail(`${name} must be an exact non-negative integer`);
  }
  return value;
}

function exactString(value, name) {
  if (typeof value !== "string" || /[\uD800-\uDFFF]/u.test(value)) fail(`${name} must be a valid string`);
  return value;
}

function ordered(manifest, includeSignature) {
  const output = {
    schemaVersion: exactInteger(manifest.schemaVersion, "schemaVersion"),
    product: exactString(manifest.product, "product"),
    channel: exactString(manifest.channel, "channel"),
    platform: exactString(manifest.platform, "platform"),
    architecture: exactString(manifest.architecture, "architecture"),
    version: exactString(manifest.version, "version"),
    publishedAt: exactString(manifest.publishedAt, "publishedAt"),
    expiresAt: exactString(manifest.expiresAt, "expiresAt"),
    notes: exactString(manifest.notes, "notes"),
    dmgUrl: exactString(manifest.dmgUrl, "dmgUrl"),
    dmgSizeBytes: exactInteger(manifest.dmgSizeBytes, "dmgSizeBytes"),
    dmgSha256: exactString(manifest.dmgSha256, "dmgSha256"),
  };
  if (manifest.minimumMacosVersion !== undefined) {
    output.minimumMacosVersion = exactString(manifest.minimumMacosVersion, "minimumMacosVersion");
  }
  output.metadataKeyId = exactString(manifest.metadataKeyId, "metadataKeyId");
  if (includeSignature) output.metadataSignature = exactString(manifest.metadataSignature, "metadataSignature");
  return output;
}

export function canonicalUnsignedBytes(manifest) {
  return Buffer.from(JSON.stringify(ordered(manifest, false)), "utf8");
}

export function canonicalFullBytes(manifest) {
  return Buffer.from(JSON.stringify(ordered(manifest, true)), "utf8");
}

function validateLexicalJson(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.subarray(0, 3).equals(Buffer.from([0xef, 0xbb, 0xbf]))) fail();
  const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  const keys = new Set();
  let index = 0;
  while (index < text.length) {
    if (text[index] === '"') {
      const start = ++index;
      let escaped = false;
      while (index < text.length && text[index] !== '"') {
        if (text[index] === "\\") { escaped = true; index += 2; } else index += 1;
      }
      if (index >= text.length) fail();
      let after = index + 1;
      while (/\s/u.test(text[after] ?? "")) after += 1;
      if (text[after] === ":") {
        if (escaped) fail("escaped field names are forbidden");
        const key = text.slice(start, index);
        if (keys.has(key)) fail("duplicate fields are forbidden");
        keys.add(key);
      }
      index += 1;
      continue;
    }
    if (text[index] === "-" || /[0-9]/u.test(text[index])) {
      const start = index++;
      while (index < text.length && !/[,}\]\s]/u.test(text[index])) index += 1;
      const token = text.slice(start, index);
      if (!/^(0|[1-9][0-9]*)$/u.test(token) || BigInt(token) > BigInt(MAX_SAFE_INTEGER)) fail("inexact number");
      continue;
    }
    index += 1;
  }
  return text;
}

export function parseExactManifest(bytes) {
  const text = validateLexicalJson(bytes);
  let parsed;
  try { parsed = JSON.parse(text); } catch { fail(); }
  const allowed = new Set(FULL_FIELD_ORDER);
  if (Object.keys(parsed).some((key) => !allowed.has(key))) fail("unknown field");
  validateManifest(parsed);
  if (!canonicalFullBytes(parsed).equals(bytes)) fail("manifest bytes are not canonical");
  return parsed;
}

function parseTimestamp(value) {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/u.test(value)) fail("timestamp must be exact UTC seconds");
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds) || new Date(milliseconds).toISOString() !== value.replace("Z", ".000Z")) fail("timestamp is invalid");
  return milliseconds;
}

function validateSemver(value, { allowCurrent = false } = {}) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u.test(value)) fail("version must be stable SemVer");
  if (!allowCurrent && value === "0.0.0") fail("version must be releasable");
}

function pathIsNotNormalized(value, requireTrailingSlash) {
  return !value.startsWith("/")
    || (requireTrailingSlash && !value.endsWith("/"))
    || value.includes("//")
    || value.includes("\\")
    || /[\u0000-\u001f\u007f]/u.test(value)
    || /%2f|%5c|%2e|%25/iu.test(value)
    || value.split("/").some((segment) => segment === "." || segment === "..");
}

export function validateDmgPathPrefix(value) {
  if (typeof value !== "string" || value.includes("?") || value.includes("#")
    || pathIsNotNormalized(value, true)) fail("DMG path prefix is not normalized");
  return value;
}

function rawUrlPath(value) {
  const scheme = value.indexOf("://");
  const pathStart = scheme === -1 ? -1 : value.indexOf("/", scheme + 3);
  if (pathStart === -1) return null;
  return value.slice(pathStart).split(/[?#]/u, 1)[0];
}

export function validateDmgUrl(value, trust) {
  const prefix = validateDmgPathPrefix(trust.dmgPathPrefix);
  let url;
  try { url = new URL(value); } catch { fail("DMG URL is invalid"); }
  const origin = new URL(trust.dmgOrigin);
  const deliveredPath = rawUrlPath(value);
  if (url.protocol !== "https:" || url.username || url.password || url.origin !== origin.origin
    || deliveredPath !== url.pathname || !url.pathname.startsWith(prefix) || !url.pathname.endsWith(".dmg")
    || url.search || url.hash || pathIsNotNormalized(url.pathname, false)) fail("DMG URL is outside policy");
}

export function validateManifest(manifest, { trust, now = Date.now() } = {}) {
  const required = FULL_FIELD_ORDER.filter((field) => field !== "minimumMacosVersion");
  if (required.some((field) => !(field in manifest))) fail("required field missing");
  ordered(manifest, true);
  if (manifest.schemaVersion !== MANIFEST_SCHEMA_VERSION
    || manifest.product !== "com.emuchef.desktop" || manifest.channel !== "stable"
    || manifest.platform !== "darwin" || manifest.architecture !== "aarch64") fail("release target mismatch");
  validateSemver(manifest.version);
  if (Buffer.byteLength(manifest.notes, "utf8") > MAX_NOTES_BYTES || manifest.notes.includes("\0")) fail("notes are too large");
  if (manifest.dmgSizeBytes < 1 || manifest.dmgSizeBytes > MAX_DMG_SIZE) fail("DMG size is outside policy");
  if (!/^[0-9a-f]{64}$/u.test(manifest.dmgSha256)) fail("DMG SHA-256 is invalid");
  if (!/^[0-9a-f]{128}$/u.test(manifest.metadataSignature)) fail("metadata signature is invalid");
  if (manifest.minimumMacosVersion !== undefined) {
    if (manifest.minimumMacosVersion === "") fail("minimum macOS version cannot be empty");
    validateSemver(manifest.minimumMacosVersion, { allowCurrent: true });
  }
  const published = parseTimestamp(manifest.publishedAt);
  const expires = parseTimestamp(manifest.expiresAt);
  if (published > now + 10 * 60 * 1000 || expires <= now || expires <= published || expires - published > 30 * 24 * 60 * 60 * 1000) fail("manifest validity window is invalid");
  if (trust) {
    if (manifest.metadataKeyId !== trust.metadataKeyId) fail("metadata key ID mismatch");
    validateDmgUrl(manifest.dmgUrl, trust);
  }
  return manifest;
}

function publicKeyFromRawHex(rawHex) {
  if (!/^[0-9a-f]{64}$/u.test(rawHex)) fail("metadata public key is invalid");
  const prefix = Buffer.from("302a300506032b6570032100", "hex");
  return crypto.createPublicKey({ key: Buffer.concat([prefix, Buffer.from(rawHex, "hex")]), format: "der", type: "spki" });
}

export function verifyMetadataSignature(manifest, trust) {
  validateManifest(manifest, { trust });
  const verified = crypto.verify(
    null,
    canonicalUnsignedBytes(manifest),
    publicKeyFromRawHex(trust.metadataPublicKey),
    Buffer.from(manifest.metadataSignature, "hex"),
  );
  if (!verified) fail("metadata signature verification failed");
  return manifest;
}

export function validateProductionTrust(trust, fixtureTrusts = []) {
  const allowed = new Set(["schemaVersion", "configured", "manifestUrl", "dmgOrigin", "dmgPathPrefix", "metadataKeyId", "metadataPublicKey"]);
  if (!trust || Object.keys(trust).some((key) => !allowed.has(key)) || trust.schemaVersion !== 1) fail("production trust is invalid");
  if (!trust.configured) {
    if (Object.keys(trust).length !== 2) fail("unconfigured production trust must contain two fields");
    return null;
  }
  let manifestUrl;
  let dmgOrigin;
  try {
    manifestUrl = new URL(trust.manifestUrl);
    dmgOrigin = new URL(trust.dmgOrigin);
  } catch { fail("production endpoints are invalid"); }
  if (manifestUrl.protocol !== "https:" || manifestUrl.username || manifestUrl.password
    || manifestUrl.search || manifestUrl.hash || dmgOrigin.protocol !== "https:"
    || dmgOrigin.username || dmgOrigin.password || dmgOrigin.pathname !== "/"
    || dmgOrigin.search || dmgOrigin.hash) fail("production endpoints must use fixed HTTPS URLs");
  validateDmgPathPrefix(trust.dmgPathPrefix);
  if (/^(?:test|fixture)[-_]/iu.test(trust.metadataKeyId)) fail("fixture key IDs are forbidden in production");
  if (fixtureTrusts.some((fixture) => fixture.metadataPublicKey === trust.metadataPublicKey)) fail("fixture public keys are forbidden in production");
  publicKeyFromRawHex(trust.metadataPublicKey);
  validateDmgUrl(`${trust.dmgOrigin}${trust.dmgPathPrefix}probe.dmg`, trust);
  return trust;
}

/** Production callers omit `now`; explicit injection is reserved for pure deterministic tests. */
export function prepareUnsignedManifest(input, trust, { now = Date.now() } = {}) {
  const manifest = {
    schemaVersion: 1,
    product: "com.emuchef.desktop",
    channel: "stable",
    platform: "darwin",
    architecture: "aarch64",
    version: input.version,
    publishedAt: input.publishedAt,
    expiresAt: input.expiresAt,
    notes: input.notes,
    dmgUrl: input.dmgUrl,
    dmgSizeBytes: input.dmgSizeBytes,
    dmgSha256: input.dmgSha256,
    ...(input.minimumMacosVersion === undefined ? {} : { minimumMacosVersion: input.minimumMacosVersion }),
    metadataKeyId: trust.metadataKeyId,
    metadataSignature: "0".repeat(128),
  };
  validateManifest(manifest, { trust, now });
  const { metadataSignature: _excluded, ...unsigned } = ordered(manifest, true);
  return unsigned;
}

export function finalizeManifest(unsigned, signatureText, trust) {
  if (typeof signatureText !== "string" || !/^[0-9a-f]{128}\n?$/u.test(signatureText)) fail("signature file must contain lowercase hex");
  const manifest = { ...unsigned, metadataSignature: signatureText.trimEnd() };
  verifyMetadataSignature(manifest, trust);
  return canonicalFullBytes(manifest);
}

export function rejectPathLeakage(value, forbiddenValues = []) {
  const serialized = JSON.stringify(value);
  if (/file:\/\/|\/(?:Users|home|private|tmp)\/|[A-Za-z]:\\/u.test(serialized)
    || forbiddenValues.some((item) => item && serialized.includes(item))) fail("release metadata contains a local path");
}
