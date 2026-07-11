# Real-Device RetroArch Validation

This runbook records manual evidence for the Rust planner and executor against a
safe Android test device. It is not part of automated verification and must not
be run against a device containing irreplaceable data.

The sanitized record for the completed 2026-07-11 run is
[`real-device-retroarch-2026-07-11.md`](../release/evidence/real-device-retroarch-2026-07-11.md).

## A. Preconditions

1. Use a macOS or Linux host with Rust, Cargo, Android platform tools, and enough
   free space for the RetroArch APK, assets, cores, extracted staging files, and
   logs.
2. Use a dedicated test device with USB debugging enabled. The selected device
   profile must match the device, and the device must provide the declared
   `root_shell` and `app_data_write` capabilities.
3. For the local-artifact baseline, obtain all 24 artifacts listed in
   `authored/recipes/app.retroarch.provision.yaml` from trusted sources. Record
   their origin and SHA-256 digests before use.
4. Create a temporary authored tree for the local-artifact baseline. Replace every `url:` in its
   RetroArch recipe with an absolute `file://` URL for the corresponding local
   artifact. Do not edit the checked-in authored tree.
5. Choose a device plan appropriate for the test device from
   `authored/device_plans/`. The examples below use
   `ayaneo.pocket_s_mini.base`; change it when testing another profile.
6. Treat the commands in sections C and E as destructive. Confirm the serial
   before every command that uninstalls an app or removes device files.
7. Start from a clean tracked worktree whose committed `HEAD` contains every
   product and runbook correction required for the run. If execution discovers
   a defect, stop, fix and commit it, rerun the affected verification from that
   new commit, and record the new SHA. Do not attach evidence to an older SHA.

Set explicit paths from the repository root:

```bash
export REPO_ROOT="$PWD"
export EMUCHEF="$REPO_ROOT/crates/emuchef-rust-backend/target/debug/emuchef"
export ADB="$(command -v adb)"
export SERIAL="REPLACE_WITH_TEST_DEVICE_SERIAL"
export DEVICE_PLAN="ayaneo.pocket_s_mini.base"
export RUN_ROOT="$(mktemp -d /tmp/emuchef-retroarch.XXXXXX)"
export TESTED_COMMIT="$(git rev-parse HEAD)"
test -z "$(git status --short)"
cp -R "$REPO_ROOT/authored" "$RUN_ROOT/authored-local"
```

Edit
`$RUN_ROOT/authored-local/recipes/app.retroarch.provision.yaml`, replace every
network URL with its absolute local `file://` URL, then save a digest inventory:

```bash
find /absolute/path/to/local/retroarch-artifacts -type f \
  -exec shasum -a 256 {} \; \
  | sort \
  > "$RUN_ROOT/artifact-sha256.txt"
```

## B. Build and Static Verification

Run the automated verification matrix before touching the device:

```bash
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all -- --check
cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets -- -D warnings

cd apps/config-editor
npm ci
npm run check:rust-runtime
npm run typecheck
npm run test:logic
npm run build

# This maintained command builds and copies the ignored host-target debug
# externalBin input. Do not assume src-tauri/binaries already contains it.
npm run sidecar:dev

cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

cd "$REPO_ROOT"
```

Record the commit and tool versions:

```bash
git rev-parse HEAD
git status --short
rustc --version
cargo --version
node --version
npm --version
adb version
```

## C. Clean Device Preparation

Confirm that exactly one intended device is selected:

```bash
adb devices -l
adb -s "$SERIAL" get-state
adb -s "$SERIAL" shell getprop ro.product.manufacturer
adb -s "$SERIAL" shell getprop ro.product.model
adb -s "$SERIAL" shell getprop ro.build.version.release
adb -s "$SERIAL" shell getprop ro.build.version.sdk
adb -s "$SERIAL" shell su -c id
```

After confirming the serial and accepting deletion of existing RetroArch data,
remove the test installation and its provisioned paths:

