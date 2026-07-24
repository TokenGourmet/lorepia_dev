import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("./edge-back.ts", import.meta.url),
  "utf8",
);
const surfaceSource = readFileSync(
  new URL("./back-swipe-surface.ts", import.meta.url),
  "utf8",
);
const routeSources = {
  character: readFileSync(
    new URL("../../routes/character/[id]/+page.svelte", import.meta.url),
    "utf8",
  ),
  import: readFileSync(
    new URL("../../routes/import/+page.svelte", import.meta.url),
    "utf8",
  ),
  community: readFileSync(
    new URL("../../routes/community/+page.svelte", import.meta.url),
    "utf8",
  ),
  info: readFileSync(
    new URL("../../routes/chat/info/+page.svelte", import.meta.url),
    "utf8",
  ),
  report: readFileSync(
    new URL("../../routes/chat/report/+page.svelte", import.meta.url),
    "utf8",
  ),
};

describe("edge back transition", () => {
  it("delegates to the shared engine and reveals the latest nested chat surface", () => {
    expect(source).toContain("contentSwipeBack(");
    expect(source).toContain("edgeWidth: EDGE_START");
    expect(source).toContain(
      "options.getUnderlay ?? activateLatestBackSwipeSurface",
    );
  });

  it("selects the captured surface for the actual route parent", () => {
    expect(surfaceSource).toContain("capturedSurfaceForHref(expectedHref)");
    expect(routeSources.character).toContain(
      'activateBackSwipeSurface("/")',
    );
    expect(routeSources.import).toContain(
      'activateBackSwipeSurface("/home")',
    );
    expect(routeSources.community).toContain(
      'activateBackSwipeSurface("/home")',
    );
    expect(routeSources.info).toContain(
      "activateBackSwipeSurface(chatHref)",
    );
    expect(routeSources.report).toContain(
      "activateBackSwipeSurface(infoHref)",
    );
    for (const name of ["character", "import", "community"] as const) {
      expect(routeSources[name]).toContain("onBack: navigateBack");
      expect(routeSources[name]).toContain("replaceState: true");
      expect(routeSources[name]).toContain("onclick={navigateBack}");
    }
  });
});
