import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import {
  IMPORTED_CODE_FIXTURE_ASSETS,
  STORE_SAFE_PUBLIC_ASSETS,
  admittedPublicAssetNames,
  resolveBuildProfile,
  storeSafeIsolationRouteSource,
} from "../../scripts/build-profile.mjs";
import { verifyBuiltProfile } from "../../scripts/verify-built-profile.mjs";

/** @type {string[]} */
const temporaryDirectories = [];

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function temporaryBuild() {
  const directory = mkdtempSync(join(tmpdir(), "lorepia-profile-"));
  temporaryDirectories.push(directory);
  return directory;
}

/**
 * @param {string} directory
 * @param {readonly string[]} assets
 */
function writeAssets(directory, assets) {
  for (const asset of assets) {
    const path = join(directory, asset);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, asset);
  }
}

describe("compile-target imported-code profile", () => {
  it.each(["android", "ANDROID", "ios", " iOS "])(
    "fails imported JavaScript and Lua closed for %s",
    (platform) => {
      const profile = resolveBuildProfile(platform);

      expect(profile).toMatchObject({
        id: "store-safe-mobile",
        storeSafeMobile: true,
        importedJavaScriptFixtureAllowed: false,
        importedLuaFixtureAllowed: false,
      });
    },
  );

  it.each([undefined, "macos", "windows", "linux"])(
    "retains the desktop research fixture for %s",
    (platform) => {
      const profile = resolveBuildProfile(platform);

      expect(profile).toMatchObject({
        id: "desktop-local-research",
        storeSafeMobile: false,
        importedJavaScriptFixtureAllowed: true,
        importedLuaFixtureAllowed: true,
      });
    },
  );

  it.each(["andriod", "ipados", "freebsd"])(
    "rejects unknown compile target %s instead of enabling research fixtures",
    (platform) => {
      expect(() => resolveBuildProfile(platform)).toThrow(
        /unsupported TAURI_ENV_PLATFORM/,
      );
    },
  );

  it("treats desktop Lua as fixture policy, not an implemented runtime claim", () => {
    const profile = resolveBuildProfile("macos");

    expect(profile.importedLuaFixtureAllowed).toBe(true);
    expect(profile).not.toHaveProperty("luaRuntimeImplemented");
  });

  it("admits only non-executable allowlisted public assets to mobile output", () => {
    const admitted = admittedPublicAssetNames(resolveBuildProfile("android"));

    expect(admitted).toEqual(STORE_SAFE_PUBLIC_ASSETS);
    expect(
      admitted?.some((asset) => IMPORTED_CODE_FIXTURE_ASSETS.includes(asset)),
    ).toBe(false);
  });

  it("leaves Vite's complete public directory enabled for desktop research builds", () => {
    expect(admittedPublicAssetNames(resolveBuildProfile("macos"))).toBeNull();
  });

  it.each(["android", "ios"])(
    "replaces /isolation with a non-executable status route for %s",
    (platform) => {
      const source = storeSafeIsolationRouteSource(resolveBuildProfile(platform));

      expect(source).toContain("imported JavaScript OFF, imported Lua OFF");
      expect(source).not.toMatch(/<iframe|plugin-frame|@tauri-apps|\binvoke\b/);
    },
  );

  it("retains the complete isolation harness source on desktop", () => {
    expect(
      storeSafeIsolationRouteSource(resolveBuildProfile("windows")),
    ).toBeNull();
  });
});

describe("built asset profile verification", () => {
  it.each(["android", "ios"])(
    "accepts %s output only when executable fixtures are absent",
    (platform) => {
      const directory = temporaryBuild();
      writeAssets(directory, ["index.html", ...STORE_SAFE_PUBLIC_ASSETS]);

      const result = verifyBuiltProfile(directory, resolveBuildProfile(platform));
      expect(result.includedFixtures).toEqual([]);

      writeAssets(directory, ["plugin-frame.js"]);
      expect(() =>
        verifyBuiltProfile(directory, resolveBuildProfile(platform)),
      ).toThrow(/contains imported-code fixtures/);
    },
  );

  it("rejects a mobile bundle that still references the plugin runtime", () => {
    const directory = temporaryBuild();
    writeAssets(directory, ["index.html", ...STORE_SAFE_PUBLIC_ASSETS]);
    writeAssets(directory, ["_app/isolation.js"]);
    writeFileSync(
      join(directory, "_app/isolation.js"),
      'const fixture = "/plugin-frame.html";',
    );

    expect(() =>
      verifyBuiltProfile(directory, resolveBuildProfile("android")),
    ).toThrow(/contains plugin runtime marker/);
  });

  it("requires both isolation fixture assets in desktop research output", () => {
    const directory = temporaryBuild();
    writeAssets(directory, ["index.html", ...IMPORTED_CODE_FIXTURE_ASSETS]);

    expect(
      verifyBuiltProfile(directory, resolveBuildProfile("linux"))
        .includedFixtures,
    ).toEqual(IMPORTED_CODE_FIXTURE_ASSETS);
  });
});
