import { describe, expect, it } from "vitest";

import { computeKeyboardInset } from "./keyboard-inset.svelte";

describe("computeKeyboardInset", () => {
  it("measures the keyboard as the missing visual viewport height", () => {
    expect(computeKeyboardInset(844, 508, 0)).toBe(336);
  });

  it("subtracts the visual viewport top offset when the page is pushed", () => {
    expect(computeKeyboardInset(844, 500, 20)).toBe(324);
  });

  it("clamps to zero when the keyboard is closed", () => {
    expect(computeKeyboardInset(844, 844, 0)).toBe(0);
  });

  it("clamps negative results from over-reporting viewports", () => {
    expect(computeKeyboardInset(844, 900, 0)).toBe(0);
  });

  it("rounds fractional viewport heights to whole pixels", () => {
    expect(computeKeyboardInset(844, 507.6, 0)).toBe(336);
  });
});
