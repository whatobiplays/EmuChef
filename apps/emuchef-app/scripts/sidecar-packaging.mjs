export const BINARY_BASENAME = "emuchef";

export function validateTargetTriple(value) {
  if (typeof value !== "string" || !/^[A-Za-z0-9_.-]+$/.test(value) || value.split("-").length < 3) {
    throw new Error(`Unexpected target triple '${value}'`);
  }
  return value;
}

export function externalBinArtifactName(targetTriple) {
  return `${BINARY_BASENAME}-${targetTriple}${targetTriple.includes("windows") ? ".exe" : ""}`;
}
