import {
  targetExecutableExtension,
  validateTargetTriple,
} from "../../../scripts/sidecar-target-contract.mjs";

export const BINARY_BASENAME = "emuchef";
export const QUALIFIED_MACOS_TARGET = "aarch64-apple-darwin";

export { validateTargetTriple };

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
  const target = validateTargetTriple(targetTriple);
  return `${BINARY_BASENAME}-${target}${targetExecutableExtension(target)}`;
}
