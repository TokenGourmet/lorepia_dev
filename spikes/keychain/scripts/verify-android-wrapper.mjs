import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const source = (relativePath) =>
  readFileSync(new URL(`../${relativePath}`, import.meta.url), "utf8");
const withoutCodeComments = (contents) =>
  contents.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\r\n]*/g, "");
const withoutXmlComments = (contents) =>
  contents.replace(/<!--[\s\S]*?-->/g, "");

function requirePattern(contents, pattern, description) {
  if (!pattern.test(contents)) {
    throw new Error(`Android keychain wrapper is missing ${description}`);
  }
}

const activity = withoutCodeComments(
  source(
    "src-tauri/gen/android/app/src/main/java/dev/lorepia/spike/keychain/MainActivity.kt",
  ),
);
requirePattern(
  activity,
  /^package dev\.lorepia\.spike\.keychain$/m,
  "the exact Kotlin package",
);
requirePattern(
  activity,
  /private external fun initNdkContext\(context: Context\)/,
  "the native NDK-context declaration",
);
const superOnCreate = activity.indexOf("super.onCreate(savedInstanceState)");
const initializeContext = activity.indexOf("initNdkContext(applicationContext)");
if (superOnCreate < 0 || initializeContext <= superOnCreate) {
  throw new Error(
    "Android keychain wrapper must initialize the NDK context after super.onCreate",
  );
}

const rustLibrary = withoutCodeComments(source("src-tauri/src/lib.rs"));
requirePattern(
  rustLibrary,
  /Java_dev_lorepia_spike_keychain_MainActivity_initNdkContext/,
  "the matching Rust JNI symbol",
);
requirePattern(
  rustLibrary,
  /ndk_context::initialize_android_context\s*\(/,
  "the Rust NDK-context initialization call",
);

const backend = withoutCodeComments(source("src-tauri/src/backend.rs"));
requirePattern(
  backend,
  /\("filename",\s*"lorepia-keyring-v1"\)/,
  "the SharedPreferences filename matched by the backup exclusions",
);

const manifest = withoutXmlComments(
  source("src-tauri/gen/android/app/src/main/AndroidManifest.xml"),
);
requirePattern(
  manifest,
  /android:fullBackupContent="@xml\/backup_rules"/,
  "the legacy cloud-backup exclusion reference",
);
requirePattern(
  manifest,
  /android:dataExtractionRules="@xml\/data_extraction_rules"/,
  "the Android 12+ backup and transfer exclusion reference",
);

const backupRules = withoutXmlComments(
  source("src-tauri/gen/android/app/src/main/res/xml/backup_rules.xml"),
);
if (/<include\b/.test(backupRules)) {
  throw new Error("Android keychain backup rules must not include credential data");
}
requirePattern(
  backupRules,
  /<full-backup-content>[\s\S]*<exclude\s+domain="sharedpref"\s+path="lorepia-keyring-v1\.xml"\s*\/>[\s\S]*<\/full-backup-content>/,
  "the SharedPreferences cloud-backup exclusion",
);

const extractionRules = withoutXmlComments(
  source("src-tauri/gen/android/app/src/main/res/xml/data_extraction_rules.xml"),
);
if (/<include\b/.test(extractionRules)) {
  throw new Error("Android keychain extraction rules must not include credential data");
}
const sharedPreferenceExclusion =
  /<exclude\s+domain="sharedpref"\s+path="lorepia-keyring-v1\.xml"\s*\/>/g;
const exclusions = extractionRules.match(sharedPreferenceExclusion) ?? [];
if (exclusions.length !== 2) {
  throw new Error(
    "Android keychain extraction rules need one cloud-backup and one device-transfer exclusion",
  );
}
requirePattern(
  extractionRules,
  /<cloud-backup>[\s\S]*<exclude\s+domain="sharedpref"\s+path="lorepia-keyring-v1\.xml"\s*\/>[\s\S]*<\/cloud-backup>/,
  "the Android 12+ cloud-backup exclusion",
);
requirePattern(
  extractionRules,
  /<device-transfer>[\s\S]*<exclude\s+domain="sharedpref"\s+path="lorepia-keyring-v1\.xml"\s*\/>[\s\S]*<\/device-transfer>/,
  "the device-transfer exclusion",
);

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.stdout.write("Android keychain wrapper and backup exclusions verified\n");
}
