import { describe, expect, it, vi } from "vitest";

import { createPreferenceCloseHandler } from "./preference-close-guard";

describe("preference close guard", () => {
  it("prevents close synchronously and destroys only after flush", async () => {
    let releaseFlush: () => void = () => undefined;
    const flush = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          releaseFlush = resolve;
        }),
    );
    const destroy = vi.fn(async () => undefined);
    const preventDefault = vi.fn();
    const close = createPreferenceCloseHandler(flush, destroy);

    const finishing = close({ preventDefault });
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(flush).toHaveBeenCalledOnce();
    expect(destroy).not.toHaveBeenCalled();

    releaseFlush();
    await finishing;
    expect(destroy).toHaveBeenCalledOnce();
  });

  it("coalesces repeated close requests and still closes after flush failure", async () => {
    const flush = vi.fn(async () => {
      throw new Error("write failed");
    });
    const destroy = vi.fn(async () => undefined);
    const firstPrevent = vi.fn();
    const secondPrevent = vi.fn();
    const close = createPreferenceCloseHandler(flush, destroy);

    await expect(close({ preventDefault: firstPrevent })).rejects.toThrow(
      "write failed",
    );
    await close({ preventDefault: secondPrevent });

    expect(firstPrevent).toHaveBeenCalledOnce();
    expect(secondPrevent).toHaveBeenCalledOnce();
    expect(flush).toHaveBeenCalledOnce();
    expect(destroy).toHaveBeenCalledOnce();
  });
});
