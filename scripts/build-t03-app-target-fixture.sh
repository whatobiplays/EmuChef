#!/usr/bin/env bash
# Build the source-only synthetic app used by generic T03 App Target tests.
set -euo pipefail

readonly SDK_PLATFORM="android-35"
readonly BUILD_TOOLS_VERSION="35.0.0"
readonly JAVA_RELEASE="17"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
fixture_root="$repo_root/tests/fixtures/t03_app_target_android"
android_sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
java_home="${JAVA_HOME:-}"
output_apk=""
work_root=""
output_root=""

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
  printf '%s\n' "Set ANDROID_SDK_ROOT and JAVA_HOME to the installed Android SDK and JDK 17." >&2
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
readonly keytool="$java_home/bin/keytool"

for tool in "$platform_jar" "$aapt2" "$d8" "$zipalign" "$apksigner" "$javac" "$jar" "$keytool"; do
  [[ -e "$tool" ]] || { printf 'Required SDK or JDK tool is missing: %s\n' "$tool" >&2; exit 2; }
done

if ! [[ $("$javac" -version 2>&1) =~ ^javac[[:space:]]+([0-9]+)([.]|$) ]] \
  || (( BASH_REMATCH[1] < JAVA_RELEASE )); then
  printf 'JDK %s or newer is required for the T03 fixture build.\n' "$JAVA_RELEASE" >&2
  exit 2
fi

if [[ -z "$output_apk" ]]; then
  output_root="$(mktemp -d "${TMPDIR:-/tmp}/emuchef-t03-app-target.XXXXXX")"
  output_apk="$output_root/t03-app-target-fixture.apk"
else
  output_parent="$(dirname "$output_apk")"
  mkdir -p "$output_parent"
  output_parent="$(cd "$output_parent" && pwd -P)"
  output_apk="$output_parent/$(basename "$output_apk")"
fi
case "$output_apk" in
  "$repo_root"/*)
    printf '%s\n' "Generated APKs must remain outside the repository." >&2
    exit 2
    ;;
esac

if [[ -z "$work_root" ]]; then
  work_root="$(mktemp -d "${TMPDIR:-/tmp}/emuchef-t03-app-target-build.XXXXXX")"
  trap 'rm -rf "$work_root"' EXIT
else
  mkdir -p "$work_root"
fi
mkdir -p "$(dirname "$output_apk")" "$work_root/classes" "$work_root/dex"

"$javac" --release "$JAVA_RELEASE" -classpath "$platform_jar" -d "$work_root/classes" \
  "$fixture_root/src/org/emuchef/t03/copyfixture/FixtureActivity.java"
"$aapt2" link --auto-add-overlay --manifest "$fixture_root/AndroidManifest.xml" \
  -I "$platform_jar" -o "$work_root/unsigned.apk"
"$jar" cf "$work_root/classes.jar" -C "$work_root/classes" .
"$d8" --min-api 30 --output "$work_root/dex" "$work_root/classes.jar"
"$jar" uf "$work_root/unsigned.apk" -C "$work_root/dex" classes.dex
"$zipalign" -f 4 "$work_root/unsigned.apk" "$work_root/aligned.apk"
"$keytool" -genkeypair -noprompt -alias t03-fixture -keyalg RSA -keysize 2048 \
  -validity 1 -dname "CN=EmuChef T03 Test Fixture" \
  -keystore "$work_root/signing.jks" -storepass fixture-only-not-a-secret \
  -keypass fixture-only-not-a-secret >/dev/null 2>&1
"$apksigner" sign --ks "$work_root/signing.jks" --ks-key-alias t03-fixture \
  --ks-pass pass:fixture-only-not-a-secret --key-pass pass:fixture-only-not-a-secret \
  --out "$output_apk" "$work_root/aligned.apk"

printf 'Built synthetic T03 App Target APK: %s\n' "$output_apk"
