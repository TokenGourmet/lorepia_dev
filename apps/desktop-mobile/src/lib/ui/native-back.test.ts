import { afterEach, describe, expect, it, vi } from "vitest";

import {
  NATIVE_BACK_PROGRESS_EVENT,
  connectNativeBackCommit,
  connectNativeBackProgress,
  normalizeNativeBackProgress,
  normalizeNativeBackStatus,
  shouldOptimisticallyArmNativeBack,
  usesNativeBackChrome,
} from "./native-back";

describe("native back status boundary", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

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

  it("normalizes the shared native progress contract", () => {
    expect(NATIVE_BACK_PROGRESS_EVENT).toBe(
      "lorepia:native-back-progress",
    );
    expect(
      normalizeNativeBackProgress({
        phase: "progress",
        progress: 0.42,
      }),
    ).toEqual({
      phase: "progress",
      progress: 0.42,
      edge: "left",
    });
    expect(
      normalizeNativeBackProgress({
        phase: "commit",
        progress: 1.5,
      }),
    ).toEqual({
      phase: "commit",
      progress: 1,
      edge: "left",
    });
    expect(
      normalizeNativeBackProgress({
        phase: "cancel",
        progress: -1,
      }),
    ).toEqual({
      phase: "cancel",
      progress: 0,
      edge: "left",
    });
    expect(
      normalizeNativeBackProgress({
        phase: "progress",
        progress: 0.5,
        edge: "right",
      }),
    ).toEqual({
      phase: "progress",
      progress: 0.5,
      edge: "right",
    });
    expect(
      normalizeNativeBackProgress({
        phase: "dragging",
        progress: 0.5,
      }),
    ).toBeNull();
    expect(
      normalizeNativeBackProgress({
        phase: "progress",
        progress: Number.NaN,
      }),
    ).toBeNull();
  });

  it("reserves native header ownership for UIKit while Android keeps web fallback", () => {
    const active = {
      supported: true,
      active: true,
      gestureEnabled: true,
    };
    expect(usesNativeBackChrome(active, "ios")).toBe(true);
    expect(usesNativeBackChrome(active, "android")).toBe(false);
    expect(usesNativeBackChrome(active, undefined)).toBe(false);
    expect(
      usesNativeBackChrome(
        { ...active, gestureEnabled: false },
        "ios",
      ),
    ).toBe(false);
    expect(shouldOptimisticallyArmNativeBack("ios")).toBe(true);
    expect(shouldOptimisticallyArmNativeBack("android")).toBe(false);
    expect(shouldOptimisticallyArmNativeBack(undefined)).toBe(false);
  });

  it("accepts structurally valid progress events across WebView realms", () => {
    const fakeWindow = new EventTarget();
    vi.stubGlobal("window", fakeWindow);
    const seen: unknown[] = [];
    const disconnect = connectNativeBackProgress((progress) => {
      seen.push(progress);
    });
    const event = new Event(NATIVE_BACK_PROGRESS_EVENT);
    Object.defineProperty(event, "detail", {
      value: { phase: "progress", progress: 0.4 },
    });
    fakeWindow.dispatchEvent(event);
    expect(seen).toEqual([
      { phase: "progress", progress: 0.4, edge: "left" },
    ]);

    disconnect();
    fakeWindow.dispatchEvent(event);
    expect(seen).toHaveLength(1);
  });

  it("keeps commit ownership separate from native enablement", async () => {
    const fakeWindow = new EventTarget();
    vi.stubGlobal("window", fakeWindow);
    const onBack = vi.fn();
    const disconnect = connectNativeBackCommit(onBack);

    fakeWindow.dispatchEvent(new Event("lorepia:native-back"));
    await Promise.resolve();
    await Promise.resolve();
    expect(onBack).toHaveBeenCalledOnce();

    disconnect();
    fakeWindow.dispatchEvent(new Event("lorepia:native-back"));
    await Promise.resolve();
    await Promise.resolve();
    expect(onBack).toHaveBeenCalledOnce();
  });
});
