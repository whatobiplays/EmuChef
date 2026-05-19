# Manual Real-Device RetroArch Apply Matrix

This is a manual checklist for opt-in RetroArch real-device validation.

This document does not prove validation has been completed. Treat it as an
operator checklist until a filled result record with logs and artifacts is
attached to a specific validation run.

Real `apply` mutates the selected Android device. The workflow must only target
an explicitly selected ADB serial through `EMUCHEF_DEVICE_SERIAL`; do not rely
on implicit ADB device selection. This workflow is not part of normal CI.

App-private RetroArch paths under `/data/user/0/com.retroarch.aarch64/...` may
contain user data. Back up RetroArch data before mutating runs, especially on
root-capable devices where app-private writes may be possible.

## Current Repo Facts

- Runnable CLI module: `python -m emuchef`
- Authored RetroArch recipe: `authored/recipes/app.retroarch.provision.yaml`
- RetroArch recipe id: `app.retroarch.provision`
- Expected package name: `com.retroarch.aarch64`
- Optional config input id: `retroarch_cfg`
- Optional config target: `/sdcard/Android/data/com.retroarch.aarch64/files/retroarch.cfg`
- Frontend artifact group: `retroarch_assets`
- Core artifact group: `retroarch_cores`
- Core system file artifact group: `retroarch_core_system_files`
- Example root-capable plan id: `ayaneo.pocket_s_mini.base`
- Other current plan ids: `ayaneo.generic.base`, `ayaneo.konkr_pocket_fit.base`,
  `ayaneo.pocket_air_mini.base`, `ayaneo.pocket_s2.base`

Use the device plan that matches the attached device. The example below uses
`ayaneo.pocket_s_mini.base` because its current profile declares both
`root_shell: true` and `app_data_write: true`, which makes app-private
RetroArch copy paths manually inspectable when the real device actually permits
root access.

## Prerequisites

- A supported test device that matches one of the authored device plans.
- ADB installed and available on `PATH`, or configured with `--adb` or
  `EMUCHEF_ADB`.
- The device is connected, authorized, and intentionally selected by serial.
- The tester understands that real apply installs an APK, launches and force
  stops RetroArch, grants permissions/appops where applicable, copies files,
  and may overwrite destination files according to the authored copy policy.
- Network access is available for uncached remote artifacts, or the artifact
  cache already contains the required files. Artifact resolution uses the
  plan-file directory for `.emuchef_cache/artifacts` and `.emuchef_runtime`.
- Any existing RetroArch app-private data has been backed up before real apply.

Set explicit variables before every manual run:

```bash
export EMUCHEF_DEVICE_SERIAL="<adb-serial>"
export EMUCHEF_DEVICE_PLAN="ayaneo.pocket_s_mini.base"
export EMUCHEF_PLAN_OUT="/tmp/emuchef-retroarch-plan.yaml"
```

If the local shell does not provide `python`, use the repo virtual environment
binary in the commands below:

```bash
export EMUCHEF_PYTHON="./.venv/bin/python"
```

Otherwise:

```bash
export EMUCHEF_PYTHON="python"
```

## Manual Commands

### 1. Preflight

These commands read device state and do not intentionally mutate the device:

```bash
adb -s "$EMUCHEF_DEVICE_SERIAL" get-state
adb -s "$EMUCHEF_DEVICE_SERIAL" shell getprop ro.product.model
adb -s "$EMUCHEF_DEVICE_SERIAL" shell getprop ro.build.version.release
adb -s "$EMUCHEF_DEVICE_SERIAL" shell getprop ro.build.version.sdk
adb -s "$EMUCHEF_DEVICE_SERIAL" shell pm path com.retroarch.aarch64 || true
adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c true || true
```

Record whether `su -c true` succeeds. App-private inspections and app-private
copy expectations only apply to root-capable devices where the executor has the
required `root_shell` and `app_data_write` capabilities.

### 2. Catalog Validation

