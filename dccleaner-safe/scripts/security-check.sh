#!/usr/bin/env bash
set -euo pipefail

APK="${1:-app/build/outputs/apk/debug/app-debug.apk}"
BUILD_TOOLS="${ANDROID_HOME}/build-tools/35.0.0"
AAPT="${BUILD_TOOLS}/aapt"
APKSIGNER="${BUILD_TOOLS}/apksigner"

if [[ ! -f "$APK" ]]; then
  echo "APK not found: $APK" >&2
  exit 1
fi

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

DEX_COUNT=$(unzip -l "$APK" | grep -Ec '(^|[[:space:]])classes[0-9]*\.dex([[:space:]]|$)' || true)
if [[ "$DEX_COUNT" -ne 1 ]]; then
  echo "Unexpected DEX count: $DEX_COUNT" >&2
  exit 1
fi

"$AAPT" dump badging "$APK" > apk-badging.txt
"$AAPT" dump xmltree "$APK" AndroidManifest.xml > manifest-tree.txt
"$APKSIGNER" verify --verbose --print-certs "$APK" > signature.txt
sha256sum "$APK" > app-debug.apk.sha256

echo "Security checks passed: INTERNET-only permission, one DEX, no native libraries or forbidden runtime surfaces."
