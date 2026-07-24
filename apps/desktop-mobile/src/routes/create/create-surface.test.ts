import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./+page.svelte", import.meta.url), "utf8");

describe("character creation availability surface", () => {
  it("states the real storage boundary instead of rendering fake controls", () => {
    expect(source).toContain("기능 준비 중");
    expect(source).toContain("실제 기기 저장소");
    expect(source).toContain("입력 내용이 저장되지 않는 임시 생성 화면");
    expect(source).not.toMatch(/<(?:button|form|input|select|textarea)\b/i);
  });
});
