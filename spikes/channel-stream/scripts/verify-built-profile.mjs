import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  IMPORTED_CODE_FIXTURE_ASSETS,
  STORE_SAFE_PUBLIC_ASSETS,
  resolveBuildProfile,
} from "./build-profile.mjs";

const MOBILE_FORBIDDEN_RUNTIME_MARKERS = Object.freeze([
  "/plugin-frame.html",
  "lorepia:plugin:run-suite",
  "악성 플러그인 격리 시험 프레임",
]);

/**
 * @param {string} root
 * @param {string} [directory]
 * @returns {string[]}
 */
function listFiles(root, directory = root) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const absolute = resolve(directory, entry.name);
    if (entry.isDirectory()) return listFiles(root, absolute);
    return [absolute.slice(root.length + 1)];
  });
}

/**
 * @param {string} outputDirectory
 * @param {ReturnType<typeof resolveBuildProfile>} profile
 */
export function verifyBuiltProfile(outputDirectory, profile) {
  const root = resolve(outputDirectory);
  const files = listFiles(root);
  const fileSet = new Set(files);
  const includedFixtures = IMPORTED_CODE_FIXTURE_ASSETS.filter((fileName) =>
    fileSet.has(fileName),
  );

  if (profile.storeSafeMobile) {
    if (includedFixtures.length > 0) {
      throw new Error(
        `Store-Safe output contains imported-code fixtures: ${includedFixtures.join(", ")}`,
      );
    }
    const missingSafeAssets = STORE_SAFE_PUBLIC_ASSETS.filter(
      (fileName) => !fileSet.has(fileName),
    );
    if (missingSafeAssets.length > 0) {
      throw new Error(
        `Store-Safe output is missing allowlisted public assets: ${missingSafeAssets.join(", ")}`,
      );
    }
    for (const file of files.filter((name) => /\.(?:html|js|json)$/.test(name))) {
      const contents = readFileSync(resolve(root, file), "utf8");
      const marker = MOBILE_FORBIDDEN_RUNTIME_MARKERS.find((value) =>
        contents.includes(value),
      );
      if (marker !== undefined) {
        throw new Error(
          `Store-Safe output contains plugin runtime marker ${JSON.stringify(marker)} in ${file}`,
        );
      }
    }
  } else {
    const missingFixtures = IMPORTED_CODE_FIXTURE_ASSETS.filter(
      (fileName) => !fileSet.has(fileName),
    );
    if (missingFixtures.length > 0) {
      throw new Error(
        `Desktop research output is missing isolation fixtures: ${missingFixtures.join(", ")}`,
      );
    }
  }

  return Object.freeze({ files, includedFixtures });
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  const profile = resolveBuildProfile(process.env.TAURI_ENV_PLATFORM);
  const result = verifyBuiltProfile(
    fileURLToPath(new URL("../build/", import.meta.url)),
    profile,
  );
  process.stdout.write(
    `${profile.id} (${profile.targetPlatform}) verified: ${result.files.length} built files\n`,
  );
}