```bash
PYTHONPATH=src "$EMUCHEF_PYTHON" -m emuchef validate --authored-root authored
```

Expected observation: validation exits successfully or reports authored
diagnostics that must be fixed before device testing.

### 3. Device Detection

```bash
PYTHONPATH=src "$EMUCHEF_PYTHON" -m emuchef detect \
  --serial "$EMUCHEF_DEVICE_SERIAL"
```

Expected observation: the selected serial, model, Android version, and root
availability are reported for the intended device.

### 4. Plan Generation

Without optional RetroArch config:

```bash
PYTHONPATH=src "$EMUCHEF_PYTHON" -m emuchef plan \
  --authored-root authored \
  --device-plan "$EMUCHEF_DEVICE_PLAN" \
  --serial "$EMUCHEF_DEVICE_SERIAL" \
  --output "$EMUCHEF_PLAN_OUT" \
  --verbose
```

With optional RetroArch config:

```bash
export EMUCHEF_RETROARCH_CFG="/absolute/path/to/retroarch.cfg"

PYTHONPATH=src "$EMUCHEF_PYTHON" -m emuchef plan \
  --authored-root authored \
  --device-plan "$EMUCHEF_DEVICE_PLAN" \
  --serial "$EMUCHEF_DEVICE_SERIAL" \
  --bind "retroarch_cfg=$EMUCHEF_RETROARCH_CFG" \
  --output "$EMUCHEF_PLAN_OUT" \
  --verbose
```

Expected observation: the emitted planning result contains an `execution_plan`
and runnable RetroArch steps. The config-present run should include the
`seed_retroarch_cfg` step when the bound file exists and validation accepts it.

### 5. Dry Run

```bash
PYTHONPATH=src "$EMUCHEF_PYTHON" -m emuchef apply \
  --plan-file "$EMUCHEF_PLAN_OUT" \
  --dry-run \
  --verbose
```

`--dry-run` uses the in-memory `DryRunAdb` instead of the selected device, so it
avoids intentional device mutation. It still runs host-side executor logic,
including artifact resolution and extraction, so it may use network access and
the artifact cache.

Expected observation: the summary reports dry-run success or a specific failed
step. Capture the full output because dry-run failures usually point to plan,
artifact, cache, or host-environment problems before device mutation.

### 6. Real Apply

Warning: the next command mutates the selected device identified by
`EMUCHEF_DEVICE_SERIAL`.

```bash
PYTHONPATH=src "$EMUCHEF_PYTHON" -m emuchef apply \
  --plan-file "$EMUCHEF_PLAN_OUT" \
  --serial "$EMUCHEF_DEVICE_SERIAL" \
  --verbose
```

Expected observation: the summary reports execution success or the first failed
step and blocked downstream steps. Capture stdout/stderr, the generated plan,
and relevant ADB inspection output.

## Validation Matrix

