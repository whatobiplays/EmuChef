/**
 * Source-level regression checks for security boundaries that are not fully
 * observable through the application logic tests.
 *
 * These checks deliberately verify ownership and enablement constraints at the
 * IPC boundary. They complement, rather than replace, Rust behavioral tests.
 */
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const appDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const rustSourceDir = path.join(appDir, "src-tauri/src");

function read(relativePath) {
  return fs.readFileSync(path.join(appDir, relativePath), "utf8");
}

function sourceSlice(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start + startMarker.length);
  assert.notEqual(start, -1, `missing source marker: ${startMarker}`);
  assert.notEqual(end, -1, `missing source marker: ${endMarker}`);
  return source.slice(start, end);
}

test("production CSP has no web network origin", () => {
  const config = JSON.parse(fs.readFileSync(path.join(appDir, "src-tauri/tauri.conf.json"), "utf8"));
  const csp = config.app.security.csp;
  assert.equal(csp.includes("https:"), false);
  assert.equal(csp.includes("unsafe-eval"), false);
  assert.deepEqual(config.bundle.externalBin, ["binaries/emuchef"]);
});

test("app does not bundle or fetch Platform-Tools", () => {
  const config = read("src-tauri/tauri.conf.json");
  assert.equal(/platform-tools|latest-darwin\.zip/i.test(config), false);
  const packageJson = read("package.json");
  assert.equal(/axios|node-fetch|updater/i.test(packageJson), false);
  const rustSources = fs
    .readdirSync(rustSourceDir)
    .filter((name) => name.endsWith(".rs"))
    .map((name) => read(`src-tauri/src/${name}`))
    .join("\n");
  assert.equal(/reqwest|hyper::client|ureq|download_to|latest-darwin\.zip/i.test(rustSources), false);
});

test("Platform-Tools setup exposes only the official page and no React path argument", () => {
  const adb = read("src-tauri/src/adb.rs");
  const commands = read("src-tauri/src/commands.rs");
  const api = read("src/api.ts");
  assert.match(
    adb,
    /PLATFORM_TOOLS_URL: &str = "https:\/\/developer\.android\.com\/tools\/releases\/platform-tools"/,
  );
  assert.match(
    commands,
    /pub async fn import_platform_tools_zip\(\s*app: AppHandle\s*\)/s,
  );
  assert.doesNotMatch(api, /importPlatformTools:\s*\([^)]*path/i);
  assert.doesNotMatch(commands, /open_platform_tools_download_page\([^)]*(url|uri):/i);
});

