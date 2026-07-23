import { afterEach, describe, expect, it, vi } from "vitest";

import {
  classifyContentBackIntent,
  contentBackReleaseVelocity,
  contentSwipeBack,
  needsExplicitContentBackCapture,
  shouldCommitContentBack,
} from "./content-back";

describe("iOS 26 content backswipe", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("waits through slop, accepts a rightward horizontal pan, and rejects competing intent", () => {
    expect(classifyContentBackIntent(7, 7)).toBe("pending");
    expect(classifyContentBackIntent(9, 4)).toBe("accept");
    expect(classifyContentBackIntent(-12, 1)).toBe("reject");
    expect(classifyContentBackIntent(8, 8)).toBe("reject");
    expect(classifyContentBackIntent(4, 12)).toBe("reject");
  });

  it("commits by distance or by a short fast flick", () => {
    expect(shouldCommitContentBack(127, 360, 0)).toBe(true);
    expect(shouldCommitContentBack(120, 360, 0.4)).toBe(false);
    expect(shouldCommitContentBack(25, 360, 0.51)).toBe(true);
    expect(shouldCommitContentBack(24, 360, 2)).toBe(false);
  });

  it("relies on implicit capture for Android touch streams", () => {
    expect(needsExplicitContentBackCapture("touch")).toBe(false);
    expect(needsExplicitContentBackCapture("pen")).toBe(false);
    expect(needsExplicitContentBackCapture("mouse")).toBe(true);
    expect(needsExplicitContentBackCapture("")).toBe(true);
  });

  it("derives release velocity only from fresh samples", () => {
    expect(
      contentBackReleaseVelocity(
        [
          { x: 10, time: 100 },
          { x: 60, time: 150 },
        ],
        150,
      ),
    ).toBe(1);

    expect(
      contentBackReleaseVelocity(
        [
          { x: 10, time: 100 },
          { x: 60, time: 150 },
          { x: 60, time: 300 },
        ],
        300,
      ),
    ).toBe(0);
  });

  it("fully detaches the web fallback while native back is active", () => {
    const listeners = new Map<string, Set<EventListener>>();
    const node = {
      style: { touchAction: "auto" },
      addEventListener(type: string, listener: EventListener): void {
        const entries = listeners.get(type) ?? new Set<EventListener>();
        entries.add(listener);
        listeners.set(type, entries);
      },
      removeEventListener(type: string, listener: EventListener): void {
        listeners.get(type)?.delete(listener);
      },
    } as unknown as HTMLElement;
    vi.stubGlobal("window", {
      matchMedia: () => ({ matches: false }),
    });

    const action = contentSwipeBack(node, {
      enabled: true,
      onBack: () => undefined,
    });
    expect(node.style.touchAction).toBe("pan-y");
    expect(
      [...listeners.values()].reduce((total, set) => total + set.size, 0),
    ).toBe(9);

    action.update({
      enabled: false,
      onBack: () => undefined,
    });
    expect(node.style.touchAction).toBe("auto");
    expect(
      [...listeners.values()].reduce((total, set) => total + set.size, 0),
    ).toBe(0);

    action.update({
      enabled: true,
      onBack: () => undefined,
    });
    expect(node.style.touchAction).toBe("pan-y");

    action.destroy();
    expect(node.style.touchAction).toBe("auto");
  });

  it("cancels a direct-manipulation drag when a second touch arrives and restores selection", () => {
    vi.useFakeTimers();
    const listeners = new Map<string, Set<EventListener>>();
    class FakeElement {
      clientWidth = 360;
      parentElement = null;
      style = {
        touchAction: "auto",
        translate: "",
        transition: "",
        willChange: "",
        boxShadow: "",
        userSelect: "text",
        getPropertyValue: () => "",
        setProperty: () => undefined,
        removeProperty: () => undefined,
      };

      addEventListener(type: string, listener: EventListener): void {
        const entries = listeners.get(type) ?? new Set<EventListener>();
        entries.add(listener);
        listeners.set(type, entries);
      }

      removeEventListener(type: string, listener: EventListener): void {
        listeners.get(type)?.delete(listener);
      }

      closest(): null {
        return null;
      }

      contains(): boolean {
        return false;
      }

      getBoundingClientRect(): Pick<DOMRect, "width"> {
        return { width: 360 };
      }

      hasPointerCapture(): boolean {
        return false;
      }
    }

    vi.stubGlobal("Element", FakeElement);
    vi.stubGlobal("Node", FakeElement);
    vi.stubGlobal("window", {
      matchMedia: () => ({ matches: false }),
    });
    vi.stubGlobal(
      "requestAnimationFrame",
      (callback: FrameRequestCallback) =>
        setTimeout(() => callback(0), 0) as unknown as number,
    );
    vi.stubGlobal(
      "cancelAnimationFrame",
      (handle: number) => clearTimeout(handle),
    );

    const node = new FakeElement() as unknown as HTMLElement;
    const onBack = vi.fn();
    const action = contentSwipeBack(node, { onBack });
    const touch = (identifier: number, clientX: number, clientY = 20) => ({
      identifier,
      clientX,
      clientY,
    });
    const touchList = (...items: ReturnType<typeof touch>[]) => ({
      length: items.length,
      item: (index: number) => items[index] ?? null,
    });
    const dispatch = (
      type: string,
      event: Record<string, unknown>,
    ): void => {
      for (const listener of listeners.get(type) ?? []) {
        listener(event as unknown as Event);
      }
    };

    const first = touch(1, 10);
    dispatch("touchstart", {
      target: node,
      touches: touchList(first),
      timeStamp: 0,
    });
    dispatch("touchmove", {
      target: node,
      touches: touchList(touch(1, 170)),
      timeStamp: 50,
      preventDefault: vi.fn(),
    });
    dispatch("touchstart", {
      target: node,
      touches: touchList(touch(1, 170), touch(2, 180)),
      timeStamp: 60,
    });

    vi.advanceTimersByTime(240);
    expect(onBack).not.toHaveBeenCalled();
    expect(node.style.translate).toBe("");
    expect(node.style.userSelect).toBe("text");

    action.destroy();
  });
});