| Case | Purpose | Preconditions | Command or inspection | Expected observation | Evidence to capture | Mutates device? | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Validate authored catalog | Confirm authored YAML loads before device work | Repo checkout is current | `PYTHONPATH=src "$EMUCHEF_PYTHON" -m emuchef validate --authored-root authored` | Validation exits successfully | Command output | No | Fix authored diagnostics before continuing |
| Detect selected device | Confirm explicit serial and root facts | `EMUCHEF_DEVICE_SERIAL` is set | `PYTHONPATH=src "$EMUCHEF_PYTHON" -m emuchef detect --serial "$EMUCHEF_DEVICE_SERIAL"` | Intended model, Android version, and root availability are reported | Command output | No | Do not continue if the serial is not the intended device |
| Generate RetroArch plan | Emit current execution plan from authored YAML | Catalog validates; device is detected | Plan generation command above | Planning result contains an execution plan with RetroArch steps | Plan file and command output | No | The CLI detects the selected device while planning |
| Dry-run apply | Exercise executor flow without real ADB mutation | Plan file exists | Dry-run command above | Dry-run summary reports success or concrete failed step | Full dry-run output and cache notes | No device mutation | May still use network/cache/artifact extraction |
| Real apply | Run full artifact-based apply | Backup completed; correct serial selected; dry-run reviewed | Real apply command above | RetroArch install, permission, copy, verify, and launch steps execute or fail with diagnostics | Full output, plan file, ADB logs | Yes | This is the only normal checklist command that intentionally applies the plan |
| Optional config absent | Verify optional unbound config behavior | Generate plan without `--bind retroarch_cfg=...` | Inspect plan/run output for `seed_retroarch_cfg` | Config copy step is absent, deselected, or not runnable due to unbound optional input | Plan excerpt and output | Plan: no; real apply: yes if applied | Do not treat absence as failure unless downstream behavior breaks |
| Optional config present | Verify config file seeding | `EMUCHEF_RETROARCH_CFG` points to an existing `.cfg` file | Plan with `--bind "retroarch_cfg=$EMUCHEF_RETROARCH_CFG"`; after apply inspect config target | `seed_retroarch_cfg` runs and `/sdcard/Android/data/com.retroarch.aarch64/files/retroarch.cfg` exists | Plan excerpt, apply output, path inspection | Yes during apply | Binding name is `retroarch_cfg` |
| Grouped core extraction/copy | Verify `retroarch_cores` handling | Artifact downloads/cache available; root-capable app-private write expected for copy | Inspect apply output for `extract_cores` and `copy_cores`; root-capable inspection: `adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'ls /data/user/0/com.retroarch.aarch64/cores'` | Core zips resolve/extract; copy succeeds only when app-private writes are supported | Step output and directory listing if root-capable | Yes during apply | No-root devices should record expected block/failure for app-private copy |
| Frontend assets | Verify assets, autoconfig, cheats, database/rdb, info, overlays, and GLSL shaders | Artifact downloads/cache available | Inspect apply output; public check: `adb -s "$EMUCHEF_DEVICE_SERIAL" shell test -d /storage/emulated/0/RetroArch/cheats` | Shared cheats path exists; app-private frontend paths succeed only with app-private write support | Step output and selected path checks | Yes during apply | App-private targets include `assets`, `autoconfig`, `database/rdb`, `info`, `overlays`, and `shaders/shaders_glsl` |
| System files | Verify `retroarch_core_system_files` handling | Artifact downloads/cache available | `adb -s "$EMUCHEF_DEVICE_SERIAL" shell test -d /storage/emulated/0/RetroArch/system` | `/storage/emulated/0/RetroArch/system` exists with Dolphin, FBNeo, and PPSSPP expected subpaths when copied | Directory listing and apply output | Yes during apply | Recipe verify checks `dolphin-emu`, `fbneo`, and `PPSSPP` |
| App-private core path, no-root | Confirm unsupported app-private behavior is documented | Device/profile lacks both `root_shell` and `app_data_write` | Real apply output for `copy_cores` or app-private frontend copy steps | App-private copy fails or is blocked with `app_data_write_unavailable`; downstream dependent steps are blocked/not run | Failure output and selected profile id | Yes if real apply reached copy step | Do not force root inspection on no-root devices |
| App-private core path, root-capable | Confirm app-private copy path can be inspected | Device/profile supports root and app-private write; `su -c true` succeeds | `adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'test -d /data/user/0/com.retroarch.aarch64/cores'` | Directory exists after successful copy | Apply output and root listing | Yes during apply | Root-capable expectation still depends on real device root behavior |
| Permissions/appops | Verify `grant_permissions` step behavior | RetroArch package installed or install step succeeds | Inspect apply summary permission actions; optional manual checks: `adb -s "$EMUCHEF_DEVICE_SERIAL" shell dumpsys package com.retroarch.aarch64` and root/appops checks when allowed | Runtime permission actions execute, fail with warnings, or are not applicable by API/root condition; `MANAGE_EXTERNAL_STORAGE` appop only applies when rooted | Permission action summary and relevant dumpsys/appops output | Yes during apply | Step policy is warning-oriented for optional permissions |
| Launch verification | Confirm app can start after apply | Package installed | `adb -s "$EMUCHEF_DEVICE_SERIAL" shell monkey -p com.retroarch.aarch64 -c android.intent.category.LAUNCHER 1` | Launch command succeeds; UI appears or logcat shows launch attempt | Command output, screenshot/logcat if useful | Yes, launches app | Also inspect `adb -s "$EMUCHEF_DEVICE_SERIAL" shell pm path com.retroarch.aarch64` |
| Rerun/idempotency expectations | Observe repeat apply behavior | First run completed or failed after partial writes | Rerun plan generation and dry-run before any second real apply | Existing package may skip install; merge/replace copy policies determine destination behavior; failures should remain diagnosable | Before/after outputs and changed notes | Yes if real apply rerun | Back up again if user data changed after first run |
| Failure capture and blocked downstream behavior | Record actual failure source | Any failed dry-run or real apply | Capture verbose output and inspect failed step plus subsequent summary counts | Failed step reports a concrete error; dependent steps are blocked or not run according to executor summary | stdout/stderr, plan, cache state, relevant `adb` output | Depends on failed command | Do not convert this checklist into a claim of validation success |