```bash
adb -s "$SERIAL" uninstall com.retroarch.aarch64 || true
adb -s "$SERIAL" shell su -c \
  'rm -rf /data/user/0/com.retroarch.aarch64 /sdcard/Android/data/com.retroarch.aarch64 /sdcard/RetroArch'
adb -s "$SERIAL" shell pm list packages com.retroarch.aarch64
```

The final command must return no package. If cleanup is denied or files remain,
stop and record the capability failure instead of weakening the test.

## D. Local-Artifact Baseline

Validate and plan from the temporary authored tree. Planning may use live device
facts, but it must not use explicit manufacturer/model overrides for the main
baseline.

```bash
"$EMUCHEF" validate \
  "$RUN_ROOT/authored-local/recipes/app.retroarch.provision.yaml" \
  --authored-root "$RUN_ROOT/authored-local"
"$EMUCHEF" plan \
  --adb "$ADB" \
  --serial "$SERIAL" \
  --authored-root "$RUN_ROOT/authored-local" \
  --device-plan "$DEVICE_PLAN" \
  --output "$RUN_ROOT/retroarch-plan.yaml" \
  > "$RUN_ROOT/plan.stdout.log" \
  2> "$RUN_ROOT/plan.stderr.log"
"$EMUCHEF" apply \
  --adb "$ADB" \
  --serial "$SERIAL" \
  --plan-file "$RUN_ROOT/retroarch-plan.yaml" \
  --dry-run \
  > "$RUN_ROOT/dry-run.stdout.log" \
  2> "$RUN_ROOT/dry-run.stderr.log"
```

Inspect the plan and dry-run output. Confirm the plan has 24 artifacts, every
artifact URL is `file://`, the optional config step is absent when unbound, and
no path escapes the chosen local sources or EmuChef work roots.

Run the mutating apply from `RUN_ROOT` so its cache and runtime directories are
isolated and retained as evidence:

```bash
cd "$RUN_ROOT"
"$EMUCHEF" apply \
  --adb "$ADB" \
  --serial "$SERIAL" \
  --plan-file "$RUN_ROOT/retroarch-plan.yaml" \
  > "$RUN_ROOT/apply.stdout.log" \
  2> "$RUN_ROOT/apply.stderr.log"
cd "$REPO_ROOT"
```

Do not report a pass from the process exit code alone. Complete section E.

## E. Required Runtime Checks

Record each check as pass, fail, or blocked, with a short evidence reference.

1. The validation, planning, dry-run, and mutating apply commands exit zero.
2. The emitted plan id is `plan.<device-plan>.001` and the selected device
   profile matches the observed manufacturer, model, and Android version.
3. All 24 artifacts resolve from the declared local files; none attempts an
   HTTP(S) request.
4. The apply log reports no failed or dependency-blocked step.
5. `adb -s "$SERIAL" shell pm path com.retroarch.aarch64` returns an APK path.
6. RetroArch launches successfully and remains the foreground application long
   enough to show its UI.
7. `/data/user/0/com.retroarch.aarch64/assets` exists and contains extracted
   frontend assets.
8. `/data/user/0/com.retroarch.aarch64/autoconfig` exists and is non-empty.
9. `/storage/emulated/0/RetroArch/cheats` exists and is non-empty.
10. `/data/user/0/com.retroarch.aarch64/database/rdb` exists and contains RDB
    files.
11. `/data/user/0/com.retroarch.aarch64/info` exists and contains core info.
12. `/data/user/0/com.retroarch.aarch64/overlays` and
    `/data/user/0/com.retroarch.aarch64/shaders/shaders_glsl` exist and are
    non-empty.
13. `/data/user/0/com.retroarch.aarch64/cores` contains the selected core
    libraries.
14. `/storage/emulated/0/RetroArch/system/dolphin-emu`, `fbneo`, and `PPSSPP`
    exist after system-file extraction and copy.
15. The cache contains complete files, the runtime staging tree contains no
    leaked partial artifact, and the recorded host/device logs contain no
    secrets or unrelated device data.
16. ZIP artifacts that use Deflate extraction complete successfully. Deflate is
    supported by the Rust archive reader; a Deflate failure is not an expected
    limitation.

