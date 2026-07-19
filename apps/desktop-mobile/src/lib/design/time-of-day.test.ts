import { describe, expect, it } from "vitest";

import { dayPeriod, formatMessageTime } from "./time-of-day";

describe("dayPeriod", () => {
  it("maps every hour to a period boundary contract", () => {
    expect(dayPeriod(0)).toBe("새벽");
    expect(dayPeriod(5)).toBe("새벽");
    expect(dayPeriod(6)).toBe("아침");
    expect(dayPeriod(10)).toBe("아침");
    expect(dayPeriod(11)).toBe("낮");
    expect(dayPeriod(16)).toBe("낮");
    expect(dayPeriod(17)).toBe("저녁");
    expect(dayPeriod(20)).toBe("저녁");
    expect(dayPeriod(21)).toBe("밤");
    expect(dayPeriod(23)).toBe("밤");
  });

  it("rejects out-of-range and fractional hours", () => {
    expect(() => dayPeriod(-1)).toThrow(RangeError);
    expect(() => dayPeriod(24)).toThrow(RangeError);
    expect(() => dayPeriod(1.5)).toThrow(RangeError);
  });
});

describe("formatMessageTime", () => {
  it("renders a 12-hour clock with the period word carrying meridiem", () => {
    expect(formatMessageTime(new Date(2026, 6, 19, 23, 42))).toBe("밤 11:42");
    expect(formatMessageTime(new Date(2026, 6, 19, 3, 7))).toBe("새벽 3:07");
    expect(formatMessageTime(new Date(2026, 6, 19, 0, 0))).toBe("새벽 12:00");
    expect(formatMessageTime(new Date(2026, 6, 19, 12, 5))).toBe("낮 12:05");
    expect(formatMessageTime(new Date(2026, 6, 19, 9, 30))).toBe("아침 9:30");
    expect(formatMessageTime(new Date(2026, 6, 19, 18, 59))).toBe("저녁 6:59");
  });
});
