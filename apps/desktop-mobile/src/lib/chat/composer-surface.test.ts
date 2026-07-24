import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("./Composer.svelte", import.meta.url),
  "utf8",
);

describe("chat composer surface", () => {
  it("keeps Add inside the single input capsule", () => {
    expect(source).toMatch(
      /<div class="field">\s*<!--[\s\S]*?<button class="extra"/u,
    );
    expect(source).not.toMatch(
      /<div class="composer-row">\s*<button class="extra"/u,
    );
    expect(source).toMatch(
      /\.extra\s*\{[\s\S]*position:\s*absolute;[\s\S]*width:\s*var\(--size-touch\);[\s\S]*height:\s*var\(--size-touch\);[\s\S]*border:\s*0;[\s\S]*background:\s*transparent;/u,
    );
  });

  it("preserves disabled attachment semantics and 44px send hit geometry", () => {
    expect(source).toContain(
      '<button class="extra" type="button" disabled aria-label="첨부 (준비 중)">',
    );
    expect(source).toMatch(
      /\.send\s*\{[\s\S]*width:\s*var\(--size-touch\);[\s\S]*height:\s*var\(--size-touch\);/u,
    );
    expect(source).toMatch(
      /\.send::before\s*\{[\s\S]*inset:\s*5px;[\s\S]*background:\s*var\(--tint\);/u,
    );
  });
});