Useful inspection commands:

```bash
adb -s "$SERIAL" shell pm path com.retroarch.aarch64
adb -s "$SERIAL" shell dumpsys window | grep -i retroarch
adb -s "$SERIAL" shell su -c \
  "'for p in assets autoconfig database/rdb info overlays shaders/shaders_glsl cores; do echo \"\$p\"; find \"/data/user/0/com.retroarch.aarch64/\$p\" -type f | wc -l; done'"
adb -s "$SERIAL" shell \
  'for p in cheats system/dolphin-emu system/fbneo system/PPSSPP; do echo "$p"; find "/storage/emulated/0/RetroArch/$p" -type f | wc -l; done'
find "$RUN_ROOT/.emuchef_cache" -type f -print
find "$RUN_ROOT/.emuchef_runtime" -type f -print
find "$RUN_ROOT" -type f -name '*.partial' -print
```

Do not use an unqualified `grep -i 'failed\|blocked'` as the success check. It
also matches zero-valued summary fields such as `failed: 0`. Check the process
exit status and use a positive-count expression when reviewing summary logs:

```bash
grep -E '(^|[[:space:]])(failed|blocked):[[:space:]]*[1-9][0-9]*([[:space:]]|$)' \
  "$RUN_ROOT/apply.stdout.log" "$RUN_ROOT/apply.stderr.log" || true
```

No matching line is only one inspection result; the required device and file
checks still determine the manual disposition.

## F. Optional-Input Matrix

Use a known-safe RetroArch configuration file and a fresh plan. The binding name
is the fully qualified recipe input.

```bash
"$EMUCHEF" plan \
  --adb "$ADB" \
  --serial "$SERIAL" \
  --authored-root "$RUN_ROOT/authored-local" \
  --device-plan "$DEVICE_PLAN" \
  --bind "app.retroarch.provision/retroarch_cfg=/absolute/path/to/retroarch.cfg" \
  --output "$RUN_ROOT/retroarch-config-plan.yaml"
```

| Case | Expected result |
| --- | --- |
| Input omitted | `seed_retroarch_cfg` is pruned from the plan. |
| Existing `.cfg` file | Step is present; apply replaces the destination and verification passes. |
| Missing path | Planning rejects the binding. |
| Wrong extension | Planning rejects the binding. |
| Directory instead of file | Planning rejects the binding. |

For the successful case, confirm
`/sdcard/Android/data/com.retroarch.aarch64/files/retroarch.cfg` matches the
source digest or byte content. Do not overwrite a user-owned configuration.

## G. Failure Matrix

Run destructive cases only on disposable state. A failure passes this matrix
only when it is typed or clearly explained, returns nonzero, and leaves no
corrupt cache publication or unintended device mutation.

| Case | Expected boundary |
| --- | --- |
| Network URL with an empty cache | Downloads successfully or fails with a typed, redacted network error; no partial file remains. |
| Missing local artifact | Artifact resolution fails before dependent device steps. |
| Corrupt ZIP | Extraction fails; dependent copy is blocked. |
| Unauthorized or offline serial | ADB setup fails without targeting another device. |
| Profile mismatch | Planning rejects the real detected facts. |
| Root shell unavailable | App-data operations fail clearly; no false success. |
| App-data destination not writable | Copy or verification fails and dependents are blocked. |
| APK install rejected | Install fails and launch/bootstrap dependents do not run. |
| Device disconnect during copy | Apply fails and does not report the interrupted step as complete. |
| Invalid execution plan | Load or validation fails before any device mutation. |

## H. Rerun and Idempotency

Without cleaning the device or host cache after the successful baseline, rerun
the same plan and verify:

1. The same plan remains valid and no generated identifier changes.
2. Artifact resolution uses complete cached files.
3. The installed-package skip condition prevents unnecessary APK replacement.
4. Merge copies preserve expected files and do not create duplicated nested
   directories.
5. Replace copies, when the optional config is bound, preserve exact content.
6. Every verification condition passes again.
7. The second run exits zero with no failed or blocked step and no partial-file
   residue.

