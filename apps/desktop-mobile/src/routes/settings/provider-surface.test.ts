import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("./+page.svelte", import.meta.url),
  "utf8",
);

describe("provider settings surface", () => {
  it("renders the product-owned provider catalog", () => {
    expect(source).toContain("LLM_PROVIDER_CATALOG");
    expect(source).toContain("LLM 제공자 선택");
    expect(source).toContain("selectedProvider.setupFields");
    expect(source).toContain('type="radio"');
    expect(source).not.toContain("aria-pressed");
  });

  it("does not collect credentials before the UI is wired to the native vault", () => {
    expect(source).not.toMatch(/type=["']password["']/i);
    expect(source).not.toMatch(/localStorage|sessionStorage|document\.cookie/i);
    expect(source).not.toMatch(/\bfetch\s*\(|\binvoke\s*\(/i);
    expect(source).toMatch(
      /API 키·토큰·서비스\s+계정 파일은 입력하거나 수집하지 않습니다/,
    );
    expect(source).toContain("<button class=\"connect\" type=\"button\" disabled>");
  });
});
