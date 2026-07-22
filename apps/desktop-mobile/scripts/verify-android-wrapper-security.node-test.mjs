// Keep this outside Vitest's *.test.* glob; these checks use Node's test runner.
import assert from "node:assert/strict";
import { resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  verifyAndroidWrapperSecurity,
  verifyFileProviderPaths,
  verifyManifest,
} from "./verify-android-wrapper-security.mjs";

const safePaths = `
  <paths xmlns:android="http://schemas.android.com/apk/res/android">
    <files-path name="imports" path="import/" />
    <cache-path name="shares" path="share/" />
  </paths>
`;

const phoneManifest = `
  <manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application android:allowBackup="false">
      <activity android:name=".MainActivity" android:exported="true">
        <intent-filter>
          <action android:name="android.intent.action.MAIN" />
          <category android:name="android.intent.category.LAUNCHER" />
        </intent-filter>
      </activity>
    </application>
  </manifest>
`;

test("the committed product Android wrapper keeps the release boundary", () => {
  const scriptDirectory = resolve(fileURLToPath(new URL(".", import.meta.url)));
  const wrapperRoot = resolve(scriptDirectory, "../src-tauri/gen/android");
  assert.doesNotThrow(() => verifyAndroidWrapperSecurity(wrapperRoot));
});

test("accepts only the two intended app-owned subdirectories", () => {
  assert.deepEqual(verifyFileProviderPaths(safePaths).paths, [
    { element: "files-path", name: "imports", path: "import/" },
    { element: "cache-path", name: "shares", path: "share/" },
  ]);
});

test("rejects Tauri's broad external-storage template mapping", () => {
  const unsafe = safePaths.replace(
    '<files-path name="imports" path="import/" />',
    '<external-path name="my_images" path="." />',
  );
  assert.throws(() => verifyFileProviderPaths(unsafe), /unauthorized path/);
});

test("rejects a cache-root mapping even though cache is app-owned", () => {
  const unsafe = safePaths.replace('path="share/"', 'path="."');
  assert.throws(() => verifyFileProviderPaths(unsafe), /unauthorized path/);
});

test("rejects additional path mappings", () => {
  const unsafe = safePaths.replace(
    "</paths>",
    '<files-path name="extra" path="extra/" />\n</paths>',
  );
  assert.throws(
    () => verifyFileProviderPaths(unsafe),
    /exactly 2 purpose-specific paths/,
  );
});

test("rejects a paired-tag path mapping that tries to evade the policy", () => {
  const unsafe = safePaths.replace(
    "</paths>",
    '<root-path name="root" path="."></root-path>\n</paths>',
  );
  assert.throws(
    () => verifyFileProviderPaths(unsafe),
    /exactly 2 purpose-specific paths/,
  );
});

test("keeps the normal phone and tablet launcher", () => {
  assert.deepEqual(verifyManifest(phoneManifest), {
    phoneLauncher: true,
    tvLauncher: false,
    backupDisabled: true,
  });
});

test("rejects OS backup that contradicts the local-only product disclosure", () => {
  const unsafe = phoneManifest.replace(
    'android:allowBackup="false"',
    'android:allowBackup="true"',
  );
  assert.throws(() => verifyManifest(unsafe), /backup.*must be disabled/i);
});

test("rejects stale backup-rule attributes even when backup is disabled", () => {
  const unsafe = phoneManifest.replace(
    'android:allowBackup="false"',
    'android:allowBackup="false" android:fullBackupContent="@xml/backup_rules"',
  );
  assert.throws(() => verifyManifest(unsafe), /backup rule attributes/);
});

test("rejects an unintended Android TV launcher", () => {
  const unsafe = phoneManifest.replace(
    '<category android:name="android.intent.category.LAUNCHER" />',
    '<category android:name="android.intent.category.LAUNCHER" />\n' +
      '<category android:name="android.intent.category.LEANBACK_LAUNCHER" />',
  );
  assert.throws(() => verifyManifest(unsafe), /LEANBACK_LAUNCHER/);
});

test("rejects the Android TV feature even without its launcher category", () => {
  const unsafe = phoneManifest.replace(
    '<application android:allowBackup="false">',
    '<uses-feature android:name="android.software.leanback" android:required="false" />\n' +
      '<application android:allowBackup="false">',
  );
  assert.notEqual(unsafe, phoneManifest, "fixture mutation must take effect");
  assert.throws(() => verifyManifest(unsafe), /leanback feature/);
});

test("rejects removal of the normal phone and tablet launcher", () => {
  const unsafe = phoneManifest.replace(
    '<category android:name="android.intent.category.LAUNCHER" />',
    "",
  );
  assert.throws(() => verifyManifest(unsafe), /MAIN LAUNCHER/);
});