Record both run durations and explain any step-result differences.

## I. HTTP(S) Manual Validation

HTTP(S) downloading is implemented and covered by local automated tests. The
2026-07-11 evidence record validates commit
`5dca50603cf3a4831867c229157a94906151cbb7`; every later release
candidate needs its own evidence against the exact tested commit. Use a fresh
temporary authored tree that retains the checked-in remote URLs; do not edit
checked-in authored YAML.

Create a separate network-validation workspace, produce a plan from the remote
URLs, remove only that workspace's host cache, and record the exact commit:

```bash
export NETWORK_RUN_ROOT="$(mktemp -d /tmp/emuchef-retroarch-network.XXXXXX)"
cp -R "$REPO_ROOT/authored" "$NETWORK_RUN_ROOT/authored"
git rev-parse HEAD > "$NETWORK_RUN_ROOT/tested-commit.txt"
"$EMUCHEF" plan \
  --adb "$ADB" \
  --serial "$SERIAL" \
  --authored-root "$NETWORK_RUN_ROOT/authored" \
  --device-plan "$DEVICE_PLAN" \
  --output "$NETWORK_RUN_ROOT/retroarch-network-plan.yaml"
rm -rf "$NETWORK_RUN_ROOT/.emuchef_cache" "$NETWORK_RUN_ROOT/.emuchef_runtime"
```

After re-confirming the disposable device and serial, run the plan from
`NETWORK_RUN_ROOT` and capture a cache manifest after each run:

```bash
cd "$NETWORK_RUN_ROOT"
"$EMUCHEF" apply \
  --adb "$ADB" \
  --serial "$SERIAL" \
  --plan-file "$NETWORK_RUN_ROOT/retroarch-network-plan.yaml" \
  > "$NETWORK_RUN_ROOT/cold.stdout.log" \
  2> "$NETWORK_RUN_ROOT/cold.stderr.log"
find "$NETWORK_RUN_ROOT/.emuchef_cache" -type f ! -name '*.partial' \
  -exec shasum -a 256 {} \; | sort > "$NETWORK_RUN_ROOT/cache-cold.sha256"
find "$NETWORK_RUN_ROOT" -type f -name '*.partial' -print

"$EMUCHEF" apply \
  --adb "$ADB" \
  --serial "$SERIAL" \
  --plan-file "$NETWORK_RUN_ROOT/retroarch-network-plan.yaml" \
  > "$NETWORK_RUN_ROOT/warm.stdout.log" \
  2> "$NETWORK_RUN_ROOT/warm.stderr.log"
find "$NETWORK_RUN_ROOT/.emuchef_cache" -type f ! -name '*.partial' \
  -exec shasum -a 256 {} \; | sort > "$NETWORK_RUN_ROOT/cache-warm.sha256"
cmp "$NETWORK_RUN_ROOT/cache-cold.sha256" "$NETWORK_RUN_ROOT/cache-warm.sha256"
find "$NETWORK_RUN_ROOT" -type f -name '*.partial' -print
cd "$REPO_ROOT"
```

For a macOS Wi-Fi host with USB ADB, identify the Wi-Fi hardware device rather
than assuming `en0`. Reconfirm the USB device before disconnecting the network.
The restoration trap prevents an interrupted shell from intentionally leaving
Wi-Fi disabled:

