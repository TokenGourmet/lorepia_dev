import { describe, expect, it } from "vitest";

import {
  approachValue,
  clampGlassPoint,
  liquidGlowIntensity,
  liquidRippleFrame,
} from "./liquid-glass";

describe("liquid glass interaction math", () => {
  it("clamps pointer coordinates to the current glass bounds", () => {
    expect(
      clampGlassPoint(
        { x: -20, y: 90 },
        { width: 80, height: 44 },
      ),
    ).toEqual({ x: 0, y: 44 });
  });

  it("prioritizes press, hover, and focus glow strengths", () => {
    expect(
      liquidGlowIntensity({ pressed: true, hovered: true, focused: true }),
    ).toBe(1);
    expect(
      liquidGlowIntensity({ pressed: false, hovered: true, focused: true }),
    ).toBe(0.56);
    expect(
      liquidGlowIntensity({ pressed: false, hovered: false, focused: true }),
    ).toBe(0.38);
    expect(
      liquidGlowIntensity({ pressed: false, hovered: false, focused: false }),
    ).toBe(0);
  });

  it("approaches a target without overshooting", () => {
    expect(approachValue(0, 10, 0.25)).toBe(2.5);
    expect(approachValue(0, 10, 4)).toBe(10);
    expect(approachValue(10, 0, -2)).toBe(10);
  });

  it("finishes the touch ripple at the configured duration", () => {
    expect(liquidRippleFrame(0)).toMatchObject({
      progress: 0,
      opacity: 0.22,
      active: true,
    });
    expect(liquidRippleFrame(460)).toMatchObject({
      progress: 1,
      opacity: 0,
      active: false,
    });
  });
});
