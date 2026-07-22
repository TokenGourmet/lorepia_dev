import { describe, expect, it, vi } from "vitest";

import { createFixedWindowAdmission } from "./fixed-window-admission";

describe("fixed-window admission", () => {
  it("rejects excess work without inspecting attacker data", () => {
    let now = 0;
    const gate = createFixedWindowAdmission({
      maxAttempts: 2,
      windowMs: 100,
      clock: () => now,
    });

    expect(gate.consume()).toBe(true);
    expect(gate.consume()).toBe(true);
    expect(gate.consume()).toBe(false);

    now = 100;
    expect(gate.consume()).toBe(true);
  });

  it("fails closed for invalid or throwing clocks", () => {
    const throwing = createFixedWindowAdmission({
      maxAttempts: 1,
      windowMs: 100,
      clock: () => {
        throw new Error("clock failed");
      },
    });
    const invalid = createFixedWindowAdmission({
      maxAttempts: 1,
      windowMs: 100,
      clock: () => Number.NaN,
    });

    expect(throwing.consume()).toBe(false);
    expect(invalid.consume()).toBe(false);
  });

  it("does not reset when the clock regresses", () => {
    let now = 10;
    const gate = createFixedWindowAdmission({
      maxAttempts: 1,
      windowMs: 100,
      clock: () => now,
    });

    expect(gate.consume()).toBe(true);
    now = 0;
    expect(gate.consume()).toBe(false);
    now = 110;
    expect(gate.consume()).toBe(true);
  });

  it("rejects invalid configuration", () => {
    expect(() =>
      createFixedWindowAdmission({ maxAttempts: 0, windowMs: 1 }),
    ).toThrow(RangeError);
    expect(() =>
      createFixedWindowAdmission({ maxAttempts: 1, windowMs: 0 }),
    ).toThrow(RangeError);

    const clock = vi.fn(() => 0);
    createFixedWindowAdmission({ maxAttempts: 1, windowMs: 1, clock });
    expect(clock).not.toHaveBeenCalled();
  });
});