```bash
export WIFI_DEVICE="$(networksetup -listallhardwareports | awk '/Hardware Port: Wi-Fi/{getline; sub(/^Device: /, ""); print; exit}')"
test -n "$WIFI_DEVICE"
adb -s "$SERIAL" get-state
restore_wifi() { networksetup -setairportpower "$WIFI_DEVICE" on; }
trap restore_wifi EXIT INT TERM
networksetup -setairportpower "$WIFI_DEVICE" off

cd "$NETWORK_RUN_ROOT"
"$EMUCHEF" apply \
  --adb "$ADB" \
  --serial "$SERIAL" \
  --plan-file "$NETWORK_RUN_ROOT/retroarch-network-plan.yaml" \
  > "$NETWORK_RUN_ROOT/offline-warm.stdout.log" \
  2> "$NETWORK_RUN_ROOT/offline-warm.stderr.log"
find "$NETWORK_RUN_ROOT/.emuchef_cache" -type f ! -name '*.partial' \
  -exec shasum -a 256 {} \; | sort > "$NETWORK_RUN_ROOT/cache-offline.sha256"
cmp "$NETWORK_RUN_ROOT/cache-cold.sha256" "$NETWORK_RUN_ROOT/cache-offline.sha256"
find "$NETWORK_RUN_ROOT" -type f -name '*.partial' -print
cd "$REPO_ROOT"

restore_wifi
trap - EXIT INT TERM
```

Complete every runtime check in section E after each device run. Record the
network-disconnection method. When an instrumented local origin is used, also
record its unchanged request count; for the checked-in remote origins, disabled
host networking plus successful USB ADB, identical manifests, and successful
execution provide the offline boundary. Do not mark the case passed if Wi-Fi
was not observed disabled or any partial file remains.

1. Strict TLS certificate and hostname verification.
2. HTTPS succeeds against a trusted local test certificate setup.
3. Invalid, expired, and wrong-host certificates fail closed.
4. Plain HTTP behavior is an explicit policy decision and is tested.
5. Redirect following is bounded.
6. HTTPS-to-HTTP downgrade redirects are rejected.
7. The 15-second connect timeout and five-minute total deadline are bounded and
   typed.
8. Response status failures are typed and include useful artifact context.
9. Deterministic cache keys prevent collisions between distinct artifacts.
10. Downloads write to temporary files inside the cache sandbox.
11. Cache publication uses same-directory no-clobber persistence, with the
    platform-specific guarantee recorded accurately.
12. Interrupted, timed-out, and failed downloads remove temporary files.
13. Existing complete cache entries are not replaced by partial content.
14. Concurrent resolution cannot expose incomplete content.
15. Response-size overflow and storage-failure behavior are typed; no arbitrary
    product size cap is imposed.
16. Local HTTP-server tests cover success, redirects, timeout, truncation, TLS,
    and the no-retry policy without a public-network dependency.
17. A clean-cache RetroArch run resolves all checked-in artifact URLs and
    completes the required runtime checks.
18. A warm-cache rerun succeeds with the server unavailable and retains
    digest-identical cache content.

## J. Evidence Record

| Field | Value |
| --- | --- |
| Date and time | |
| Operator | |
| Commit SHA | |
| Host OS and architecture | |
| Rust/Cargo versions | |
| ADB version | |
| Device manufacturer/model | |
| Android release/API | |
| Device serial, redacted | |
| Device-plan id | |
| Root/app-data capability result | |
| Artifact inventory path | |
| Artifact digest inventory path | |
| Temporary authored root | |
| Execution-plan path and digest | |
| Plan result | |
| Dry-run result | |
| First apply result and duration | |
| Rerun result and duration | |
| Optional-input cases | |
| Failure-matrix cases | |
| Runtime-check results | |
| Cache/runtime inspection result | |
| Logs/evidence location | |
| Deviations | |
| Blockers | |
| Final disposition | |

Keep raw logs outside the repository. Redact serials, usernames, home paths,
tokens, and unrelated device data before sharing evidence.

## K. Pass Criteria

A run is a pass only when all six conditions hold:

1. The exact tested commit and tool/device context are recorded.
2. The automated matrix in section B passes on that commit.
3. The local-artifact baseline completes with every required runtime check in
   section E passing.
4. The uncleaned rerun satisfies every idempotency check in section H.
5. Every attempted optional-input and failure case matches its stated result;
   unattempted cases are explicitly marked not run rather than passed.
6. No unexplained deviation, leaked partial artifact, unsafe device mutation,
   or unredacted sensitive evidence remains.

Section I remains a separate requirement for each newly tested release commit.
A successful local-artifact run or automated local-server run does not replace
real-device network-download evidence.
