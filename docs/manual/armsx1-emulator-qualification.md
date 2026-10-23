# ARMSX1 emulator qualification

EmuChef has two independent evidence paths for ARMSX1. Deterministic qualification runs in the regular offline test suite. Live Android emulator qualification is explicit, opt-in, and requires the exact disposable AVD described here. A pass in either path does not prove the other path, and an offline pass is not device or APK-signature evidence.

## Release and filename policy

The App Definition at `authored/apps/armsx1.yaml` owns the reusable APK source policy. It resolves the latest eligible stable GitHub release from `https://github.com/ARMSX2/ARMSX1`, excludes draft and prerelease releases, and requires the package identity `com.nanodata.armsx`. The anchored asset policy is:

```text
^ARMSX1-(?:release-[0-9]{8}-arm64-v8a|[0-9]+\.[0-9]+\.[0-9]+)\.apk$
```

This admits both supported stable naming forms:

- `ARMSX1-release-YYYYMMDD-arm64-v8a.apk`, such as `ARMSX1-release-20260925-arm64-v8a.apk`
- `ARMSX1-MAJOR.MINOR.PATCH.apk`, such as `ARMSX1-0.3.6.apk` and future three-component stable names

The match applies to the complete asset filename. Debug, source, nightly, malformed, or otherwise nonmatching assets are ineligible. The resolver selects one asset from the newest eligible stable release. If that release has no matching asset, resolution fails with `remote_asset_no_match` and does not fall back to an older release. If the release has more than one matching APK, resolution fails as ambiguous. Repository, package, prerelease, and App Artifact policy checks remain fail-closed.

GitHub release `0.3.6` is distinct from the Android package-manager version. The release contains `ARMSX1-0.3.6.apk` at:

```text
https://github.com/ARMSX2/ARMSX1/releases/download/0.3.6/ARMSX1-0.3.6.apk
```

The trusted publisher digest supplied for that asset is:

```text
121bccccea97a9051b061f25ec671039e613788b76e95c80ee5a21adf6127574
```

GitHub release `0.3.6` is distinct from the Android package-manager observations. Attempt 2 qualification observed the installed package report `versionName` `0.3.6` and `versionCode` `92183350` after both install runs. The two observations were equal, so the second run is a same-version replacement rather than an Android version upgrade. Treat the GitHub release tag, APK contents, and package-manager values as independently observed; do not infer one from another.

The qualification contract binds the raw App Definition and Recipe hashes, the selected release tag and asset URL, and the expected APK digest. Any change to those authored sources or release expectations requires deliberate contract and test review.

## Offline deterministic qualification

The offline tests use checked-in release metadata and a deterministic fake device. They prepare `app.armsx1.install` through the production catalog and runtime-configuration planner, inspect the reviewed plan, resolve the release through the production resolver, and execute the unchanged plan through sandboxed executor adapters. They do not use ADB, HTTPS, or a downloaded APK.

The fixture covers the real stable `0.3.6` release, the older dated ARM64 filename form, a future three-component stable filename, and a newer `0.5.0` stable release whose only asset has an ineligible filename. The latter case proves that selection fails closed instead of falling back to `0.3.6`. The tests also verify that drafts, prereleases, debug and source assets, nightly and malformed names, ambiguous matches, missing release data, and no-match behavior fail closed. Package identity, failed installation, blocked downstream work, replacement on a repeated deterministic run, and version-downgrade failure are covered with synthetic APKs and fake device state.

These tests run in the ordinary test suite and provide evidence appropriate for continuous integration. They do not prove real-device behavior, downloaded bytes, signing, or emulator identity.

## Read-only preflight diagnostic

Use the preflight diagnostic to identify operator-input or emulator-identity blockers without resolving a release, downloading an APK, installing a package, or mutating a device. The fixed target is exactly serial `emulator-5554` and AVD `EmuChef-ARMSX1-Android16-ARM64`. The Node diagnostic rejects any other serial, a missing or different AVD name, a missing dedication attestation, or a package allowlist other than exactly `com.nanodata.armsx` before starting Cargo. The ignored Rust diagnostic repeats the exact serial and AVD checks before any ADB access and uses only read-only observations.

```sh
CARGO_NET_OFFLINE=true \
EMUCHEF_RUN_ARMSX1_EMULATOR=1 \
EMUCHEF_ARMSX1_AVD_SERIAL=emulator-5554 \
EMUCHEF_ARMSX1_AVD_NAME=EmuChef-ARMSX1-Android16-ARM64 \
EMUCHEF_ARMSX1_DEDICATED_AVD=I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED \
EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST=com.nanodata.armsx \
node tools/qualify-armsx1-emulator.mjs --diagnose-preflight
```

