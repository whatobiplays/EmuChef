/**
 * Pure, security-critical macOS packaging policy.
 *
 * This module is the production policy consumed by the OS-backed packaging
 * adapter. Its complete line, branch, and function coverage is enforced at a
 * 95% minimum independently from command execution and filesystem traversal.
 */
import crypto from "node:crypto";

import {
  QUALIFIED_MACOS_TARGET,
  requireQualifiedMacosTarget,
  validateTargetTriple,
} from "./sidecar-packaging.mjs";

export const QUALIFICATION_POLICY_VERSION = 1;
export const REQUIRED_CATALOG_DIRECTORIES = Object.freeze([
  "apps",
  "device_plans",
  "device_profiles",
  "recipes",
]);
export const APPLE_VARIABLE_ALLOWLIST = Object.freeze([
  "APPLE_SIGNING_IDENTITY",
  "APPLE_API_ISSUER",
  "APPLE_API_KEY",
  "APPLE_API_KEY_PATH",
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_TEAM_ID",
  "APPLE_PROVIDER_SHORT_NAME",
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
]);

const API_NOTARY_VARIABLES = ["APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH"];
const APPLE_ID_NOTARY_VARIABLES = ["APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"];

function nonEmpty(env, name) {
  return typeof env[name] === "string" && env[name].trim() !== "";
}

/**
 * Produces a local build environment without inspecting or validating Apple
 * credential values. Fixed Tauri variable names are removed and ad-hoc signing
 * is selected explicitly, so ambient credentials cannot select release mode.
 */
export function localBuildEnvironment(env) {
  const child = { ...env };
  for (const name of APPLE_VARIABLE_ALLOWLIST) delete child[name];
  child.APPLE_SIGNING_IDENTITY = "-";
  return child;
}

/** Validates only the documented allowlist after developer-id mode is selected. */
export function validateDeveloperIdEnvironment(env) {
  if (!nonEmpty(env, "APPLE_SIGNING_IDENTITY")) {
    throw new Error("developer-id mode requires APPLE_SIGNING_IDENTITY");
  }
  if (!env.APPLE_SIGNING_IDENTITY.startsWith("Developer ID Application:")) {
    throw new Error("APPLE_SIGNING_IDENTITY must select a Developer ID Application identity");
  }
  const apiPresent = API_NOTARY_VARIABLES.every((name) => nonEmpty(env, name));
  const appleIdPresent = APPLE_ID_NOTARY_VARIABLES.every((name) => nonEmpty(env, name));
  if (!apiPresent && !appleIdPresent) {
    throw new Error(
      "developer-id mode requires either the APPLE_API_* notary set or the APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID set",
    );
  }
  return { authentication: apiPresent ? "app-store-connect-api" : "apple-id" };
}

export function developerIdBuildEnvironment(env) {
  const summary = validateDeveloperIdEnvironment(env);
  return { env: { ...env }, summary };
}

/** Clears caller-dependent compiler settings and installs stable path remaps. */
export function normalizedBuildEnvironment(env, { repoRoot, homeDir, mode }) {
  const child = mode === "developer-id" ? developerIdBuildEnvironment(env).env : localBuildEnvironment(env);
  delete child.RUSTFLAGS;
  delete child.CARGO_BUILD_RUSTFLAGS;
  delete child.TAURI_ENV_DEBUG;
  child.CARGO_ENCODED_RUSTFLAGS = [
    `--remap-path-prefix=${repoRoot}=emuchef-source`,
    `--remap-path-prefix=${homeDir}=user-home`,
  ].join("\x1f");
  return child;
}

export function parseOptions(argv) {
  const options = { mode: "local", positional: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (["--mode", "--app", "--dmg", "--manifest"].includes(argument)) {
      const value = argv[index + 1];
      if (!value) throw new Error(`${argument} requires a value`);
      options[argument.slice(2)] = value;
      index += 1;
    } else if (argument.startsWith("--")) {
      throw new Error(`unknown option '${argument}'`);
    } else {
      options.positional.push(argument);
    }
  }
  if (!new Set(["local", "developer-id"]).has(options.mode)) {
    throw new Error("--mode must be local or developer-id");
  }
  return options;
}

