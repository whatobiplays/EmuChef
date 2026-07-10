export const BINARY_BASENAME = "emuchef";

export function validateTargetTriple(value, source) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${source} returned an empty target triple`);
  }
  if (!/^[A-Za-z0-9_.-]+$/.test(value) || value.split("-").length < 3) {
    throw new Error(`${source} returned unexpected target triple '${value}'`);
  }
  if (value.includes("/") || value.includes("\\") || /\s/.test(value)) {
    throw new Error(`${source} returned unsafe target triple '${value}'`);
  }
  return value;
}

export function isWindowsTargetTriple(targetTriple) {
  return targetTriple.includes("windows");
}

export function binaryExtensionForTargetTriple(targetTriple) {
  return isWindowsTargetTriple(targetTriple) ? ".exe" : "";
}

export function externalBinArtifactName(targetTriple) {
  return `${BINARY_BASENAME}-${targetTriple}${binaryExtensionForTargetTriple(targetTriple)}`;
}

export function packagedBinaryNameForTargetTriple(targetTriple) {
  return `${BINARY_BASENAME}${binaryExtensionForTargetTriple(targetTriple)}`;
}
