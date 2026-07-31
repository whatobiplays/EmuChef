# Phase 6C.2 Root Executor Qualification

Phase 6C.2 qualifies only the privileged behavior already reachable through
EmuChef's production executor. It does not add root commands, recipe step
types, ownership changes, permission-mode changes, SELinux changes, remounting,
system writes, Magisk integration, or privileged package-manager behavior.

Automated implementation can be ready while manual rooted-device qualification
remains pending. Physical success must not be claimed without completed command
output and a filled evidence record based on
`phase-6c2-root-executor-evidence-template.md`.

## 1. Authoritative operation inventory

The production-supported root surface is:

1. The bounded root probe `adb -s <serial> shell su -c id`.
2. `path_exists` and `path_is_dir` for `/data/data/...` and `/data/user/...`.
3. `mkdir -p`, file removal, and recursive tree removal used internally by
   `copy_files` and qualification cleanup.
4. Staged host-to-private file and directory copy.
5. On-device private file copy and recursive directory copy.
6. Private-path skip predicates and verification.
7. Cleanup of exact qualification-owned children.

`ExecutorRunner` performs root preflight before a root-capable step or private
predicate. Current production behavior permits at most one successful root
preflight in an executor run unless the implementation intentionally
revalidates authority. Qualification tests characterize this behavior; they do
not introduce a caching requirement. Authority loss is represented by a
successful preflight followed by denial of the first privileged operation.

## 2. Fixed assets and path authority

The existing Phase 6C.1 fixture remains the application authority:

- APK: `tests/fixtures/phase-6c/non-root/android-fixture/fixture.apk`
- package: `com.emuchef.fixture`
- corpus: `tests/fixtures/phase-6c/non-root/corpus/`

The root contract is
`fixtures/android/phase-6c2-root/qualification-contract.json`. Only normalized,
non-root children beneath these exact prefixes are owned:

1. `/data/data/com.emuchef.fixture/emuchef-qualification-data/`
2. `/data/user/0/com.emuchef.fixture/emuchef-qualification-user/`

The different child names prevent `/data/data` aliasing `/data/user/0` from
making one case satisfy or clean the other. The harness refuses prefix equality,
traversal, unrelated packages, siblings, partial allowlists, or additional
prefixes before ADB runs. Android user 0 is the only supported user for this
qualification contract.

The cleanup-failure group leaves one unique child such as:

```text
/data/user/0/com.emuchef.fixture/emuchef-qualification-user/cleanup-failure-123-456/
```

It never leaves the approved prefix itself as the reported residual.

## 3. Rooted-device prerequisites

Use one deliberately prepared Android device where:

1. `adb devices -l` reports exactly one online device.
2. The selected serial is exact and is not recorded unsanitized in evidence.
3. `su -c id` is supported and returns UID 0 after any interactive root-manager approval.
4. The active Android user is user 0.
5. The fixture APK may be installed or replaced through the production
   `ExecutionPlan` used by the mutating groups.
6. Platform-Tools is available as `adb` and its version is recorded.

Magisk may supply the compatible `su` command, but EmuChef does not call Magisk
APIs, install modules, or alter root-manager policy. Missing `su`, denial,
timeouts, transport failures, and unexpected responses remain distinct
preflight outcomes. Do not alter root policy automatically to manufacture a
failure case.

Capture read-only facts before qualification:

```bash
adb devices -l
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.product.manufacturer
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.product.model
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.build.version.release
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.build.version.sdk
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.build.fingerprint
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell getprop ro.product.cpu.abi
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell cmd activity get-current-user
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell su -v
adb version
git rev-parse HEAD
shasum -a 256 tests/fixtures/phase-6c/non-root/android-fixture/fixture.apk
```

Treat `su -v` as optional evidence when the installed implementation supports
it. Sanitize the selected device identity and build fingerprint as required by
the evidence-storage policy.

## 4. Required environment authority

Set the common values exactly:

```bash
export EMUCHEF_RUN_REAL_ADB_TESTS=1
export EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1
export EMUCHEF_TEST_DEVICE_SERIAL="REPLACE_WITH_THE_ONLY_PREPARED_DEVICE"
export EMUCHEF_TEST_PACKAGE_ALLOWLIST="com.emuchef.fixture"
export EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST="/data/data/com.emuchef.fixture/emuchef-qualification-data/,/data/user/0/com.emuchef.fixture/emuchef-qualification-user/"
```

Every mutating group additionally requires:

```bash
export EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS=1
```

Exactly one group variable may equal `1`. The harness validates all common,
group, package, path, destructive, and serial authority before querying ADB.
It then requires exactly one online device matching the selected serial.

## 5. Individual qualification groups

Run commands from the repository root. Each command clears every other group
so inherited shell state cannot widen authority.

### 5.1 Root preflight

This group is non-mutating and does not require the destructive opt-in.

