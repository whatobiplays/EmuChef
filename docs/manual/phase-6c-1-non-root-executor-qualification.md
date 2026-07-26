# Phase 6C.1 Non-root Executor Qualification

Phase 6C.1 qualifies EmuChef's supported non-root executor operations. The
automated implementation can be ready while physical-device qualification and
the UI smoke remain unrun. Do not describe this phase as complete until the
manual evidence in this runbook has been captured.

## 1. Fixed qualification assets

All qualification assets are committed beneath
`tests/fixtures/phase-6c/non-root/`. The Android fixture has the exact package
`com.emuchef.fixture`, version `1.0.0`, launcher activity
`com.emuchef.fixture.MainActivity`, declared `CAMERA` permission, minimum SDK
30, and target SDK 35. Its documented test keystore is a deterministic fixture
input only; it is not a production signing identity or trust boundary.

The qualification catalog fragment is
`tests/fixtures/phase-6c/non-root/recipe/`. Its exact recipe is
`tests/fixtures/phase-6c/non-root/recipe/recipes/phase-6c-qualification.yaml`.
It remains outside `authored/` and is merged into an isolated app-cache overlay
only under the explicit UI opt-ins in section 6.

The only device roots owned by this procedure come from
`tests/fixtures/phase-6c/non-root/qualification-contract.json`:

1. `/sdcard/EmuChefQualification/com.emuchef.fixture/`
2. `/sdcard/Android/data/com.emuchef.fixture/files/`

The app-specific external root may be used only when that capability is
available. Never substitute another package, `/data`, a system path, an
arbitrary shared-storage path, or a caller-provided catalog.

ZIP entries are fully pre-scanned before extraction. Traversal, rooted paths,
Windows drive paths, empty normalized paths, duplicates, file/directory
conflicts, and symlinks identified by the ZIP crate's Unix-mode metadata are
rejected. ZIP formats do not require reliable symlink mode metadata, so an
entry with no symlink metadata cannot be inferred to be a symlink from content
alone.

## 2. Host verification

Run from the repository root:

```bash
git diff --check
cargo fmt --all -- --check

cd crates/emuchef-rust-backend
cargo check
cargo test
cargo check --features real-execution
cargo test --features real-execution
cd ../..

node --test tools/phase-6c-fixture.test.mjs

cd apps/emuchef-app
npm test -- --run
npm run typecheck
npm run lint
npm run build
cd ../..

cargo check --locked --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --no-default-features
cargo test --locked --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --no-default-features
cargo check --locked --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --no-default-features --features real-execution
cargo test --locked --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --no-default-features --features real-execution
```

Verify the committed APK with JDK 17, `platforms;android-35`, and Build Tools
`35.0.0`:

```bash
export JAVA_HOME="/absolute/path/to/jdk-17"
export ANDROID_SDK_ROOT="/absolute/path/to/android-sdk"
node tools/phase-6c-fixture.mjs \
  --apk tests/fixtures/phase-6c/non-root/android-fixture/fixture.apk \
  --metadata tests/fixtures/phase-6c/non-root/android-fixture/fixture-metadata.json \
  --checksum tests/fixtures/phase-6c/non-root/android-fixture/fixture.apk.sha256 \
  --aapt2 "$ANDROID_SDK_ROOT/build-tools/35.0.0/aapt2" \
  --apksigner "$ANDROID_SDK_ROOT/build-tools/35.0.0/apksigner"
```

Build to a temporary output and perform semantic-only verification. Rebuilt
bytes and checksum are deliberately not compared with the committed APK:

```bash
rebuilt_apk="$(mktemp -t emuchef-phase6c-rebuilt.XXXXXX.apk)"
bash scripts/build-phase-6c-android-fixture.sh --output "$rebuilt_apk"
node tools/phase-6c-fixture.mjs --semantic-only \
  --apk "$rebuilt_apk" \
  --metadata tests/fixtures/phase-6c/non-root/android-fixture/fixture-metadata.json \
  --checksum tests/fixtures/phase-6c/non-root/android-fixture/fixture.apk.sha256 \
  --aapt2 "$ANDROID_SDK_ROOT/build-tools/35.0.0/aapt2" \
  --apksigner "$ANDROID_SDK_ROOT/build-tools/35.0.0/apksigner"
rm -f -- "$rebuilt_apk"
```

## 3. Device selection and evidence facts

Use exactly one explicitly selected online device. It must run Android API 30
or newer, and `adb shell id -u` must not return `0`.

```bash
export EMUCHEF_TEST_DEVICE_SERIAL="REPLACE_WITH_PREPARED_DEVICE"
export EMUCHEF_TEST_PACKAGE_ALLOWLIST="com.emuchef.fixture"

adb devices -l
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.product.manufacturer
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.product.model
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.build.version.release
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.build.version.sdk
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.build.fingerprint
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.product.cpu.abi
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell id -u
adb version
git rev-parse HEAD
shasum -a 256 tests/fixtures/phase-6c/non-root/android-fixture/fixture.apk
```

Sanitize the selected identity in evidence; do not record the raw serial in a
normal result projection.

## 4. Individual ignored qualification groups

Each backend invocation requires the global opt-in and exactly one matching
group. The commands below use a clean `env` invocation so another group cannot
remain enabled accidentally. They all execute a real `ExecutionPlan` through
`ExecutorRunner<RealAdbDevice>`.

