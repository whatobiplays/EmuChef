#!/usr/bin/env bash
# Build the deliberately dependency-free Phase 6C.1 Android fixture with SDK tools.
set -euo pipefail

readonly SDK_PLATFORM="android-35"
readonly BUILD_TOOLS_VERSION="35.0.0"
readonly JAVA_RELEASE="17"
readonly FIXTURE_RELATIVE_PATH="tests/fixtures/phase-6c/non-root/android-fixture"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="$repo_root/$FIXTURE_RELATIVE_PATH"
android_sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
java_home="${JAVA_HOME:-}"
output_apk="$fixture_root/fixture.apk"
work_root=""

usage() {
  printf '%s\n' "Usage: $0 [--output <apk-path>] [--work-dir <directory>]"
}

while (($# > 0)); do
  case "$1" in
    --output)
      output_apk="${2:?--output requires a path}"
      shift 2
      ;;
    --work-dir)
      work_root="${2:?--work-dir requires a directory}"
      shift 2
      ;;
    --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$android_sdk_root" || -z "$java_home" ]]; then
  printf '%s\n' "Set ANDROID_SDK_ROOT and JAVA_HOME to the locked Android SDK and JDK 17." >&2
  exit 2
fi

readonly platform_jar="$android_sdk_root/platforms/$SDK_PLATFORM/android.jar"
readonly build_tools_root="$android_sdk_root/build-tools/$BUILD_TOOLS_VERSION"
readonly aapt2="$build_tools_root/aapt2"
readonly d8="$build_tools_root/d8"
readonly zipalign="$build_tools_root/zipalign"
readonly apksigner="$build_tools_root/apksigner"
readonly javac="$java_home/bin/javac"
readonly jar="$java_home/bin/jar"

for tool in "$platform_jar" "$aapt2" "$d8" "$zipalign" "$apksigner" "$javac" "$jar"; do
  [[ -e "$tool" ]] || { printf 'Required SDK or JDK tool is missing: %s\n' "$tool" >&2; exit 2; }
done

if ! "$javac" -version 2>&1 | grep -Eq "javac ${JAVA_RELEASE}(\\.| )"; then
  printf 'JDK %s is required for the Phase 6C.1 fixture build.\n' "$JAVA_RELEASE" >&2
  exit 2
fi

if [[ -z "$work_root" ]]; then
  work_root="$(mktemp -d "${TMPDIR:-/tmp}/emuchef-phase-6c-fixture.XXXXXX")"
  trap 'rm -rf "$work_root"' EXIT
else
  mkdir -p "$work_root"
fi

mkdir -p "$(dirname "$output_apk")" "$work_root/classes" "$work_root/dex"

"$javac" --release "$JAVA_RELEASE" -classpath "$platform_jar" -d "$work_root/classes" \
  "$fixture_root/src/com/emuchef/fixture/MainActivity.java"
"$aapt2" compile --dir "$fixture_root/res" -o "$work_root/resources.zip"
"$aapt2" link --auto-add-overlay --manifest "$fixture_root/AndroidManifest.xml" -I "$platform_jar" \
  -R "$work_root/resources.zip" -o "$work_root/unsigned.apk"
"$jar" cf "$work_root/classes.jar" -C "$work_root/classes" .
"$d8" --min-api 30 --output "$work_root/dex" "$work_root/classes.jar"
"$jar" uf "$work_root/unsigned.apk" -C "$work_root/dex" classes.dex
"$zipalign" -f 4 "$work_root/unsigned.apk" "$work_root/aligned.apk"
"$apksigner" sign \
  --ks "$fixture_root/test-only-emuchef-fixture-signing.jks" \
  --ks-key-alias emuchef-fixture-test \
  --ks-pass pass:fixture-only-not-a-secret \
  --key-pass pass:fixture-only-not-a-secret \
  --out "$output_apk" \
  "$work_root/aligned.apk"

printf 'Built Phase 6C.1 fixture APK: %s\n' "$output_apk"