```bash
env -u EMUCHEF_RUN_REAL_ADB_ROOT_FILESYSTEM_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_COPY_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_COMBINED_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_CLEANUP_FAILURE_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_PREFLIGHT_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST="$EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST" \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::root_qualification::manual_real_adb_root_preflight_group \
  -- --ignored --exact --nocapture
```

### 5.2 Private predicates and filesystem mutation

```bash
env -u EMUCHEF_RUN_REAL_ADB_ROOT_PREFLIGHT_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_COPY_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_COMBINED_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_CLEANUP_FAILURE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_FILESYSTEM_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST="$EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST" \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::root_qualification::manual_real_adb_root_filesystem_group \
  -- --ignored --exact --nocapture
```

### 5.3 Privileged copy

```bash
env -u EMUCHEF_RUN_REAL_ADB_ROOT_PREFLIGHT_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_FILESYSTEM_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_COMBINED_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_CLEANUP_FAILURE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_COPY_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST="$EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST" \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::root_qualification::manual_real_adb_root_copy_group \
  -- --ignored --exact --nocapture
```

This group covers all six copy-qualification aspects through
`ExecutorRunner<RealAdbDevice>`:

1. Host file staging into a private source child.
2. Recursive host directory staging into a different private source child.
3. On-device private file copy into a distinct destination child.
4. Recursive on-device private directory copy into a distinct destination child.
5. Verification of both staged sources and both copied destinations.
6. Cleanup and absence checks for the exact two contract-owned group roots.

Both on-device operations are authored `copy_files` steps in the reviewed
`ExecutionPlan`; qualification cleanup is the only direct adapter activity in
this group.

### 5.4 Combined executor workflow

```bash
env -u EMUCHEF_RUN_REAL_ADB_ROOT_PREFLIGHT_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_FILESYSTEM_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_COPY_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_CLEANUP_FAILURE_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_COMBINED_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST="$EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST" \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::root_qualification::manual_real_adb_root_combined_group \
  -- --ignored --exact --nocapture
```

The combined plan performs root preflight, fixture installation or installed
skip, host staging, placement under both aliases, on-device file copy,
recursive directory copy, dependent verification, and final executor
reporting. Cleanup uses the same production ADB adapter after exact
qualification-ownership validation because the authored schema has no removal
step. This is a backend reviewed-plan qualification boundary; existing Tauri
tests separately cover stale and invalid review/device authority.

### 5.5 Controlled cleanup failure

```bash
env -u EMUCHEF_RUN_REAL_ADB_ROOT_PREFLIGHT_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_FILESYSTEM_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_COPY_TESTS \
  -u EMUCHEF_RUN_REAL_ADB_ROOT_COMBINED_TESTS \
  EMUCHEF_RUN_REAL_ADB_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS=1 \
  EMUCHEF_RUN_REAL_ADB_ROOT_CLEANUP_FAILURE_TESTS=1 \
  EMUCHEF_TEST_DEVICE_SERIAL="$EMUCHEF_TEST_DEVICE_SERIAL" \
  EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture \
  EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST="$EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST" \
  cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor_real_adb_tests::root_qualification::manual_real_adb_root_cleanup_failure_group \
  -- --ignored --exact --nocapture
```

The expected report keeps `operation: succeeded` and `cleanup: failed`, and
lists exactly one unique `cleanup-failure-<run-id>` child. Remove that exact
reported child manually after preserving evidence.

## 6. Result interpretation and cleanup

Reports contain fixed classifications and approved residual paths. Arbitrary
command errors, raw serials, and unapproved paths are not serialized.

- `preflight_failed`: root was not granted before the operation.
- `operation_failed`: preflight succeeded but a supported privileged operation failed.
- `cleanup_failed`: cleanup failed independently of the operation outcome.
- `succeeded`: the relevant phase completed.

The normal mutating groups remove and verify every exact child they created.
After interruption, panic, or controlled cleanup failure, inspect only the
approved prefixes:

```bash
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell su -c \
  "find /data/data/com.emuchef.fixture/emuchef-qualification-data -mindepth 1 -maxdepth 2 -print"
adb -s "$EMUCHEF_TEST_DEVICE_SERIAL" shell su -c \
  "find /data/user/0/com.emuchef.fixture/emuchef-qualification-user -mindepth 1 -maxdepth 2 -print"
```

Before manual removal, compare the exact residual to the committed contract.
Remove only the reported child, never `/data/data`, `/data/user`, the package
directory, or either contract prefix. Verify the exact child no longer exists.

## 7. Host verification and evidence status

Run:

```bash
cargo fmt --all --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --check
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets --all-features -- -D warnings
make test
git diff --check
```

The repository has no root `Cargo.toml`; root-level `cargo test -p
emuchef-rust-backend` and `cargo test --workspace` are structurally
inapplicable. Use the manifest-path commands above.

The ignored tests and manual denial/revocation procedures are unrun unless
their exact output and a completed evidence record exist. Host tests alone do
not qualify rooted hardware, the combined UI, packaging, or release readiness.