function cargoPackageVersion(cargoToml) {
  const packageSection = cargoToml.match(/\[package\]([\s\S]*?)(?:\n\[|$)/)?.[1] ?? "";
  const version = packageSection.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if (!version) throw new Error("app Cargo.toml must define a package version");
  return version;
}

export function validatePackagingConfiguration({ packageJson, tauriConfig, cargoToml, targetTriple }) {
  const architecture = requireQualifiedMacosTarget(validateTargetTriple(targetTriple, "rustc"));
  const versions = [packageJson.version, tauriConfig.version, cargoPackageVersion(cargoToml)];
  if (versions.some((value) => typeof value !== "string" || value !== versions[0])) {
    throw new Error("package.json, tauri.conf.json, and app Cargo.toml versions must match");
  }
  if (tauriConfig.identifier !== "com.emuchef.desktop" || tauriConfig.productName !== "EmuChef") {
    throw new Error("Tauri product identity is not the qualified EmuChef identity");
  }
  if (tauriConfig.build?.devUrl !== undefined) {
    throw new Error("production Tauri configuration must not define devUrl");
  }
  if (
    JSON.stringify(tauriConfig.app?.security?.csp ?? "").match(
      /(?:https?|wss?):\/\/(?:localhost|127\.0\.0\.1)(?=[:/; ]|$)|unsafe-eval/,
    )
  ) {
    throw new Error("production CSP contains a development or unsafe value");
  }
  if (JSON.stringify(tauriConfig.bundle?.externalBin) !== JSON.stringify(["binaries/emuchef"])) {
    throw new Error("Tauri externalBin must contain only the Rust emuchef sidecar");
  }
  const resourceValues = Object.values(tauriConfig.bundle?.resources ?? {});
  for (const directory of REQUIRED_CATALOG_DIRECTORIES) {
    if (!resourceValues.includes(`catalog/${directory}`)) {
      throw new Error(`Tauri resources omit catalog/${directory}`);
    }
  }
  if (!resourceValues.includes("qualification/qualification-policy.json")) {
    throw new Error("Tauri resources omit qualification policy metadata");
  }
  const macos = tauriConfig.bundle?.macOS ?? {};
  if (macos.signingIdentity !== "-" || macos.hardenedRuntime !== true || macos.minimumSystemVersion !== "11.0") {
    throw new Error("macOS bundle must use ad-hoc default signing, hardened runtime, and macOS 11.0 minimum");
  }
  if (!(tauriConfig.bundle?.icon ?? []).includes("icons/icon.icns")) {
    throw new Error("macOS bundle icon is not configured");
  }
  return { appVersion: versions[0], architecture, targetTriple };
}

export function qualificationPolicy({ appVersion, architecture, targetTriple }) {
  return {
    schemaVersion: 1,
    qualificationPolicyVersion: QUALIFICATION_POLICY_VERSION,
    appVersion,
    targetTriple,
    architecture,
    buildMode: "release",
    realExecutionEnabled: false,
  };
}

export function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

export function sha256Bytes(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

export function semanticDigest(value) {
  return sha256Bytes(canonicalJson(value));
}

export function normalizedContentManifest(content) {
  const normalized = {
    qualificationPolicyVersion: QUALIFICATION_POLICY_VERSION,
    product: content.product,
    target: content.target,
    executables: content.executables,
    infoPlist: content.infoPlist,
    catalog: content.catalog,
    qualificationPolicy: content.qualificationPolicy,
    capabilities: content.capabilities,
    tauriSecurity: content.tauriSecurity,
  };
  return { content: normalized, normalizedContentSha256: semanticDigest(normalized) };
}

export function createReleaseManifest({ normalized, rawArtifacts, provenance, signingState }) {
  return {
    schemaVersion: 1,
    qualificationPolicyVersion: QUALIFICATION_POLICY_VERSION,
    reproducibility: {
      claim: "normalized-content-repeatability",
      byteIdenticalSignedArtifactsClaimed: false,
      rawHashesArePerBuildIdentities: true,
      volatileLayersExcludedFromNormalizedDigest: [
        "code-signature-blobs",
        "signing-and-notarization-timestamps",
        "dmg-container-metadata",
        "filesystem-mtimes",
        "temporary-names",
        "caller-specific-absolute-paths",
      ],
    },
    provenance,
    normalizedContent: normalized,
    rawArtifactIdentities: rawArtifacts,
    verification: {
      bundleContents: "passed",
      cleanEnvironmentSmoke: "passed",
      signing: signingState,
      notarization: "skipped",
      stapling: "skipped",
      gatekeeper: "skipped",
    },
  };
}

export function compareReleaseManifests(left, right) {
  const normalizedMatch =
    left?.normalizedContent?.normalizedContentSha256 ===
    right?.normalizedContent?.normalizedContentSha256;
  return {
    normalizedContentMatches: normalizedMatch,
    rawArtifactHashesMayDiffer: true,
    result: normalizedMatch ? "equivalent-normalized-content" : "meaningful-content-difference",
  };
}

export function validateQualificationProbe(report) {
  if (
    report?.kind !== "macos_packaged_app_qualification" ||
    report?.status !== "passed" ||
    report?.runtimeReady !== true ||
    report?.catalogLoaded !== true ||
    report?.readOnlyCatalogOperation !== true ||
    report?.realExecutionEnabled !== false
  ) {
    throw new Error("packaged qualification probe did not prove the required safe startup state");
  }
  return report;
}

export function assertSafeManifest(manifest, { forbiddenValues = [] } = {}) {
  const serialized = JSON.stringify(manifest);
  const forbiddenPatterns = [
    /\/(?:Users|home)\//,
    /(?:APPLE_PASSWORD|APPLE_API_KEY_PATH|APPLE_CERTIFICATE_PASSWORD)/,
    /(?:deviceSerial|credential|password|privateKey)/i,
  ];
  if (forbiddenPatterns.some((pattern) => pattern.test(serialized))) {
    throw new Error("release manifest contains forbidden private or caller-specific data");
  }
  for (const value of forbiddenValues) {
    if (value && serialized.includes(value)) {
      throw new Error("release manifest contains a credential value");
    }
  }
  return true;
}

export function forbiddenBundleReason(relative) {
  const lower = relative.toLowerCase();
  return /(?:node_modules|\/src\/|src-tauri|platform[-_]tools|latest-darwin|\.map$|\.tsx?$|\.rs$|\.log$|authkey_|\.p8$|\.p12$)/.test(
    lower,
  );
}

export function forbiddenBinaryReason(value) {
  return /(?:\/(?:Users|home)\/[A-Za-z0-9._-]+\/|node_modules|Projects\/EmuChef|http:\/\/localhost:5174|ws:\/\/localhost:5174|sourceMappingURL|unsafe-eval)/.test(
    value,
  );
}

export function requireThinArm64(label, fileDescription) {
  if (!fileDescription.includes("arm64") || fileDescription.includes("universal")) {
    throw new Error(`${label} is not thin arm64`);
  }
}

export function validateInfoPlist(info, tauriConfig) {
  for (const [field, expected] of [
    ["CFBundleIdentifier", tauriConfig.identifier],
    ["CFBundleDisplayName", tauriConfig.productName],
    ["CFBundleShortVersionString", tauriConfig.version],
    ["CFBundleVersion", tauriConfig.version],
    ["LSMinimumSystemVersion", "11.0"],
    ["CFBundlePackageType", "APPL"],
  ]) {
    if (info[field] !== expected) {
      throw new Error(`Info.plist ${field} did not match qualified metadata`);
    }
  }
  return info;
}

export function validatePackagedPolicy(policy, appVersion) {
  if (
    policy.qualificationPolicyVersion !== QUALIFICATION_POLICY_VERSION ||
    policy.realExecutionEnabled !== false ||
    policy.targetTriple !== QUALIFIED_MACOS_TARGET ||
    policy.appVersion !== appVersion
  ) {
    throw new Error("packaged qualification policy is inconsistent or enables real execution");
  }
  return policy;
}

export function classifySigningState(status, output) {
  if (status === 0 && /Signature=adhoc/.test(output)) return "ad-hoc";
  if (status === 0 && /^Authority=Developer ID Application:/m.test(output)) return "developer-id";
  throw new Error("application is neither valid ad-hoc nor Developer ID signed content");
}

export function buildNormalizedContent(verification, capability, tauriConfig, executableHashes) {
  const info = verification.info;
  return normalizedContentManifest({
    product: {
      productName: info.CFBundleDisplayName,
      bundleIdentifier: info.CFBundleIdentifier,
      appVersion: info.CFBundleShortVersionString,
    },
    target: {
      targetTriple: verification.policy.targetTriple,
      architecture: verification.policy.architecture,
      buildMode: verification.policy.buildMode,
    },
    executables: {
      main: {
        relativePath: `Contents/MacOS/${info.CFBundleExecutable}`,
        unsignedContentSha256: executableHashes.main,
      },
      sidecar: {
        relativePath: "Contents/MacOS/emuchef",
        unsignedContentSha256: executableHashes.sidecar,
      },
    },
    infoPlist: {
      CFBundleDisplayName: info.CFBundleDisplayName,
      CFBundleExecutable: info.CFBundleExecutable,
      CFBundleIdentifier: info.CFBundleIdentifier,
      CFBundlePackageType: info.CFBundlePackageType,
      CFBundleShortVersionString: info.CFBundleShortVersionString,
      CFBundleVersion: info.CFBundleVersion,
      LSMinimumSystemVersion: info.LSMinimumSystemVersion,
    },
    catalog: verification.catalog,
    qualificationPolicy: verification.policy,
    capabilities: { semanticSha256: semanticDigest(capability), value: capability },
    tauriSecurity: {
      csp: tauriConfig.app.security.csp,
      externalBin: tauriConfig.bundle.externalBin,
      resources: tauriConfig.bundle.resources,
      macOS: {
        hardenedRuntime: tauriConfig.bundle.macOS.hardenedRuntime,
        minimumSystemVersion: tauriConfig.bundle.macOS.minimumSystemVersion,
      },
    },
  });
}

export function selectArtifacts(apps, dmgs) {
  if (apps.length !== 1 || dmgs.length !== 1) {
    throw new Error("qualification requires exactly one generated app and one DMG");
  }
  return { appPath: apps[0], dmgPath: dmgs[0] };
}

export function validateDeveloperIdMetadata(output) {
  for (const pattern of [/^Authority=Developer ID Application:/m, /^Timestamp=(?!none)/m, /flags=.*runtime/]) {
    if (!pattern.test(output)) throw new Error("application lacks required Developer ID metadata");
  }
  return true;
}
