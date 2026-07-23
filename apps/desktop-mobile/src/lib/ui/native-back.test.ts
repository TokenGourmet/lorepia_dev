import { describe, expect, it } from "vitest";

import { normalizeNativeBackStatus } from "./native-back";

describe("native back status boundary", () => {
  it("accepts only explicit boolean status fields", () => {
    expect(
      normalizeNativeBackStatus({
        supported: true,
        active: true,
        gestureEnabled: true,
      }),
    ).toEqual({
      supported: true,
      active: true,
      gestureEnabled: true,
    });
  });

  it("fails closed for malformed native payloads", () => {
    expect(
      normalizeNativeBackStatus({
        supported: "true",
        active: 1,
        gestureEnabled: null,
      }),
    ).toEqual({
      supported: false,
      active: false,
      gestureEnabled: false,
    });
    expect(normalizeNativeBackStatus(null)).toEqual({
      supported: false,
      active: false,
      gestureEnabled: false,
    });
  });
});
