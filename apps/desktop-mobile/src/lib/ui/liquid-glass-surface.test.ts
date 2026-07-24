import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("./LiquidGlass.svelte", import.meta.url),
  "utf8",
);

describe("liquid glass surface", () => {
  it("keeps runtime visuals compatible with the strict style CSP", () => {
    expect(source).not.toContain("style:");
    expect(source).not.toMatch(/\sstyle=/);
    expect(source).toContain("<canvas");
    expect(source).toContain("backdrop-filter");
  });

  it("tracks pointer light and press ripples without intercepting content", () => {
    expect(source).toContain("onpointermove={handlePointerMove}");
    expect(source).toContain("liquidRippleFrame");
    expect(source).toContain("pointer-events: none");
    expect(source).toContain("window.addEventListener(\"pointerup\"");
  });

  it("honors reduced motion and a no-backdrop fallback", () => {
    expect(source).toContain("prefers-reduced-motion: reduce");
    expect(source).toContain("prefers-contrast: more");
    expect(source).toContain("glass-fallback-fill");
  });
});
