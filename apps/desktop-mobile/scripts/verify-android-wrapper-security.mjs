import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ALLOWED_FILE_PROVIDER_PATHS = new Set([
  "files-path\0imports\0import/",
  "cache-path\0shares\0share/",
]);

const stripXmlComments = (contents) =>
  contents.replace(/<!--[\s\S]*?-->/g, "");

function attributesOf(source) {
  return new Map(
    [...source.matchAll(/([\w:-]+)\s*=\s*"([^"]*)"/g)].map(
      ([, name, value]) => [name, value],
    ),
  );
}

export function verifyFileProviderPaths(contents) {
  const xml = stripXmlComments(contents);
  const paths = xml.match(/<paths\b[^>]*>([\s\S]*?)<\/paths>/);
  if (!paths) {
    throw new Error("Android FileProvider path policy is missing a <paths> root");
  }

  const entries = [...paths[1].matchAll(/<([a-z][a-z0-9-]*)\b([^>]*)\/?>/g)].map(
    ([, element, rawAttributes]) => {
      const attributes = attributesOf(rawAttributes);
      return {
        element,
        name: attributes.get("name") ?? "",
        path: attributes.get("path") ?? "",
      };
    },
  );

  if (entries.length !== ALLOWED_FILE_PROVIDER_PATHS.size) {
    throw new Error(
      `Android FileProvider must expose exactly ${ALLOWED_FILE_PROVIDER_PATHS.size} purpose-specific paths`,
    );
  }

  const seen = new Set();
  for (const entry of entries) {
    const key = `${entry.element}\0${entry.name}\0${entry.path}`;
    if (!ALLOWED_FILE_PROVIDER_PATHS.has(key)) {
      throw new Error(
        `Android FileProvider exposes an unauthorized path: ${entry.element} name=${JSON.stringify(entry.name)} path=${JSON.stringify(entry.path)}`,
      );
    }
    if (seen.has(key)) {
      throw new Error("Android FileProvider contains a duplicate path mapping");
    }
    seen.add(key);
  }

  for (const required of ALLOWED_FILE_PROVIDER_PATHS) {
    if (!seen.has(required)) {
      throw new Error("Android FileProvider is missing a required scoped path");
    }
  }

  return { paths: entries };
}

export function verifyManifest(contents) {
  const xml = stripXmlComments(contents);
  if (/android\.intent\.category\.LEANBACK_LAUNCHER/.test(xml)) {
    throw new Error("Android TV LEANBACK_LAUNCHER must not be declared");
  }
  if (/android\.software\.leanback/.test(xml)) {
    throw new Error("Android TV leanback feature must not be declared");
  }

  const application = xml.match(/<application\b([^>]*)>/);
  if (!application) {
    throw new Error("Android application declaration is missing");
  }
  const applicationAttributes = attributesOf(application[1]);
  if (applicationAttributes.get("android:allowBackup") !== "false") {
    throw new Error(
      "Android OS cloud backup and device-transfer backup must be disabled",
    );
  }
  if (
    applicationAttributes.has("android:fullBackupContent") ||
    applicationAttributes.has("android:dataExtractionRules")
  ) {
    throw new Error(
      "disabled Android backup must not retain misleading backup rule attributes",
    );
  }

  const launchFilter = [
    ...xml.matchAll(/<intent-filter\b[^>]*>([\s\S]*?)<\/intent-filter>/g),
  ]
    .map((match) => match[1])
    .find(
      (body) =>
        /android:name="android\.intent\.action\.MAIN"/.test(body) &&
        /android:name="android\.intent\.category\.LAUNCHER"/.test(body),
    );
  if (!launchFilter) {
    throw new Error("Android phone/tablet MAIN LAUNCHER intent filter is missing");
  }

  return { phoneLauncher: true, tvLauncher: false, backupDisabled: true };
}

export function verifyAndroidWrapperSecurity(wrapperRoot) {
  const manifestPath = resolve(
    wrapperRoot,
    "app/src/main/AndroidManifest.xml",
  );
  const filePathsPath = resolve(
    wrapperRoot,
    "app/src/main/res/xml/file_paths.xml",
  );

  const manifest = verifyManifest(readFileSync(manifestPath, "utf8"));
  const fileProvider = verifyFileProviderPaths(readFileSync(filePathsPath, "utf8"));
  return { manifest, fileProvider };
}

const invokedPath = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : "";
if (import.meta.url === invokedPath) {
  const scriptDirectory = resolve(fileURLToPath(new URL(".", import.meta.url)));
  const wrapperRoot = resolve(scriptDirectory, "../src-tauri/gen/android");
  const result = verifyAndroidWrapperSecurity(wrapperRoot);
  process.stdout.write(
    `verified Android wrapper: ${result.fileProvider.paths.length} scoped FileProvider paths, phone/tablet launcher only, OS backup disabled\n`,
  );
}
