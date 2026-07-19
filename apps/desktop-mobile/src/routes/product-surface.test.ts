import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const pageSource = readFileSync(
  fileURLToPath(new URL("./+page.svelte", import.meta.url)),
  "utf8",
);

// This is a temporary M0 ownership guard. Update it when the owner-authored
// visual design slice begins; it is not a permanent ban on product CSS.
describe("owner-authored product surface boundary", () => {
  it("contains no visual styling or animation", () => {
    expect(pageSource).not.toContain("<style");
    expect(pageSource).not.toMatch(/transition:|animate:|use:/);
  });

  it("does not enable unreviewed executable or media surfaces", () => {
    expect(pageSource).not.toMatch(/<iframe|<audio|{@html/i);
  });
});