Useful safe inspections after a real apply:

```bash
adb -s "$EMUCHEF_DEVICE_SERIAL" shell pm path com.retroarch.aarch64
adb -s "$EMUCHEF_DEVICE_SERIAL" shell monkey -p com.retroarch.aarch64 -c android.intent.category.LAUNCHER 1
adb -s "$EMUCHEF_DEVICE_SERIAL" shell test -d /storage/emulated/0/RetroArch/cheats
adb -s "$EMUCHEF_DEVICE_SERIAL" shell test -d /storage/emulated/0/RetroArch/system
adb -s "$EMUCHEF_DEVICE_SERIAL" shell test -f /sdcard/Android/data/com.retroarch.aarch64/files/retroarch.cfg || true
```

Root-capable inspections only:

```bash
adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'test -d /data/user/0/com.retroarch.aarch64/assets'
adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'test -d /data/user/0/com.retroarch.aarch64/autoconfig'
adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'test -d /data/user/0/com.retroarch.aarch64/database/rdb'
adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'test -d /data/user/0/com.retroarch.aarch64/info'
adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'test -d /data/user/0/com.retroarch.aarch64/overlays'
adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'test -d /data/user/0/com.retroarch.aarch64/shaders/shaders_glsl'
adb -s "$EMUCHEF_DEVICE_SERIAL" shell su -c 'test -d /data/user/0/com.retroarch.aarch64/cores'
```

## Result Record Template

Copy this template into the issue, PR, or validation artifact that records an
actual manual run. Redact the full serial unless the storage location is
private.

```markdown
## RetroArch Real-Device Apply Result

- Date:
- Tester:
- Git commit SHA:
- Branch:
- Dirty working tree: yes/no
- Device model:
- Android version:
- Android API level:
- Root/no-root status:
- Device serial: redacted, last 4 characters only
- Device plan:
- Recipe/config inputs used:
- Command run:
- Pass/fail:
- Logs captured:
- Generated plan path or attached artifact:
- Cache/network notes:
- Follow-up bug links:
- Freeform notes:
```

## Failure Recovery Notes

- Preserve the generated plan and verbose command output before rerunning.
- If artifact resolution fails, record whether the failure came from network,
  TLS verification, URL availability, cache state, or host filesystem access.
- If an app-private copy step fails on a no-root device, record it as expected
  unsupported behavior unless the selected device profile claimed
  `root_shell: true` and `app_data_write: true`.
- If a step fails, inspect the executor summary for failed, blocked, and not-run
  counts before rerunning. Downstream steps may be blocked because the executor
  stops useful dependent work after an upstream failure.
- Rerun catalog validation and dry-run after changing bindings, device plan,
  cache contents, or authored YAML.