test("Platform-Tools import uses the non-blocking picker before blocking worker work", () => {
  const commands = read("src-tauri/src/commands.rs");
  const start = commands.indexOf("pub async fn import_platform_tools_zip");
  const end = commands.indexOf("#[tauri::command]", start);
  const importCommand = commands.slice(start, end);
  assert.notEqual(start, -1);
  assert.match(importCommand, /picker\.pick_file\(/);
  assert.doesNotMatch(importCommand, /blocking_pick_file/);
  assert.match(importCommand, /run_import_task\(move \|\|/);
  assert.ok(importCommand.indexOf("picker.pick_file") < importCommand.indexOf("run_import_task"));
  assert.ok(importCommand.indexOf("run_import_task") < importCommand.indexOf("state.adb.lock"));
  assert.ok(importCommand.indexOf("drop(adb)") < importCommand.indexOf(".handles"));
  assert.ok(importCommand.indexOf("run_import_task") < importCommand.lastIndexOf(".await?"));
  assert.ok(importCommand.lastIndexOf(".await?") < importCommand.indexOf(".handles"));
  assert.match(
    importCommand,
    /return run_import_task\(move \|\| get_adb_setup_status\(status_app\.state::<AppState>\(\)\)\)\.await;/,
  );
  assert.match(commands, /tauri::async_runtime::spawn_blocking\(task\)/);
});

test("recipe input path dialogs use only non-blocking picker callbacks", () => {
  const commands = read("src-tauri/src/commands.rs");
  const start = commands.indexOf("pub async fn pick_input_path");
  const end = commands.indexOf("\npub(crate) fn catalog", start);
  const pickerCommand = commands.slice(start, end);
  assert.notEqual(start, -1);
  assert.notEqual(end, -1);
  assert.match(pickerCommand, /await_picker_selection\(/);
  assert.match(pickerCommand, /picker\.pick_file\(/);
  assert.match(pickerCommand, /picker\.pick_files\(/);
  assert.match(pickerCommand, /picker\.pick_folder\(/);
  assert.doesNotMatch(pickerCommand, /blocking_pick_/);
  assert.doesNotMatch(pickerCommand, /std::fs|\.metadata\(|\.exists\(/);
});

test("release ADB resolution cannot depend on PATH", () => {
  const adb = read("src-tauri/src/adb.rs");
  assert.match(
    adb,
    /#\[cfg\(debug_assertions\)\]\s*if let Some\(path\) = std::env::var_os\("EMUCHEF_ADB_PATH"\)/s,
  );
  assert.match(
    adb,
    /#\[cfg\(debug_assertions\)\]\s*if std::env::var\("EMUCHEF_ALLOW_SYSTEM_ADB"\)/s,
  );
  assert.match(adb, /\.env_clear\(\)/);
  assert.match(adb, /\.stdin\(Stdio::null\(\)\)/);
  assert.match(adb, /wait_timeout\(timeout\)/);
});

test("React DTOs contain opaque handles and no exact serial property", () => {
  const types = read("src/types.ts");
  const api = read("src/api.ts");
  const commands = read("src-tauri/src/commands.rs");
  assert.match(types, /deviceHandle: string/);
  assert.match(types, /reviewHandle: string/);
  assert.doesNotMatch(types, /(?:^|\s)serial\??:\s*string/m);
  assert.doesNotMatch(api, /(?:^|\W)serial(?:\W|$)/);
  assert.match(commands, /Sidecar DTOs are never returned directly/);
});

test("configuration failures are mapped and raw sidecar diagnostics are debug-only", () => {
  const commands = read("src-tauri/src/commands.rs");
  assert.match(
    commands,
    /\.map_err\(\|error\| configuration_sidecar_error\(&error, &exact_serial\)\)/,
  );
  assert.match(commands, /"configuration_request_invalid"/);
  assert.match(commands, /"configuration_catalog_invalid"/);
  assert.match(commands, /"configuration_validation_failed"/);
  assert.match(
    commands,
    /#\[cfg\(debug_assertions\)\]\s*eprintln!\([\s\S]*redact_internal_sidecar_error/,
  );
  assert.match(
    commands,
    /#\[cfg\(debug_assertions\)\]\s*fn redact_internal_sidecar_error/,
  );
  assert.doesNotMatch(
    commands,
    /safe_error\([^)]*(?:internal_error|exact_serial)/s,
  );
});

test("runtime startup is independent from ADB and simulation remains review-handle-only", () => {
  const app = read("src-tauri/src/lib.rs");
  const api = read("src/api.ts");
  const execution = read("src-tauri/src/execution.rs");
  assert.ok(app.indexOf("sidecar.initialize()") < app.indexOf("AdbManager::new"));
  assert.match(api, /startSimulatedExecution: \(reviewHandle: string\)/);
  assert.doesNotMatch(api, /startSimulatedExecution:[\s\S]{0,180}\b(?:plan|planDigest|mode|serial|catalog)\b/);
  assert.match(execution, /"startExecution"[\s\S]*"mode": "dry_run"/);
  assert.match(execution, /pub fn start_simulated_execution\(\s*review_handle: String,/);
  assert.doesNotMatch(execution, /apply_device/);
});

test("real execution is default-disabled with compile-time-only enablement", () => {
  const app = read("src/App.tsx");
  const cargo = read("src-tauri/Cargo.toml");
  const execution = read("src-tauri/src/execution.rs");
  const packageJson = read("package.json");

  assert.match(cargo, /\[features\]\s+default = \[\]\s+real-execution = \[\]/);
  assert.match(
    execution,
    /pub fn get_real_execution_availability\(\) -> Value \{\s*json!\(\{ "enabled": cfg!\(feature = "real-execution"\) \}\)\s*\}/,
  );

  const startCommand = sourceSlice(
    execution,
    "pub fn start_real_execution",
    "\nfn parse_real_start_request",
  );
  assert.match(startCommand, /if !cfg!\(feature = "real-execution"\)/);
  assert.ok(startCommand.indexOf("if !cfg!") < startCommand.indexOf("parse_real_start_request"));
  assert.ok(startCommand.indexOf("if !cfg!") < startCommand.indexOf("reserve_start"));

  assert.doesNotMatch(
    `${execution}\n${packageJson}`,
    /EMUCHEF_(?:ENABLE|ALLOW)_REAL_EXECUTION|--(?:enable-)?real-execution|std::env::args(?:_os)?\(/,
  );
  assert.match(app, /useState\(false\)[\s\S]{0,1800}realExecutionAvailability\(\)\.catch\(\(\) => \(\{ enabled: false \}\)\)/);
  assert.match(app, /\{realExecutionEnabled && \(\s*<button[\s\S]{0,400}>\s*Apply to Device\s*<\/button>/);
  assert.match(app, /\{realExecutionEnabled && realConfirmationOpen && \(/);
});

test("React cannot supply trusted real-execution or launch data", () => {
  const api = read("src/api.ts");
  const execution = read("src-tauri/src/execution.rs");
  const startApi = sourceSlice(api, "startRealExecution:", "\n  getRealExecution:");
  const launchApi = sourceSlice(api, "launchConfiguredApp:", "\n  pickInputPath:");
  const forbiddenFrontendFields =
    /\b(?:mode|serial|plan|planDigest|adbPath|sidecarId|executionId|package|activity|command|requestType)\b/;

  assert.match(
    startApi,
    /startRealExecution: \(reviewHandle: string, confirmation: RealExecutionConfirmation\) =>\s*invoke<RealExecutionSnapshot>\("start_real_execution", \{\s*request: \{ reviewHandle, confirmation \},\s*\}\)/,
  );
  assert.doesNotMatch(startApi, forbiddenFrontendFields);
  assert.match(
    launchApi,
    /launchConfiguredApp: \(launchActionHandle: string\) =>\s*invoke<LaunchResult>\("launch_configured_app", \{ launchActionHandle \}\)/,
  );
  assert.doesNotMatch(launchApi, forbiddenFrontendFields);

  const realStartDto = sourceSlice(
    execution,
    "#[serde(rename_all = \"camelCase\", deny_unknown_fields)]\nstruct RealExecutionStartRequest",
    "\n\n#[derive(Debug, Deserialize)]",
  );
  assert.match(
    realStartDto,
    /struct RealExecutionStartRequest \{\s*review_handle: String,\s*confirmation: RealExecutionConfirmation,\s*\}/,
  );
});

test("Tauri alone constructs the fixed real-mode sidecar request", () => {
  const execution = read("src-tauri/src/execution.rs");
  const trustedRequest = sourceSlice(execution, "fn request_real_start", "\nfn bind_real_start_result");
  const realModeOccurrences = execution.match(/"mode": "real"/g) ?? [];

  assert.equal(realModeOccurrences.length, 1);
  assert.match(trustedRequest, /runtime_request\(\s*runtime,\s*"startExecution",/s);
  assert.match(trustedRequest, /"plan": review\.response\.get\("plan"\)/);
  assert.match(trustedRequest, /"planDigest": review\.plan_digest/);
  assert.match(trustedRequest, /"mode": "real"/);
  assert.match(trustedRequest, /"targetDevice": review\.target/);
  assert.doesNotMatch(trustedRequest, /std::env|http|remote|device.*enabled|enabled.*device/i);
});
