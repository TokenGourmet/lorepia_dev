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
});