Do not use `make qualify-armsx1-emulator` for this diagnostic. The Rust diagnostic record defaults to `docs/testing/recipe-qualification/armsx1/preflight-diagnostic-evidence.json`, and the Node dispatch record defaults to `docs/testing/recipe-qualification/armsx1/preflight-diagnostic-runner-evidence.json`. Override them with `EMUCHEF_ARMSX1_DIAGNOSTIC_EVIDENCE_PATH` and `EMUCHEF_ARMSX1_DIAGNOSTIC_RUNNER_EVIDENCE_PATH` when needed. Evidence paths must be absolute and safe: either the approved ARMSX1 evidence directory or a location outside the repository, never a `.chatgpt` path.

The diagnostic does not enumerate all attached devices and does not prove that only one emulator is running. It binds the exact serial and AVD identity for the selected target. A completed diagnostic pass is not live qualification. Both evidence records report live qualification as `unverified` unless the live workflow runs and records its own result.

## Live emulator qualification

Live qualification is never part of `make test` or CI. The operator procedure requires exactly serial `emulator-5554` and AVD `EmuChef-ARMSX1-Android16-ARM64`. Do not substitute another serial, another AVD, or a physical device.

```sh
EMUCHEF_RUN_ARMSX1_EMULATOR=1 \
EMUCHEF_ARMSX1_AVD_SERIAL=emulator-5554 \
EMUCHEF_ARMSX1_AVD_NAME=EmuChef-ARMSX1-Android16-ARM64 \
EMUCHEF_ARMSX1_DEDICATED_AVD=I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED \
EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST=com.nanodata.armsx \
make qualify-armsx1-emulator
```

Before starting the live workflow, run the read-only operator diagnostic above. It performs exact `emulator-5554` and AVD-name admission, requires the dedication attestation and exact package allowlist, and runs the read-only Rust diagnostic. During normal live invocation, the Node runner validates the selected target's AVD identity; it does not itself enforce the fixed serial. The ignored Rust live test independently rejects any serial other than `emulator-5554` before mutation.

These checks establish identity for the selected serial. They do not enumerate every attached emulator. Before mutation, the Rust test requires the selected serial to report an online, booted Android 16/API 36 ARM64 Google emulator with the expected AVD identity, QEMU properties, and a non-root shell. The package allowlist must contain exactly `com.nanodata.armsx`. The package must be absent before the first installation; a pre-existing package blocks the run or is reported according to the qualification admission rules rather than treated as proof of a successful run.

The live workflow uses the ordinary production resolver and reviewed execution plan through `RealAdbDevice`. It does not use a direct `adb install` shortcut. The reviewed install and package-edit target binding stays on the exact selected serial. Before installation the workflow applies these fail-closed checks:

1. The downloaded bytes must match the trusted release SHA-256.
2. The APK manifest package must be `com.nanodata.armsx`.
3. The APK must contain a nonempty native library under `lib/arm64-v8a/`.
4. `apksigner verify --print-certs` must succeed. The verifier must be available on `PATH` or in an Android SDK build-tools directory under `ANDROID_HOME` or `ANDROID_SDK_ROOT`.

A missing verifier is reported as `apk_preinstall_signature_verifier_unavailable`; a verifier process failure is `apk_preinstall_signature_verifier_failed`; and a rejected APK signature is `apk_preinstall_signature_rejected`. These local verifier failures are APK identity failures. They stop before ADB sends an install command, leave the install and version stages `NOT_RUN`, and record only checks that completed. `android_signing_failed` and the `install_signing` category are reserved for an Android package-manager signing rejection after an install command was issued.

A missing digest, hash mismatch, manifest inspection failure, wrong package, missing ARM64 native code, unavailable signer, invalid signature, or any other verification failure stops before installation. No fallback APK, alternate URL, or unverified install is allowed.

After a successful first installation, the same test may run the same plan a second time to observe replacement or package-manager behavior. The second run is allowed only when the first succeeds. The test does not wipe or reset the AVD, uninstall unrelated packages, retry indefinitely, or continue after an uncertain result. A failed or blocked attempt is evidence of that attempt, not successful qualification.

The Rust live record defaults to `docs/testing/recipe-qualification/armsx1/live-evidence.json`; the runner record defaults to `docs/testing/recipe-qualification/armsx1/runner-evidence.json`. Override them with `EMUCHEF_ARMSX1_EVIDENCE_PATH` and `EMUCHEF_ARMSX1_RUNNER_EVIDENCE_PATH`. These records separate APK verification, plan and review evidence, and ADB outcomes.

This manual covers ARMSX1 only. It is not evidence for a physical device or for another qualification workflow.
