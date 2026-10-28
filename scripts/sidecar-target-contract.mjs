const TARGET_SEGMENT = /^[a-z0-9_]+$/;
const SIDECAR_PREFIX = "emuchef-";

// Rust host tuples are architecture-vendor-system with an optional
// environment. Keeping the final component semantic prevents an unknown name
// such as `aarch64-apple-darwin-extra` from being mistaken for a target triple.
const ENVIRONMENTLESS_SYSTEMS = new Set([
  "aix",
  "android",
  "androideabi",
  "darwin",
  "dragonfly",
  "emscripten",
  "freebsd",
  "fuchsia",
  "haiku",
  "hermit",
  "hurd",
  "illumos",
  "ios",
  "l4re",
  "netbsd",
  "none",
  "openbsd",
  "redox",
  "solaris",
  "solid_asp3",
  "tvos",
  "uefi",
  "visionos",
  "vxworks",
  "wasi",
  "watchos",
]);

const SYSTEM_ENVIRONMENTS = new Map([
  [
    "linux",
    new Set([
      "gnu",
      "gnuabi64",
      "gnueabi",
      "gnueabihf",
      "gnullvm",
      "gnuspe",
      "gnux32",
      "musl",
      "muslabi64",
      "musleabi",
      "musleabihf",
      "muslsf",
      "ohos",
      "uclibc",
    ]),
  ],
  ["ios", new Set(["macabi", "simulator"])],
  ["tvos", new Set(["simulator"])],
  ["visionos", new Set(["simulator"])],
  ["watchos", new Set(["simulator"])],
  ["windows", new Set(["gnu", "gnullvm", "msvc"])],
]);

const KNOWN_TARGET_ARCHITECTURES = new Set([
  "aarch64",
  "aarch64_be",
  "aarch64v8r",
  "amdgcn",
  "arm",
  "arm64_32",
  "arm64e",
  "arm64ec",
  "armeb",
  "armebv7r",
  "armv4t",
  "armv5te",
  "armv6",
  "armv6k",
  "armv7",
  "armv7a",
  "armv7k",
  "armv7r",
  "armv7s",
  "armv8r",
  "avr",
  "bpfeb",
  "bpfel",
  "csky",
  "hexagon",
  "i386",
  "i586",
  "i686",
  "loongarch32",
  "loongarch64",
  "m68k",
  "mips",
  "mips64",
  "mips64el",
  "mipsel",
  "mipsisa32r6",
  "mipsisa32r6el",
  "mipsisa64r6",
  "mipsisa64r6el",
  "msp430",
  "nvptx64",
  "powerpc",
  "powerpc64",
  "powerpc64le",
  "riscv32",
  "riscv32e",
  "riscv32em",
  "riscv32emc",
  "riscv32gc",
  "riscv32i",
  "riscv32im",
  "riscv32ima",
  "riscv32imac",
  "riscv32imafc",
  "riscv32imc",
  "riscv32imfc",
  "riscv64",
  "riscv64a23",
  "riscv64gc",
  "riscv64im",
  "riscv64imac",
  "s390x",
  "sparc",
  "sparc64",
  "sparcv9",
  "thumbv4t",
  "thumbv5te",
  "thumbv6",
  "thumbv6m",
  "thumbv7a",
  "thumbv7em",
  "thumbv7m",
  "thumbv7neon",
  "thumbv7r",
  "thumbv8m.base",
  "thumbv8m.main",
  "thumbv8r",
  "wasm32",
  "wasm32v1",
  "wasm64",
  "x86_64",
  "x86_64h",
  "xtensa",
]);

const KNOWN_TARGET_VENDORS = new Set([
  "amd",
  "apple",
  "esp",
  "esp32",
  "esp32s2",
  "esp32s3",
  "fortanix",
  "ibm",
  "kmc",
  "linux",
  "lynx",
  "mti",
  "nintendo",
  "none",
  "nuttx",
  "nvidia",
  "oe",
  "openwrt",
  "pc",
  "risc0",
  "rtems",
  "sony",
  "sun",
  "unikraft",
  "unknown",
  "uwp",
  "vex",
  "wali",
  "wasip1",
  "wasip2",
  "wasip3",
  "win7",
  "wrs",
]);

/**
 * Returns whether a value has the canonical Rust host-target shape accepted by
 * the sidecar producers. Unknown OS and environment components are rejected so
 * arbitrary suffixes cannot be mistaken for platform targets.
 */
export function isCanonicalTargetTriple(value) {
  if (typeof value !== "string") return false;
  const parts = value.split("-");
  if (parts.length < 3 || parts.length > 4) return false;
  if (!parts.every((part) => TARGET_SEGMENT.test(part))) return false;
  if (parts.length === 3) return ENVIRONMENTLESS_SYSTEMS.has(parts[2]);
  return SYSTEM_ENVIRONMENTS.get(parts[2])?.has(parts[3]) === true;
}

/**
 * Returns whether a value is a recognized standard Cargo target directory.
 * Cleanup requires this positive architecture-and-vendor check before deleting
 * a cross-target profile. Sidecar producers retain the canonical-shape check at
 * their dynamic host-tuple boundary so a newer rustc host remains usable.
 */
export function isKnownCargoTargetTriple(value) {
  if (!isCanonicalTargetTriple(value)) return false;
  const parts = value.split("-");
  return KNOWN_TARGET_ARCHITECTURES.has(parts[0]) && KNOWN_TARGET_VENDORS.has(parts[1]);
}

/** Validates a host target tuple or throws the producer-facing input error. */
export function validateTargetTriple(value, source = "target triple") {
  if (!isCanonicalTargetTriple(value)) {
    throw new Error(`${source} returned unexpected target triple '${value}'`);
  }
  return value;
}

/**
 * Returns the executable suffix required by a canonical Rust target triple.
 * Windows targets use `.exe`; every other supported target has no suffix.
 */
export function targetExecutableExtension(value) {
  validateTargetTriple(value);
  return value.split("-")[2] === "windows" ? ".exe" : "";
}

/** Returns the target triple encoded by an exact sidecar name, or null. */
function targetTripleFromSidecarName(name, metadata) {
  if (typeof name !== "string") return null;
  const metadataSuffix = ".metadata.json";
  if (metadata && !name.endsWith(metadataSuffix)) return null;
  let candidate = metadata ? name.slice(0, -metadataSuffix.length) : name;
  if (!candidate.startsWith(SIDECAR_PREFIX)) return null;
  candidate = candidate.slice(SIDECAR_PREFIX.length);
  const hasWindowsExtension = candidate.endsWith(".exe");
  const targetTriple = hasWindowsExtension ? candidate.slice(0, -4) : candidate;
  if (!isCanonicalTargetTriple(targetTriple)) return null;
  const expectsWindowsExtension = targetExecutableExtension(targetTriple) === ".exe";
  return hasWindowsExtension === expectsWindowsExtension ? targetTriple : null;
}

/** Returns whether a binary-directory name is an exact sidecar artifact. */
export function isCanonicalSidecarArtifactName(name) {
  return targetTripleFromSidecarName(name, false) !== null;
}

/** Returns whether a binary-directory name is exact sidecar metadata. */
export function isCanonicalSidecarMetadataName(name) {
  return targetTripleFromSidecarName(name, true) !== null;
}
