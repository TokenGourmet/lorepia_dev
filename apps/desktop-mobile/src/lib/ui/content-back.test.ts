import { afterEach, describe, expect, it, vi } from "vitest";

import {
  classifyContentBackIntent,
  contentBackReleaseVelocity,
  contentSwipeBack,
  shouldCommitContentBack,
} from "./content-back";

describe("iOS 26 content backswipe", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
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
    ).toBe(5);

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
});
