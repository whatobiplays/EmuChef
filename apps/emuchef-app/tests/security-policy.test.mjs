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

function commandTokens(command) {
  return command.trim().split(/\s+/);
}

function commandFeatures(command) {
  const tokens = commandTokens(command);
  const features = new Set();
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (token.startsWith("--features=")) {
      for (const feature of token.slice("--features=".length).split(",")) {
        if (feature) features.add(feature);
      }
    } else if (token === "--features" || token === "-f") {
      for (const feature of (tokens[index + 1] ?? "").split(",")) {
        if (feature) features.add(feature);
      }
    }
  }
  return features;
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
    .filter((name) => name.endsWith(".rs") && name !== "updates.rs")
    .map((name) => read(`src-tauri/src/${name}`))
    .join("\n");
  assert.equal(/reqwest|hyper::client|ureq|download_to|latest-darwin\.zip/i.test(rustSources), false);
});

test("manual update discovery keeps all navigation and trust authority in Rust", () => {
  const api = read("src/api.ts");
  const types = read("src/types.ts");
  const updates = read("src-tauri/src/updates.rs");
  const app = read("src-tauri/src/lib.rs");
  const capability = JSON.parse(read("src-tauri/capabilities/default.json"));
  const cargo = read("src-tauri/Cargo.toml");
  const frontend = `${read("src/App.tsx")}\n${read("src/UpdatesPanel.tsx")}`;
  const updateApi = sourceSlice(api, "getUpdateStatus:", "\n};");

  assert.doesNotMatch(`${cargo}\n${app}`, /tauri-plugin-updater|tauri_plugin_updater/i);
  assert.deepEqual(capability.permissions, ["core:default"]);
  assert.doesNotMatch(frontend, /@tauri-apps\/plugin-opener|openUrl|openPath|https?:\/\//i);
  assert.doesNotMatch(updateApi, /\b(?:url|uri|path|signature|publicKey|privateKey|endpoint|origin)\b/i);
  assert.doesNotMatch(types, /interface UpdateStatus[\s\S]{0,900}\b(?:url|uri|path|signature|key|endpoint|origin)\??:/i);
  assert.match(updates, /app\.opener\(\)\s*\.open_url\(candidate\.manifest\.dmg_url/s);
  assert.match(updates, /redirect\(reqwest::redirect::Policy::none\(\)\)/);
  assert.match(updates, /\.no_proxy\(\)/);
  assert.match(updates, /\.header\(ACCEPT_ENCODING, "identity"\)/);
});

test("production update trust is fail-closed and fixture authority is separate", () => {
  assert.deepEqual(JSON.parse(read("src-tauri/update-trust.json")), {
    schemaVersion: 1,
    configured: false,
  });
  const fixture = JSON.parse(read("tests/fixtures/update-trust.json"));
  assert.equal(fixture.configured, true);
  assert.match(fixture.metadataKeyId, /^test-/);
  assert.notEqual(read("src-tauri/update-trust.json").includes(fixture.metadataPublicKey), true);
});

test("Updates UI is accessible, manual, and explicit about browser download trust", () => {
  const panel = read("src/UpdatesPanel.tsx");
  assert.match(panel, /<AccessibleDialog/);
  assert.match(panel, /initialFocusRef=\{closeRef\}/);
  assert.match(panel, /dismissible=\{!closeBlocked\}/);
  assert.match(panel, /onDismissBlocked=/);
  assert.match(panel, /Install the update manually/);
  assert.match(panel, /EmuChef verifies the release information/);
  assert.match(panel, /Your browser[\s\S]*downloads the installer,[\s\S]*macOS checks the app/);
  assert.doesNotMatch(panel, /status\?\.dmgSha256|Release SHA-256|Developer ID|notarization|stapling|Gatekeeper/);
  assert.match(panel, /role=\{status\.state === "failed" \? "alert" : "status"\}/);
});

test("update manifest release preparation reuses Phase 3E credentialed verification", () => {
  const packaging = read("scripts/macos-packaging.mjs");
  const release = read("scripts/macos-update-manifest.mjs");
  const packageJson = JSON.parse(read("package.json"));
  assert.match(packaging, /verifyCredentialedRelease\(app\.appPath, dmgPath/);
  assert.match(packaging, /hdiutil", \["attach", "-readonly", "-nobrowse"/);
  assert.match(packaging, /appTreeDigest\(contained\.appPath[\s\S]*appTreeDigest\(app\.appPath/);
  assert.match(release, /verifyCredentialedUpdateArtifacts\(appPath, dmgPath\)/);
  assert.doesNotMatch(release, /private[-_ ]?key|sign\(/i);
  assert.equal(packageJson.scripts["release:macos:update-manifest"], "node scripts/macos-update-manifest.mjs");
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
    /pub async fn pick_platform_tools_zip\(app: AppHandle\)/,
  );
  assert.match(
    commands,
    /pub async fn install_platform_tools_selection\(\s*selection_handle: String,\s*expected_revision: Option<u64>,\s*app: AppHandle/s,
  );
  assert.doesNotMatch(api, /pickPlatformToolsZip:\s*\([^)]*(path|file)/i);
  assert.doesNotMatch(api, /installPlatformToolsSelection:\s*\([^)]*(path|file)/i);
  assert.doesNotMatch(commands, /open_platform_tools_download_page\([^)]*(url|uri):/i);
});

test("Platform-Tools picker and installation split filesystem authority across an opaque handle", () => {
  const commands = read("src-tauri/src/commands.rs");
  const pickerStart = commands.indexOf("pub async fn pick_platform_tools_zip");
  const installStart = commands.indexOf("pub async fn install_platform_tools_selection");
  const installEnd = commands.indexOf("pub(crate) type PickerCompletion", installStart);
  const pickerCommand = commands.slice(pickerStart, installStart);
  const installCommand = commands.slice(installStart, installEnd);
  assert.notEqual(pickerStart, -1);
  assert.notEqual(installStart, -1);
  assert.match(pickerCommand, /picker\.pick_file\(/);
  assert.doesNotMatch(pickerCommand, /blocking_pick_file|run_import_task/);
  assert.match(pickerCommand, /\.replace\(path\)/);
  assert.match(installCommand, /\.take\(&selection_handle\)/);
  assert.match(installCommand, /run_import_task\(move \|\|/);
  assert.ok(installCommand.indexOf("run_import_task") < installCommand.indexOf(".await?"));
  assert.ok(installCommand.indexOf("drop(adb)") < installCommand.indexOf(".handles"));
  assert.doesNotMatch(installCommand, /picker\.pick_file|blocking_pick_file/);
  assert.match(commands, /tauri::async_runtime::spawn_blocking\(task\)/);
});

test("recipe input path dialogs keep picking and immediate validation in Tauri", () => {
  const commands = read("src-tauri/src/commands.rs");
  const api = read("src/api.ts");
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
  assert.match(pickerCommand, /input_contracts[\s\S]*\.require\(request_generation, &input_key\)/);
  assert.match(pickerCommand, /std::fs::File::open|std::fs::read_dir/);
  const frontendPicker = sourceSlice(api, "pickInputPath:", "\n  listRecentConfigurations:");
  assert.doesNotMatch(frontendPicker, /pathKind|allowedExtensions|multiple/);
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
  assert.match(adb, /const PROCESS_TIMEOUT: Duration\s*=\s*Duration::from_secs\(\d+\);/);
  assert.match(adb, /const PROCESS_CLEANUP_TIMEOUT: Duration\s*=\s*Duration::from_secs\(\d+\);/);
  assert.match(adb, /Timer::after\(timeout\)/);
  assert.match(adb, /Timer::after\(PROCESS_CLEANUP_TIMEOUT\)/);
  assert.match(adb, /future::race\(/);
  assert.match(adb, /Command::new\(program\)/);
  assert.match(adb, /cleanup_process\(&mut child\)/);
  assert.match(adb, /child\.kill\(\)/);
  assert.doesNotMatch(adb, /Command::new\(\s*"adb"\s*\)/);
  assert.doesNotMatch(adb, /\b(?:which|where)\b/);
  assert.doesNotMatch(
    adb,
    /Command::new\(\s*"(?:sh|bash|zsh)"\s*\)|\b(?:sh|bash|zsh)\b\s+-c/,
  );
  assert.match(adb, /#\[cfg\(debug_assertions\)\]\s*fn find_system_adb\(\)/s);
  const debugPathLookup = sourceSlice(adb, "fn find_system_adb()", "\nfn setup_error");
  assert.match(debugPathLookup, /std::env::var_os\("PATH"\)/);
  const releaseSource = adb.replace(debugPathLookup, "");
  assert.doesNotMatch(
    releaseSource,
    /std::env::(?:var_os|var)\("PATH"\)|std::env::split_paths/,
  );
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

test("support and cache IPC expose opaque logical entries without filesystem authority", () => {
  const api = read("src/api.ts");
  const types = read("src/types.ts");
  const support = read("src-tauri/src/support.rs");
  const frontendApi = sourceSlice(api, "cacheInventory:", "\n};");
  assert.match(types, /cacheEntryHandle: string/);
  assert.doesNotMatch(frontendApi, /\b(?:cacheRoot|path|fileName|metadataPath|destination|rawBundle|url)\b/i);
  assert.match(support, /pub struct SupportStore/);
  assert.match(support, /fn canonical_cache_root/);
  assert.match(support, /fn delete_logical_entry/);
  assert.doesNotMatch(types, /(?:cache|diagnostics)[\s\S]{0,800}\b(?:path|fileName|metadataPath|destination|rawBundle|url)\??:/i);
});

test("end-user cache root is injected only from trusted app-data startup", () => {
  const app = read("src-tauri/src/lib.rs");
  const sidecar = read("src-tauri/src/sidecar.rs");
  const backend = read("../../crates/emuchef-rust-backend/src/execution_session.rs");
  assert.match(app, /let cache_root = app_data\.join\("artifact-cache"\)/);
  assert.match(app, /SidecarState::new\(cache_root\.clone\(\)\)/);
  assert.match(sidecar, /\.arg\("--cache-root"\)\s*\.arg\(cache_root\)/s);
  assert.match(backend, /cache_root: root\.join\("\.emuchef_cache"\)\.join\("artifacts"\)/);
  assert.doesNotMatch(`${app}\n${sidecar}`, /EMUCHEF_(?:CACHE|ARTIFACT)|std::env::var[^\n]*CACHE/i);
});

test("saved configuration file authority remains behind opaque Tauri handles", () => {
  const api = read("src/api.ts");
  const types = read("src/types.ts");
  const saved = read("src-tauri/src/saved_configurations.rs");
  const frontendSavedApi = sourceSlice(
    api,
    "listRecentConfigurations:",
    "\n};",
  );
  assert.doesNotMatch(frontendSavedApi, /\b(?:path|documentId|configurationId|yaml|planDigest|reviewHandle|executionHandle|serial)\b/);
  assert.match(types, /configurationHandle: string/);
  assert.match(types, /devicePlan: string/);
  assert.doesNotMatch(types, /interface SavedConfigurationDocument[\s\S]{0,700}\b(?:path|documentId|configurationId|yaml|planDigest|reviewHandle|executionHandle|serial)\b/);
  assert.match(saved, /Configuration file[\s\S]*paths[\s\S]*remain in this module/);
  assert.match(saved, /picker\.save_file\(/);
  assert.match(saved, /picker\.pick_file\(/);
  assert.match(saved, /configurationHandle/);
  const projection = sourceSlice(saved, "fn project_document", "\nfn classify_document_bindings");
  assert.doesNotMatch(projection, /"(?:path|documentId|configurationId|yaml|planDigest|reviewHandle|executionHandle|serial)"/);
  assert.match(saved, /describeConfiguration/);
  assert.match(saved, /Some\(false\)[\s\S]*safe\.insert/);
  assert.match(saved, /None => omitted\.push/);
});

test("portable saved state excludes generated plan and device authority", () => {
  const saved = read("src-tauri/src/saved_configurations.rs");
  const createRequest = sourceSlice(
    saved,
    "pub struct CreateSavedConfigurationRequest",
    "\n\n#[derive(Debug, Deserialize)]",
  );
  assert.match(createRequest, /device_plan: String/);
  assert.match(createRequest, /selected_recipes: Vec<String>/);
  assert.match(createRequest, /bindings: HashMap<String, Value>/);
  assert.doesNotMatch(createRequest, /\b(?:plan|digest|review|execution|confirmation|launch|serial|facts)\b/);
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
  const api = read("src/api.ts");
  const appBackend = read("src-tauri/src/lib.rs");
  const reviewStep = read("src/ReviewStep.tsx");
  const cargo = read("src-tauri/Cargo.toml");
  const execution = read("src-tauri/src/execution.rs");
  const packageJsonSource = read("package.json");
  const scripts = JSON.parse(packageJsonSource).scripts;
  const types = read("src/types.ts");
  const featuresSection = sourceSlice(cargo, "[features]", "\n[build-dependencies]");

  assert.match(featuresSection, /^default\s*=\s*\[\s*\]\s*$/m);
  assert.match(featuresSection, /^real-execution\s*=\s*\[\s*\]\s*$/m);
  assert.match(
    execution,
    /pub struct ExecutionCapabilities \{\s*pub real_execution_compiled: bool,\s*pub platform_tools_status: PlatformToolsStatus,\s*pub executor_readiness: ExecutorReadiness,\s*\}/,
  );
  assert.match(
    execution,
    /pub async fn get_execution_capabilities\(\s*state: State<'_, AppState>,\s*\) -> Result<ExecutionCapabilities, String>/,
  );
  const capabilityCommand = sourceSlice(
    execution,
    "pub async fn get_execution_capabilities",
    "\nfn execution_capabilities_unavailable",
  );
  assert.match(capabilityCommand, /if !cfg!\(feature = "real-execution"\)/);
  assert.match(capabilityCommand, /\.readiness_snapshot\(\)/);
  assert.match(capabilityCommand, /tauri::async_runtime::spawn_blocking/);
  assert.match(capabilityCommand, /generations\.matches\(current_adb_revision, current_runtime_generation\)/);
  assert.doesNotMatch(
    capabilityCommand,
    /listAdbDevices|probeDevice|startExecution|adb\s+devices|start-server|reconnect|std::env/,
  );
  assert.match(
    execution,
    /fn execution_capabilities_unavailable\(\) -> String \{\s*safe_error\(\s*"execution_capabilities_unavailable"/,
  );
  assert.doesNotMatch(
    capabilityCommand,
    /safe_error\(\s*"(?!execution_capabilities_unavailable)/,
  );
  assert.match(appBackend, /execution::get_execution_capabilities,/);
  assert.doesNotMatch(`${execution}\n${appBackend}\n${api}`, /get_real_execution_availability/);
  assert.match(
    api,
    /executionCapabilities: \(\) =>\s*invoke<ExecutionCapabilities>\("get_execution_capabilities"\)/,
  );
  assert.match(
    types,
    /export interface ExecutionCapabilities \{\s*readonly realExecutionCompiled: boolean;\s*readonly platformToolsStatus: PlatformToolsStatus;\s*readonly executorReadiness: ExecutorReadiness;\s*\}/,
  );
  assert.match(types, /export type PlatformToolsStatus =\s*\| "notApplicable"\s*\| "ready"\s*\| "notFound"\s*\| "invalid"\s*\| "checkFailed";/);
  assert.match(types, /export type ExecutorReadiness =\s*\| "notCompiled"\s*\| "ready"\s*\| "blocked"\s*\| "unknown";/);

  const startCommand = sourceSlice(
    execution,
    "pub fn start_real_execution",
    "\nfn parse_real_start_request",
  );
  assert.match(startCommand, /if !cfg!\(feature = "real-execution"\)/);
  assert.ok(startCommand.indexOf("if !cfg!") < startCommand.indexOf("parse_real_start_request"));
  assert.ok(startCommand.indexOf("if !cfg!") < startCommand.indexOf("reserve_start"));

  assert.doesNotMatch(
    `${execution}\n${packageJsonSource}`,
    /EMUCHEF_(?:ENABLE|ALLOW)_REAL_EXECUTION|--(?:enable-)?real-execution|std::env::args(?:_os)?\(/,
  );

  assert.deepEqual(commandTokens(scripts["tauri:dev:real"]).slice(0, 2), ["tauri", "dev"]);
  assert.ok(commandFeatures(scripts["tauri:dev:real"]).has("real-execution"));
  assert.equal(commandFeatures(scripts.tauri).has("real-execution"), false);
  assert.equal(commandFeatures(scripts["tauri:dev"]).has("real-execution"), false);

  const productionScriptNames = Object.keys(scripts).filter((name) =>
    name.split(":").some((segment) => ["build", "bundle", "release"].includes(segment)),
  );
  assert.ok(productionScriptNames.length > 0);
  for (const name of productionScriptNames) {
    assert.equal(commandFeatures(scripts[name]).has("real-execution"), false, `${name} must remain simulation-only`);
  }

  const realExecutionScriptNames = Object.entries(scripts)
    .filter(([, command]) => command.includes("real-execution"))
    .map(([name]) => name);
  assert.deepEqual(new Set(realExecutionScriptNames), new Set(["tauri:dev:real"]));

  assert.match(
    app,
    /\[executionCapabilities, setExecutionCapabilities\] = useState<ExecutionCapabilities \| null>\(null\)/,
  );
  assert.match(app, /api\.executionCapabilities\(\)/);
  assert.doesNotMatch(app, /executionCapabilities\(\)\.catch|realExecutionCompiled:\s*false/);
  assert.doesNotMatch(
    `${app}\n${api}`,
    /localStorage|sessionStorage|URLSearchParams|import\.meta\.env|process\.env/,
  );
  assert.match(reviewStep, /\{realExecutionCompiled && \(\s*<button[\s\S]{0,400}>\s*Apply to Device\s*<\/button>/);
  assert.match(app, /activeDialog\?\.payload\.kind === "real-execution" && workflow\.review/);

  const readinessProbe = sourceSlice(
    read("src-tauri/src/adb.rs"),
    "impl AdbReadinessSnapshot",
    "\nfn secure_open_zip",
  );
  assert.match(readinessProbe, /validate_current_install/);
  assert.doesNotMatch(
    readinessProbe,
    /"devices"|"start-server"|"reconnect"|listAdbDevices|probeDevice|startExecution/,
  );

  const realStart = sourceSlice(
    execution,
    "fn start_real_execution_inner",
    "\nfn request_real_start",
  );
  assert.match(realStart, /\.revalidate_for_execution\(expected_adb\)/);
  assert.ok(realStart.indexOf("revalidate_for_execution") < realStart.indexOf('"listAdbDevices"'));
  assert.doesNotMatch(realStart, /ExecutionCapabilities|platform_tools_status|executor_readiness/);
});

test("the Phase 6C qualification catalog is fixed, debug-only, and fully gated", () => {
  const catalog = read("src-tauri/src/catalog.rs");
  const packageJson = read("package.json");
  const gatedResolution = sourceSlice(
    catalog,
    '#[cfg(all(debug_assertions, feature = "real-execution"))]',
    "\n        let (root, source_id, version) = resolved;",
  );

  assert.match(gatedResolution, /EMUCHEF_RUN_REAL_ADB_TESTS/);
  assert.match(gatedResolution, /EMUCHEF_PHASE_6C_QUALIFICATION_CATALOG/);
  assert.match(gatedResolution, /tests\/fixtures\/phase-6c\/non-root\/recipe/);
  assert.match(gatedResolution, /\.app_cache_dir\(\)/);
  assert.match(gatedResolution, /"emuchef\.phase6c\.qualification"/);
  assert.match(gatedResolution, /"phase6c-qualification-1"/);
  assert.doesNotMatch(gatedResolution, /std::env::args|URLSearchParams|localStorage|sessionStorage/);
  assert.doesNotMatch(packageJson, /EMUCHEF_PHASE_6C_QUALIFICATION_CATALOG/);
});

test("accessible fallbacks and technical details cannot expose protected data", () => {
  const fallback = read("src/ErrorBoundary.tsx");
  const app = read("src/App.tsx");
  const supportPanel = read("src/SupportPanel.tsx");
  const fallbackView = sourceSlice(fallback, "export function FrontendErrorFallback", "\n\n/** Top-level boundary");
  assert.doesNotMatch(fallbackView, /error\.(?:message|stack)|String\(error\)|console\.|serial|handle|path|raw/i);
  assert.match(fallbackView, /Reload EmuChef safely/);
  assert.match(app, /<summary>Technical details<\/summary>[\s\S]{0,200}<code>\{diagnostic\.code\}/);
  assert.match(supportPanel, /\{outcome\.supportCode && \([\s\S]{0,300}<code>\{outcome\.supportCode\}<\/code>/);
  assert.doesNotMatch(supportPanel, /outcome\.code|Technical details/);
  assert.doesNotMatch(supportPanel, /<code>\{(?:entry\.cacheEntryHandle|outcome\.entryHandle)\}<\/code>|>\{(?:entry\.cacheEntryHandle|outcome\.entryHandle)\}</);
});

test("CI continuously verifies both execution feature configurations", () => {
  const workflow = read("../../.github/workflows/emuchef-execution-feature-matrix.yml");

  assert.match(workflow, /^name: EmuChef execution feature matrix$/m);
  assert.match(workflow, /^permissions:\s*\n\s*contents: read$/m);
  assert.match(workflow, /^\s*security-policy:\s*$/m);
  assert.match(workflow, /npm --prefix apps\/emuchef-app run test:security/);
  assert.match(workflow, /^\s*rust-feature-matrix:\s*$/m);
  assert.match(workflow, /^\s*fail-fast: false$/m);
  assert.match(workflow, /configuration: simulation-only\s*\n\s*cargo_arguments: --no-default-features/);
  assert.match(
    workflow,
    /configuration: real-execution\s*\n\s*cargo_arguments: --no-default-features --features real-execution/,
  );
  assert.match(
    workflow,
    /cargo check --locked --manifest-path apps\/emuchef-app\/src-tauri\/Cargo\.toml \$\{\{ matrix\.cargo_arguments \}\}/,
  );
  assert.match(
    workflow,
    /cargo test --locked --manifest-path apps\/emuchef-app\/src-tauri\/Cargo\.toml \$\{\{ matrix\.cargo_arguments \}\}/,
  );
  assert.match(
    workflow,
    /cargo fmt --manifest-path apps\/emuchef-app\/src-tauri\/Cargo\.toml -- --check/,
  );
  assert.doesNotMatch(
    workflow,
    /tauri build[^\n]*real-execution|package:macos[^\n]*real-execution|release[^\n]*--features real-execution/,
  );
  for (const path of [
    "crates/emuchef-rust-backend/**",
    "apps/emuchef-app/**",
    "docs/manual/phase-6c-1-non-root-executor-qualification.md",
    "scripts/build-phase-6c-android-fixture.sh",
    "tools/phase-6c-fixture.mjs",
    "tools/phase-6c-fixture.test.mjs",
    "tests/fixtures/phase-6c/non-root/**",
    ".github/workflows/emuchef-execution-feature-matrix.yml",
  ]) {
    assert.match(workflow, new RegExp(path.replaceAll(".", "\\.").replaceAll("**", "\\*\\*")));
  }
});

test("Phase 6D.6 storage recovery is allowlisted and physical evidence is gated", () => {
  const backend = read("../../crates/emuchef-rust-backend/src/execution_session.rs");
  const executor = read("../../crates/emuchef-rust-backend/src/executor.rs");
  const tauri = read("src-tauri/src/execution.rs");
  const harness = read("../../crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs");
  const evidenceValidator = read("../../tools/phase-6d6-evidence.mjs");
  assert.match(backend, /DeviceStorageExhausted/);
  assert.match(backend, /device_storage_exhausted/);
  assert.match(executor, /DeviceStorageExhausted/);
  assert.match(tauri, /The device ran out of storage during execution/);
  assert.match(tauri, /old execution cannot resume/);
  const tauriProduction = tauri.slice(0, tauri.indexOf("#[cfg(test)]"));
  assert.doesNotMatch(tauriProduction, /private ADB output|\/data\/private/);
  assert.match(harness, /#\[ignore =/);
  assert.match(harness, /EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS/);
  assert.match(harness, /SENTINEL_TIMEOUT: Duration = Duration::from_secs\(10 \* 60\)/);
  assert.match(harness, /online != \[invocation\.serial\.clone\(\)\]/);
  assert.match(evidenceValidator, /device_storage_exhausted/);
  assert.match(evidenceValidator, /raw serial or private payload/);
});

test("device qualification is backend-authored, sanitized, and non-authorizing", () => {
  const app = read("src-tauri/src/lib.rs");
  const api = read("src/api.ts");
  const types = read("src/types.ts");
  const qualification = read("src-tauri/src/device_qualification.rs");
  const execution = read("src-tauri/src/execution.rs");
  const qualificationApi = sourceSlice(
    api,
    "deviceQualification:",
    "\n  startRealExecution:",
  );

  assert.match(app, /mod device_qualification;/);
  assert.match(app, /device_qualification::get_device_qualification,/);
  assert.match(app, /device_qualification::check_device_root,/);
  assert.match(
    qualificationApi,
    /invoke<DeviceQualificationSnapshot>\("get_device_qualification", \{ deviceHandle \}\)/,
  );
  assert.doesNotMatch(
    qualificationApi,
    /\b(?:serial|adbPath|command|plan|reviewHandle|executionHandle)\b/,
  );
  assert.match(types, /readonly root: RootQualification \| null;/);
  assert.doesNotMatch(types, /interface DeviceQualificationSnapshot[\s\S]{0,900}\bserial\b/i);
  assert.match(qualification, /RootQualificationState::CheckFailed/);
  assert.match(qualification, /RootQualificationFailureReason::UnexpectedResponse/);
  assert.match(qualification, /"checkRoot"/);
  assert.match(qualification, /devices\.len\(\) != 1/);
  assert.doesNotMatch(qualification, /Command::new|std::process|listAdbDevices|probeDevice|startExecution/);
  assert.doesNotMatch(execution, /DeviceQualificationSnapshotDto|qualification_revision/);
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

test("recovery persistence is fixed-path, strict, and schema-sensitive", () => {
  const frontend = read("src/App.tsx");
  const app = read("src-tauri/src/lib.rs");
  const api = read("src/api.ts");
  const recovery = read("src-tauri/src/recovery.rs");
  const productionRecovery = recovery.slice(0, recovery.indexOf("#[cfg(test)]"));
  const stageApi = sourceSlice(api, "stageRecoveryDraft:", "\n  deferRecoveryDraft:");

  assert.match(app, /app_data\.join\("recovery-draft\.json"\)/);
  assert.match(app, /app_data\.join\("session-active\.marker"\)/);
  assert.doesNotMatch(stageApi, /(?:recovery|draft|marker)(?:File)?Path|executionHandle|reviewHandle|deviceHandle/);
  assert.match(recovery, /serde\(rename_all = "camelCase", deny_unknown_fields\)/);
  assert.match(productionRecovery, /self\.sensitivity\.get\(&key\)\.copied\(\) == Some\(false\)/);
  assert.doesNotMatch(
    productionRecovery,
    /Regex|to_(?:ascii_)?lowercase|contains\("(?:password|token|credential|apiKey)"\)/,
  );
  assert.doesNotMatch(productionRecovery, /secret-value|masked|hash(?:ed)?_value/i);
  assert.match(frontend, /activeDialog\?\.payload\.kind === "recovery"/);
  assert.match(frontend, />Restore<\/button>/);
  assert.match(frontend, />Discard<\/button>/);
  assert.match(frontend, />Not now<\/button>/);
  assert.match(frontend, /onDismiss=\{\(\) => dialogController\.settle\(activeDialog\.id, "not-now"\)\}/);
  assert.ok(frontend.indexOf("await offerRecovery(session.recovery)") < frontend.indexOf("setStartupReady(true)"));
});
