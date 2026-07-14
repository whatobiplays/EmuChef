export const BINARY_BASENAME = "emuchef";
export const QUALIFIED_MACOS_TARGET = "aarch64-apple-darwin";

export function validateTargetTriple(value, source = "target triple") {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    !/^[A-Za-z0-9_.-]+$/.test(value) ||
    value.split("-").length < 3 ||
    value.includes("/") ||
    value.includes("\\") ||
    /\s/.test(value)
  ) {
    throw new Error(`${source} returned unexpected target triple '${value}'`);
  }
  return value;
}

export function macosArchitecture(targetTriple) {
  const target = validateTargetTriple(targetTriple);
  if (target === "aarch64-apple-darwin") return "arm64";
  if (target === "x86_64-apple-darwin") return "x86_64";
  if (target === "universal-apple-darwin") return "universal";
  throw new Error(`Unsupported macOS target triple '${target}'`);
}

export function requireQualifiedMacosTarget(targetTriple) {
  const architecture = macosArchitecture(targetTriple);
  if (targetTriple !== QUALIFIED_MACOS_TARGET) {
    throw new Error(
      `Phase 3E qualifies only '${QUALIFIED_MACOS_TARGET}', not '${targetTriple}' (${architecture})`,
    );
  }
  return architecture;
}

export function externalBinArtifactName(targetTriple) {
  return `${BINARY_BASENAME}-${targetTriple}${targetTriple.includes("windows") ? ".exe" : ""}`;
}
