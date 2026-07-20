#!/usr/bin/env bash
set -euo pipefail

APK="${1:-app/build/outputs/apk/debug/app-debug.apk}"
BUILD_TOOLS="${ANDROID_HOME}/build-tools/35.0.0"
AAPT="${BUILD_TOOLS}/aapt"
APKSIGNER="${BUILD_TOOLS}/apksigner"
APKANALYZER="$(command -v apkanalyzer || true)"
if [[ -z "$APKANALYZER" ]]; then
  APKANALYZER="$(find "${ANDROID_HOME}/cmdline-tools" -type f -path '*/bin/apkanalyzer' -print -quit)"
fi

if [[ ! -f "$APK" ]]; then
  echo "APK not found: $APK" >&2
  exit 1
fi
if [[ -z "$APKANALYZER" || ! -x "$APKANALYZER" ]]; then
  echo "apkanalyzer not found" >&2
  exit 1
fi

# Generate immutable evidence before applying the policy gates.
"$AAPT" dump badging "$APK" > apk-badging.txt
"$AAPT" dump xmltree "$APK" AndroidManifest.xml > manifest-tree.txt
"$APKSIGNER" verify --verbose --print-certs "$APK" > signature.txt
"$APKANALYZER" dex list "$APK" > dex-files.txt
"$APKANALYZER" dex packages --defined-only "$APK" > dex-packages.txt
sha256sum "$APK" > app-debug.apk.sha256

mapfile -t permissions < <(
  "$AAPT" dump permissions "$APK" \
    | sed -n "s/uses-permission: name='\([^']*\)'.*/\1/p" \
    | sort -u
)

printf '%s\n' "${permissions[@]}" > permissions.txt
if [[ "${#permissions[@]}" -ne 1 || "${permissions[0]}" != "android.permission.INTERNET" ]]; then
  echo "Unexpected permission set:" >&2
  printf '  %s\n' "${permissions[@]}" >&2
  exit 1
fi

for forbidden in \
  'android.permission.WAKE_LOCK' \
  'android.permission.RECEIVE_BOOT_COMPLETED' \
  'android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS' \
  'android.permission.SCHEDULE_EXACT_ALARM' \
  'addJavascriptInterface' \
  'Runtime.getRuntime' \
  'ProcessBuilder' \
  'DexClassLoader' \
  '2captcha' \
  'writePost(' \
  'writeComment('; do
  if grep -R --line-number --fixed-strings "$forbidden" app/src/main; then
    echo "Forbidden surface found: $forbidden" >&2
    exit 1
  fi
done

if find app/src/main -type f -name '*.java' -print0 \
    | xargs -0 grep -n --fixed-strings 'http://'; then
  echo "Cleartext URL found in Java source" >&2
  exit 1
fi

if grep -R --line-number --fixed-strings 'android:allowBackup="true"' app/src/main; then
  echo "Android backup was unexpectedly enabled" >&2
  exit 1
fi

if unzip -l "$APK" | grep -E '(^|[[:space:]])lib/.*\.so([[:space:]]|$)'; then
  echo "Unexpected native library found" >&2
  exit 1
fi

DEX_COUNT=$(grep -Ec '^classes[0-9]*\.dex$' dex-files.txt || true)
if [[ "$DEX_COUNT" -lt 1 || "$DEX_COUNT" -gt 2 ]]; then
  echo "Unexpected DEX count: $DEX_COUNT" >&2
  cat dex-files.txt >&2
  exit 1
fi

mapfile -t unexpected_classes < <(
  awk '$1 == "C" && $2 == "d" {print $NF}' dex-packages.txt \
    | grep -Ev '^com\.tokengourmet\.dccleanersafe(\.|$)' \
    || true
)
if [[ "${#unexpected_classes[@]}" -ne 0 ]]; then
  echo "Unexpected classes defined in APK:" >&2
  printf '  %s\n' "${unexpected_classes[@]}" >&2
  exit 1
fi

for required_class in \
  'com.tokengourmet.dccleanersafe.MainActivity' \
  'com.tokengourmet.dccleanersafe.DcClient' \
  'com.tokengourmet.dccleanersafe.SafeUrlPolicy'; do
  if ! grep -Eq "^C d .* ${required_class//./\\.}$" dex-packages.txt; then
    echo "Required class missing from APK: $required_class" >&2
    exit 1
  fi
done

echo "Security checks passed: INTERNET-only permission, package-defined DEX allowlist, no native libraries or forbidden runtime surfaces."
