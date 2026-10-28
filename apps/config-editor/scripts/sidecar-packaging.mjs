import {
  targetExecutableExtension,
  validateTargetTriple,
} from "../../../scripts/sidecar-target-contract.mjs";

export const BINARY_BASENAME = "emuchef";

export { validateTargetTriple };

export function isWindowsTargetTriple(targetTriple) {
  return binaryExtensionForTargetTriple(targetTriple) === ".exe";
}

export function binaryExtensionForTargetTriple(targetTriple) {
  return targetExecutableExtension(targetTriple);
}

export function externalBinArtifactName(targetTriple) {
  return `${BINARY_BASENAME}-${targetTriple}${binaryExtensionForTargetTriple(targetTriple)}`;
}

export function packagedBinaryNameForTargetTriple(targetTriple) {
  return `${BINARY_BASENAME}${binaryExtensionForTargetTriple(targetTriple)}`;
}
