import { describe, expect, it } from "vitest";

import {
  dayPeriod,
  formatMessageTime,
  formatThreadStamp,
} from "./time-of-day";

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

describe("formatThreadStamp", () => {
  const now = new Date(2026, 6, 23, 10, 0);

  it("uses relative day words for today and yesterday", () => {
    expect(formatThreadStamp(new Date(2026, 6, 23, 0, 12), now)).toBe(
      "오늘 새벽 12:12",
    );
    expect(formatThreadStamp(new Date(2026, 6, 22, 23, 42), now)).toBe(
      "어제 밤 11:42",
    );
  });

  it("switches to calendar dates two days back, adding the year only when it differs", () => {
    expect(formatThreadStamp(new Date(2026, 6, 21, 9, 5), now)).toBe(
      "7월 21일 아침 9:05",
    );
    expect(formatThreadStamp(new Date(2025, 11, 31, 18, 30), now)).toBe(
      "2025년 12월 31일 저녁 6:30",
    );
  });
});
