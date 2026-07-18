import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const IMPORTED_CODE_FIXTURE_ASSETS = Object.freeze([
  "plugin-frame.html",
  "plugin-frame.js",
]);

export const STORE_SAFE_PUBLIC_ASSETS = Object.freeze([
  "favicon.png",
  "svelte.svg",
  "tauri.svg",
  "vite.svg",
]);

const STORE_SAFE_MOBILE_PLATFORMS = new Set(["android", "ios"]);
const ISOLATION_ROUTE_PATH = fileURLToPath(
  new URL("../src/routes/isolation/+page.svelte", import.meta.url),
);

/**
 * The execution policy is derived only from Tauri's compile target. Keeping the
 * resolver pure makes it difficult for runtime state (manifest fields, query
 * parameters, storage, or a stale preference) to re-enable imported code.
 *
 * @param {string | undefined} tauriPlatform
 */
export function resolveBuildProfile(tauriPlatform) {
  const targetPlatform =
    typeof tauriPlatform === "string" && tauriPlatform.trim().length > 0
      ? tauriPlatform.trim().toLowerCase()
      : "desktop";
  const storeSafeMobile = STORE_SAFE_MOBILE_PLATFORMS.has(targetPlatform);

  return Object.freeze({
    id: storeSafeMobile ? "store-safe-mobile" : "desktop-local-research",
    targetPlatform,
    storeSafeMobile,
    // These are fixture packaging/execution policy flags, not claims that this
    // spike implements a general imported-JS or Lua runtime.
    importedJavaScriptFixtureAllowed: !storeSafeMobile,
    importedLuaFixtureAllowed: !storeSafeMobile,
  });
}

/** @param {ReturnType<typeof resolveBuildProfile>} profile */
export function admittedPublicAssetNames(profile) {
  return profile.storeSafeMobile ? STORE_SAFE_PUBLIC_ASSETS : null;
}

/** @param {ReturnType<typeof resolveBuildProfile>} profile */
export function storeSafeIsolationRouteSource(profile) {
  if (!profile.storeSafeMobile) return null;

  return `<svelte:head>
  <title>LorePia Store-Safe imported-code status</title>
  <meta name="description" content="Imported JavaScript and Lua are disabled by the mobile build target" />
</svelte:head>

<main>
  <nav><a href="/">Channel 실증으로 돌아가기</a></nav>
  <h1>Store-Safe mobile profile</h1>
  <p role="status" data-testid="store-safe-imported-code-status">
    ${profile.targetPlatform} build: imported JavaScript OFF, imported Lua OFF. 실행 fixture와
    plugin iframe은 이 빌드에 포함되지 않습니다.
  </p>
</main>
`;
}

/**
 * Mobile builds use an explicit safe-asset allowlist instead of Vite's public
 * directory copy. The executable plugin fixture is therefore never admitted
 * to the Rollup output in the first place.
 *
 * @param {ReturnType<typeof resolveBuildProfile>} profile
 * @returns {import("vite").Plugin}
 */
export function storeSafeAssetGate(profile) {
  return {
    name: "lorepia-store-safe-imported-code-gate",
    enforce: "pre",
    config() {
      return profile.storeSafeMobile ? { publicDir: false } : {};
    },
    buildStart() {
      const admittedAssets = admittedPublicAssetNames(profile);
      if (admittedAssets === null) return;

      for (const fileName of admittedAssets) {
        const source = readFileSync(
          new URL(`../static/${fileName}`, import.meta.url),
        );
        this.emitFile({ type: "asset", fileName, source });
      }
    },
    transform(_code, id) {
      const routeSource = storeSafeIsolationRouteSource(profile);
      if (
        routeSource === null ||
        id.split("?", 1)[0] !== ISOLATION_ROUTE_PATH
      ) {
        return null;
      }
      return { code: routeSource, map: null };
    },
  };
}
