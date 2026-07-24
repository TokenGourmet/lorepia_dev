import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("./+layout.svelte", import.meta.url),
  "utf8",
);
describe("root layout interaction surfaces", () => {
  it("suppresses internal dock previews only in native mobile wrappers", () => {
    expect(source).toContain(
      'platform === "ios" || platform === "android"',
    );
    expect(source).toContain(
      "oncontextmenu={suppressNativeAppLinkPreview}",
    );
    expect(source).toContain(
      "ondragstart={suppressNativeAppLinkPreview}",
    );
    expect(source).not.toContain("draggable={false}");
  });

  it("reveals a non-raster DOM clone with the shared curved progress contract", () => {
    expect(source).toContain("@property --back-transition-progress");
    expect(source).toContain("--back-transition-underlay-scale");
    expect(source).toContain("--back-transition-shadow-x");
    expect(source).toContain(
      "transform-origin: var(--back-transition-origin-x) center",
    );
    expect(source).toMatch(
      /\.back-swipe-underlay::after\s*\{[\s\S]*z-index:\s*2;/u,
    );
    expect(source).toContain("detailStackDepth(to.url.pathname)");
    expect(source).toContain("detailStackDepth(from.url.pathname)");
    expect(source).toContain("isDetailPath(to.url.pathname)");
    expect(source).toContain("completeBackSwipeSurface(localHref(to.url))");
    expect(source).toMatch(
      /\.shell\[data-back-transition-state\]\)\s*\{[\s\S]*translate:\s*var\(--back-transition-x\) 0;[\s\S]*border-radius:\s*var\(--back-transition-radius\);[\s\S]*overflow:\s*clip;[\s\S]*box-shadow:/u,
    );
    expect(source).toMatch(
      /\.back-swipe-underlay\[data-back-transition-state\]\)\s*\{[\s\S]*translate:\s*var\(--back-transition-underlay-x\) 0;[\s\S]*scale:\s*var\(--back-transition-underlay-scale\);/u,
    );
    expect(source).toContain("{@render children()}");
  });

  it("owns Android native back once for every detail route", () => {
    expect(source).toContain("connectNativeBackCommit(handleAndroidNativeBack)");
    expect(source).toContain(
      "isAndroidNativeBackRoute(to.url.pathname)",
    );
    expect(source).toContain(
      "planAndroidNativeBack(\n      page.url,\n      page.state,\n      window.history.length,",
    );
    expect(source).toContain(
      'document.querySelector<HTMLDialogElement>("dialog[open]")',
    );
    expect(source).toContain(
      'new Event("cancel", { cancelable: true })',
    );
  });

  it("hands the compact mobile four-tab shell to native chrome after a successful status", () => {
    expect(source).toContain(
      'window.matchMedia("(max-width: 699px)")',
    );
    expect(source).toContain(
      'platform === "ios" || platform === "android"',
    );
    expect(source).toContain(
      "status.active && status.compact",
    );
    expect(source).toContain(
      "createNativeChromeStateSync(",
    );
    expect(source).toContain(
      "connectNativeTabSelection(",
    );
    expect(source).toContain(
      "createNativeTabNavigationSync(",
    );
    expect(source).toContain(
      "nativeTabNavigationSync?.request(href)",
    );
    expect(source).toContain(
      "visible: nativeChromeCompact && !isDetailScreen",
    );
    expect(source).toContain(
      "appearance: appPreferences.current.theme",
    );
    expect(source).toContain(
      "class:native-chrome-active={nativeChromeActive}",
    );
    expect(source).toMatch(
      /\.shell\.native-chrome-active:not\(\.detail\)\s*>\s*\.dockrow\s*\{\s*display:\s*none;/u,
    );
    expect(source).toContain(
      "[data-back-swipe-captured-surface].native-chrome-active",
    );
    expect(source).toMatch(
      /> \.dockrow \.indicator\s*\)\s*\{\s*background:\s*transparent;/u,
    );
    expect(source).toContain("completeNativeBackAfterPaint()");
    expect(source).toMatch(
      /requestAnimationFrame\(\(\) => \{\s*nativeBackCompletionFrame = requestAnimationFrame/u,
    );
    expect(source).toMatch(
      /data-native-platform="ios"[\s\S]+shell\.native-chrome-active:not\(\.detail\)[\s\S]+env\(safe-area-inset-bottom,\s*0px\)\s*\+\s*49px/u,
    );
  });
});
