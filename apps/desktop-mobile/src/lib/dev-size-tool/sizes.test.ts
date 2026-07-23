import { describe, expect, it } from "vitest";

import {
  DEV_SIZE_LIMITS,
  DEV_SIZE_PRESETS,
  normalizeLogicalSize,
} from "./sizes";

describe("development window size presets", () => {
  it("keeps every preset inside the supported logical-size range", () => {
    for (const preset of DEV_SIZE_PRESETS) {
      expect(normalizeLogicalSize(preset.width, preset.height)).toEqual({
        width: preset.width,
        height: preset.height,
      });
    }
  });

  it("rounds and clamps custom sizes", () => {
    expect(normalizeLogicalSize(100.6, 2000.4)).toEqual({
      width: DEV_SIZE_LIMITS.minWidth,
      height: DEV_SIZE_LIMITS.maxHeight,
    });
  });

  it("falls back to the S25 work size for invalid input", () => {
    expect(normalizeLogicalSize(Number.NaN, Number.POSITIVE_INFINITY)).toEqual({
      width: 360,
      height: 780,
    });
  });
});
