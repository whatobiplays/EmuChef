import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const appDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function read(relativePath: string): string {
  return fs.readFileSync(path.join(appDir, relativePath), "utf8");
}

function pngMetadata(relativePath: string): {
  width: number;
  height: number;
  bitDepth: number;
  colorType: number;
} {
  const bytes = fs.readFileSync(path.join(appDir, relativePath));
  assert.deepEqual([...bytes.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  assert.equal(bytes.subarray(12, 16).toString("ascii"), "IHDR");
  return {
    width: bytes.readUInt32BE(16),
    height: bytes.readUInt32BE(20),
    bitDepth: bytes[24],
    colorType: bytes[25],
  };
}

test("approved icon master and generated Tauri assets have bounded provenance and structure", () => {
  const iconDirectory = path.join(appDir, "src-tauri/icons");
  const master = fs.readFileSync(path.join(iconDirectory, "app-icon.png"));
  assert.equal(
    createHash("sha256").update(master).digest("hex"),
    "c0075df717c1adcc9351d889bbb6412f3855b3d14f9cc12e19291c73f8203587",
  );
  assert.deepEqual(pngMetadata("src-tauri/icons/app-icon.png"), {
    width: 1254,
    height: 1254,
    bitDepth: 8,
    colorType: 2,
  });
  assert.deepEqual(pngMetadata("src-tauri/icons/icon.png"), {
    width: 512,
    height: 512,
    bitDepth: 8,
    colorType: 6,
  });
  assert.equal(fs.existsSync(path.join(iconDirectory, "app-icon.svg")), false);

  const icns = fs.readFileSync(path.join(iconDirectory, "icon.icns"));
  assert.equal(icns.subarray(0, 4).toString("ascii"), "icns");
  assert.equal(icns.readUInt32BE(4), icns.length);

  const decodedDirectory = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-icon-test-"));
  try {
    const iconset = path.join(decodedDirectory, "Decoded.iconset");
    execFileSync("iconutil", ["-c", "iconset", path.join(iconDirectory, "icon.icns"), "-o", iconset]);
    const sizes = fs.readdirSync(iconset)
      .filter((entry) => entry.endsWith(".png"))
      .map((entry) => {
        const bytes = fs.readFileSync(path.join(iconset, entry));
        return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];
      });
    assert.equal(sizes.length, 10);
    assert.ok(sizes.every(([width, height]) => width === height));
    assert.deepEqual([...new Set(sizes.map(([width]) => width))].sort((left, right) => left - right), [16, 32, 64, 128, 256, 512, 1024]);
  } finally {
    fs.rmSync(decodedDirectory, { recursive: true, force: true });
  }

  const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json"));
  assert.deepEqual(tauriConfig.bundle.icon, ["icons/icon.icns", "icons/icon.png"]);
});

test("visual system uses semantic tokens and resilient accessibility policies", () => {
  const styles = read("src/styles.css");
  const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json"));
  assert.equal(tauriConfig.app.windows[0].minWidth, 760);
  assert.match(styles, /--color-canvas:/);
  assert.match(styles, /--color-surface:/);
  assert.match(styles, /--color-text-muted:/);
  assert.match(styles, /--color-focus:/);
  assert.match(styles, /--color-danger-border:/);
  assert.match(styles, /font-family:\s*-apple-system/);
  assert.doesNotMatch(styles, /radial-gradient|linear-gradient|\bInter\b/);
  assert.match(styles, /\.configuration-bar\s*\{[^}]*display:\s*grid/s);
  assert.match(styles, /\.result-card-list\s*\{[^}]*gap:/s);
  assert.match(styles, /\.execution-group\.status-failed\s*\{[^}]*solid/s);
  assert.match(styles, /\.execution-group\.status-blocked\s*\{[^}]*dashed/s);
  assert.match(styles, /@media \(max-width: 760px\)/);
  assert.match(styles, /@media \(max-width: 440px\)/);
  assert.match(styles, /200% zoom/);
  assert.match(styles, /prefers-reduced-motion: reduce/);
  assert.match(styles, /forced-colors: active/);
  assert.match(styles, /:focus-visible/);
  assert.match(styles, /overflow-wrap:\s*anywhere/);
});

test("ordinary UI source omits retired implementation terminology and raw result formatting", () => {
  const source = [
    "src/App.tsx",
    "src/InputsStep.tsx",
    "src/ReviewStep.tsx",
    "src/ExecutionStep.tsx",
    "src/accessibility.ts",
    "src/SavedConfigurationManager.tsx",
    "src/UpdatesPanel.tsx",
    "src/ErrorBoundary.tsx",
  ].map(read).join("\n");
  assert.doesNotMatch(source, /Rust runtime|Runtime ready|backend-approved|Schema \{|Release SHA-256|app bundle or repository|No execution history[^\n]*sidecar/i);
  assert.doesNotMatch(source, /replaceAll\("_"|split\("_"/);
  assert.doesNotMatch(source, />Support &amp; Storage<|>Support & Storage</);
  assert.match(source, />Troubleshooting</);
  assert.match(source, />Setup catalog</);
  assert.match(source, /EXECUTION_STATUS_LABELS/);
  assert.match(source, /RECIPE_STATUS_LABELS/);
  assert.match(source, /STEP_STATUS_LABELS/);
});

test("window title, dark appearance, and native About metadata use authoritative application metadata", () => {
  const index = read("index.html");
  const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json"));
  const menu = read("src-tauri/src/menu.rs");
  assert.equal(tauriConfig.productName, "EmuChef");
  assert.equal(tauriConfig.app.windows[0].title, "EmuChef");
  assert.match(index, /<meta name="color-scheme" content="dark"/);
  assert.match(index, /<title>EmuChef<\/title>/);
  assert.match(menu, /let package = app\.package_info\(\)/);
  assert.match(menu, /name: Some\(package\.name\.clone\(\)\)/);
  assert.match(menu, /version: Some\(package\.version\.to_string\(\)\)/);
  assert.match(menu, /GNU General Public License v3\.0/);
  assert.doesNotMatch(menu, /version: Some\("\d+\.\d+\.\d+"/);
});
