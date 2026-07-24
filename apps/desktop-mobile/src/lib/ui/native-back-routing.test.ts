import { describe, expect, it } from "vitest";

import {
  isAndroidNativeBackRoute,
  planAndroidNativeBack,
} from "./native-back-routing";

const origin = "https://lorepia.invalid";

describe("Android native back route ownership", () => {
  it("owns every detail route and no root tab", () => {
    for (const path of [
      "/chat",
      "/chat/info",
      "/chat/report",
      "/character/seraphine",
      "/import",
      "/community",
    ]) {
      expect(isAndroidNativeBackRoute(path)).toBe(true);
    }
    for (const path of ["/", "/home", "/create", "/account"]) {
      expect(isAndroidNativeBackRoute(path)).toBe(false);
    }
  });

  it("uses history only for a safe matching route state", () => {
    expect(
      planAndroidNativeBack(
        new URL("/chat/info?character=seraphine", origin),
        { backHref: "/chat?character=seraphine" },
        2,
      ),
    ).toEqual({ kind: "history" });
    expect(
      planAndroidNativeBack(
        new URL(
          "/chat/report?character=seraphine&chatId=abc",
          origin,
        ),
        {
          backHref:
            "/chat/info?character=seraphine&chatId=abc",
        },
        3,
      ),
    ).toEqual({ kind: "history" });
    expect(
      planAndroidNativeBack(
        new URL("/chat/info?character=seraphine", origin),
        { backHref: "//hostile.example" },
        3,
      ),
    ).toEqual({
      kind: "replace",
      href: "/chat?character=seraphine",
    });
  });

  it("preserves chat identity in direct-load fallbacks", () => {
    expect(
      planAndroidNativeBack(
        new URL(
          "/chat/report?character=seraphine&chatId=abc",
          origin,
        ),
        null,
        1,
      ),
    ).toEqual({
      kind: "replace",
      href: "/chat/info?character=seraphine&chatId=abc",
    });
    expect(
      planAndroidNativeBack(
        new URL("/chat/info?character=seraphine&chatId=abc", origin),
        null,
        1,
      ),
    ).toEqual({
      kind: "replace",
      href: "/chat?character=seraphine",
    });
  });

  it("falls back to the route's stable parent", () => {
    expect(
      planAndroidNativeBack(
        new URL("/chat?character=seraphine", origin),
        { backHref: "/character/seraphine" },
        1,
      ),
    ).toEqual({
      kind: "replace",
      href: "/character/seraphine",
    });
    expect(
      planAndroidNativeBack(
        new URL("/character/seraphine", origin),
        null,
        8,
      ),
    ).toEqual({ kind: "replace", href: "/" });
    expect(
      planAndroidNativeBack(
        new URL("/community", origin),
        null,
        8,
      ),
    ).toEqual({ kind: "replace", href: "/home" });
    expect(
      planAndroidNativeBack(new URL("/", origin), null, 8),
    ).toBeNull();
  });
});