### 4.1 Install and package

```bash
env -u EMUCHEF_RUN_REAL_ADB_COPY_EXTRACTION_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_PERMISSION_APPOP_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_LAUNCH_FORCE_STOP_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_CLEANUP_FAILURE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_INSTALL_PACKAGE_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::manual_real_adb_install_package_group \
  -- --ignored --exact --nocapture
```

### 4.2 Copy and extraction

```bash
env -u EMUCHEF_RUN_REAL_ADB_INSTALL_PACKAGE_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_PERMISSION_APPOP_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_LAUNCH_FORCE_STOP_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_CLEANUP_FAILURE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_COPY_EXTRACTION_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::manual_real_adb_copy_extraction_group \
  -- --ignored --exact --nocapture
```

### 4.3 Permission and app-op

```bash
env -u EMUCHEF_RUN_REAL_ADB_INSTALL_PACKAGE_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_COPY_EXTRACTION_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_LAUNCH_FORCE_STOP_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_CLEANUP_FAILURE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_PERMISSION_APPOP_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::manual_real_adb_permission_appop_group \
  -- --ignored --exact --nocapture
```

### 4.4 Launch and force-stop

```bash
env -u EMUCHEF_RUN_REAL_ADB_INSTALL_PACKAGE_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_COPY_EXTRACTION_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_PERMISSION_APPOP_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_CLEANUP_FAILURE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_LAUNCH_FORCE_STOP_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::manual_real_adb_launch_force_stop_group \
  -- --ignored --exact --nocapture
```

### 4.5 Controlled cleanup failure

This group skips one declared qualification root through controlled
test-owned injection. It does not touch an unrelated package or path.

```bash
env -u EMUCHEF_RUN_REAL_ADB_INSTALL_PACKAGE_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_COPY_EXTRACTION_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_PERMISSION_APPOP_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_LAUNCH_FORCE_STOP_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_CLEANUP_FAILURE_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::manual_real_adb_cleanup_failure_group \
  -- --ignored --exact --nocapture
```

## 5. Expected outcomes and cleanup

Executor progress proceeds through `checking_skip_conditions`, `executing`,
`verifying`, and `finished` as applicable. Final step states are `succeeded`,
`skipped`, `failed`, or `blocked`. An already-satisfied condition must be
`skipped`; a dependent step after failure must be `blocked`. Verification and
execution failures remain failures. A missing API, package manager, activity
manager, shared-storage capability, permission, or app-op is reported as the
distinct `unsupported` qualification classification rather than success,
failure, or skip.

Cleanup is qualification-harness behavior, not a production recipe step. It
force-stops the fixture, revokes `CAMERA` where supported, resets the fixture's
app-op state where supported, uninstalls only `com.emuchef.fixture`, and removes
only contract-declared roots. Cleanup runs after successful, failed, partial,
or aborted setup results returned by the implemented qualification groups once
setup has begun. A process abort or panic can bypass harness cleanup; always run
the residual-state checks and manual cleanup below after an interrupted run. A
cleanup failure is a separate `cleanup_failed` or residual-state result and does
not overwrite the original operation result.

Check residual state:

```bash
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell pm path com.emuchef.fixture
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell dumpsys package com.emuchef.fixture
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell appops get com.emuchef.fixture CAMERA
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell pidof com.emuchef.fixture
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell test -e \
  /sdcard/EmuChefQualification/com.emuchef.fixture
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell test -e \
  /sdcard/Android/data/com.emuchef.fixture/files
```

If controlled failure leaves a declared root, remove only that exact root
after rechecking it against the contract.

## 6. Combined UI smoke

The UI smoke requires all four functional group opt-ins, excludes the cleanup
failure opt-in, and enables the qualification catalog overlay:

```bash
env -u EMUCHEF_RUN_REAL_ADB_CLEANUP_FAILURE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_INSTALL_PACKAGE_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_COPY_EXTRACTION_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_PERMISSION_APPOP_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_LAUNCH_FORCE_STOP_TESTS=1 \
  EMUCHEF_PHASE_6C_QUALIFICATION_CATALOG=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  npm --prefix apps/emuchef-app run tauri:dev:real
```

Select the prepared device, choose the Phase 6C qualification plan, bind the
committed APK and corpus inputs, create a fresh review, confirm real execution,
observe the expected progress and final classification, then perform guarded
cleanup and residual checks.

The backend-only ignored harness cannot consume Tauri's process-local opaque
qualification handles. It therefore proves the production executor path but
does not prove the product qualification-authority boundary. Deterministic
Tauri tests separately prove that stale, invalid, reconnected, or
inventory-invalidated authority prevents any `startExecution` mutation
request. Only the combined UI smoke exercises both boundaries together.

## 7. Evidence record

Record:

1. Device model
2. Android version
3. API level
4. Build fingerprint
5. ABI
6. Sanitized device identity
7. Confirmed non-root status
8. Platform-Tools revision
9. EmuChef commit
10. Fixture APK SHA-256
11. Qualification groups executed
12. Outcome and final classification for each group
13. Unsupported capabilities
14. Cleanup classification
15. Residual package, permission, app-op, process, and filesystem state

Residual reports may contain only fixed classifications and the
manifest-declared test-owned roots. Physical tests and the UI smoke are unrun
unless their exact command output and this evidence are present.
