import { describe, expect, it, vi } from "vitest";

import {
  NATIVE_CHROME_SET_STATE_COMMAND,
  NATIVE_CHROME_STATUS_COMMAND,
  NATIVE_CHROME_TAB_EVENT,
  UNSUPPORTED_NATIVE_CHROME_STATUS,
  connectNativeTabSelection,
  createNativeChromeStateSync,
  createNativeTabNavigationSync,
  getNativeChromeStatus,
  hrefForNativeTab,
  nativeTabForHref,
  normalizeNativeChromeStatus,
  normalizeNativeTabEventDetail,
  setNativeChromeState,
  type NativeChromeState,
  type NativeChromeStatus,
  type NativeChromeTransport,
  type NativeTabHref,
} from "./native-chrome";

const LIBRARY_STATE: NativeChromeState = Object.freeze({
  visible: true,
  selectedTab: "library",
  minimized: false,
  appearance: "system",
  compact: true,
});

function statusFor(
  state: NativeChromeState,
): NativeChromeStatus {
  return {
    supported: true,
    active: state.compact,
    compact: state.compact,
    visible: state.visible && state.compact,
    selectedTab: state.selectedTab,
    minimized: state.minimized,
  };
}

function transport(
  invokeCommand: NativeChromeTransport["invoke"],
  available = true,
): NativeChromeTransport {
  return {
    isTauri: () => available,
    invoke: invokeCommand,
  };
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
} {
  let resolve!: (value: T) => void;
  return {
    promise: new Promise<T>((complete) => {
      resolve = complete;
    }),
    resolve,
  };
}

describe("native chrome fixed tab contract", () => {
  it("maps only the four closed tab and href values", () => {
    expect(hrefForNativeTab("home")).toBe("/home");
    expect(hrefForNativeTab("library")).toBe("/");
    expect(hrefForNativeTab("create")).toBe("/create");
    expect(hrefForNativeTab("account")).toBe("/account");

    expect(nativeTabForHref("/home")).toBe("home");
    expect(nativeTabForHref("/")).toBe("library");
    expect(nativeTabForHref("/create")).toBe("create");
    expect(nativeTabForHref("/account")).toBe("account");

    expect(hrefForNativeTab("settings")).toBeNull();
    expect(nativeTabForHref("/account/profile")).toBeNull();
    expect(nativeTabForHref("/home?preview=1")).toBeNull();
    expect(nativeTabForHref(null)).toBeNull();
  });

  it("accepts only the exact one-field native tab event detail", () => {
    expect(normalizeNativeTabEventDetail({ tab: "library" })).toEqual({
      tab: "library",
    });
    expect(
      normalizeNativeTabEventDetail({
        tab: "library",
        href: "/",
      }),
    ).toBeNull();
    expect(normalizeNativeTabEventDetail({ tab: "settings" })).toBeNull();
    expect(normalizeNativeTabEventDetail(null)).toBeNull();
  });
});

describe("native chrome status boundary", () => {
  it("accepts an exact internally consistent status", () => {
    expect(
      normalizeNativeChromeStatus({
        supported: true,
        active: true,
        compact: true,
        visible: true,
        selectedTab: "home",
        minimized: false,
      }),
    ).toEqual({
      supported: true,
      active: true,
      compact: true,
      visible: true,
      selectedTab: "home",
      minimized: false,
    });

    expect(
      normalizeNativeChromeStatus({
        supported: false,
        active: false,
        compact: true,
        visible: false,
        selectedTab: "library",
        minimized: false,
      }),
    ).toEqual({
      supported: false,
      active: false,
      compact: true,
      visible: false,
      selectedTab: "library",
      minimized: false,
    });
  });

  it("fails the entire response closed for partial, widened, or contradictory data", () => {
    const malformed = [
      null,
      [],
      {
        supported: true,
        active: true,
        compact: true,
        visible: true,
        selectedTab: "home",
      },
      {
        supported: true,
        active: true,
        compact: true,
        visible: true,
        selectedTab: "home",
        minimized: false,
        ready: true,
      },
      {
        supported: true,
        active: false,
        compact: true,
        visible: false,
        selectedTab: "home",
        minimized: false,
      },
      {
        supported: false,
        active: false,
        compact: true,
        visible: true,
        selectedTab: "home",
        minimized: false,
      },
      {
        supported: true,
        active: true,
        compact: true,
        visible: true,
        selectedTab: "settings",
        minimized: false,
      },
    ];

    for (const value of malformed) {
      expect(normalizeNativeChromeStatus(value)).toBe(
        UNSUPPORTED_NATIVE_CHROME_STATUS,
      );
    }
  });
});

describe("native chrome Tauri commands", () => {
  it("invokes the exact plugin commands and payload envelope", async () => {
    const invokeCommand = vi
      .fn<NativeChromeTransport["invoke"]>()
      .mockImplementation(async (command) =>
        command === NATIVE_CHROME_STATUS_COMMAND
          ? statusFor(LIBRARY_STATE)
          : statusFor(LIBRARY_STATE),
      );
    const nativeTransport = transport(invokeCommand);

    await expect(
      setNativeChromeState(LIBRARY_STATE, nativeTransport),
    ).resolves.toEqual(statusFor(LIBRARY_STATE));
    await expect(
      getNativeChromeStatus(nativeTransport),
    ).resolves.toEqual(statusFor(LIBRARY_STATE));

    expect(invokeCommand).toHaveBeenNthCalledWith(
      1,
      NATIVE_CHROME_SET_STATE_COMMAND,
      { payload: LIBRARY_STATE },
    );
    expect(invokeCommand).toHaveBeenNthCalledWith(
      2,
      NATIVE_CHROME_STATUS_COMMAND,
    );
  });

  it("does not invoke outside Tauri or with malformed state", async () => {
    const invokeCommand = vi.fn<NativeChromeTransport["invoke"]>();
    const browserTransport = transport(invokeCommand, false);

    await expect(
      setNativeChromeState(LIBRARY_STATE, browserTransport),
    ).resolves.toBe(UNSUPPORTED_NATIVE_CHROME_STATUS);
    await expect(
      getNativeChromeStatus(browserTransport),
    ).resolves.toBe(UNSUPPORTED_NATIVE_CHROME_STATUS);
    await expect(
      setNativeChromeState(
        {
          ...LIBRARY_STATE,
          selectedTab: "settings",
        } as unknown as NativeChromeState,
        transport(invokeCommand),
      ),
    ).resolves.toBe(UNSUPPORTED_NATIVE_CHROME_STATUS);
    expect(invokeCommand).not.toHaveBeenCalled();
  });

  it("fails closed when the native command rejects or widens its response", async () => {
    const rejected = transport(
      vi.fn<NativeChromeTransport["invoke"]>().mockRejectedValue(
        new Error("unavailable"),
      ),
    );
    await expect(
      setNativeChromeState(LIBRARY_STATE, rejected),
    ).resolves.toBe(UNSUPPORTED_NATIVE_CHROME_STATUS);

    const widened = transport(
      vi.fn<NativeChromeTransport["invoke"]>().mockResolvedValue({
        ...statusFor(LIBRARY_STATE),
        ready: true,
      }),
    );
    await expect(
      getNativeChromeStatus(widened),
    ).resolves.toBe(UNSUPPORTED_NATIVE_CHROME_STATUS);
  });
});

describe("native chrome tab event connection", () => {
  it("delivers only validated cross-realm event details and disconnects", () => {
    const target = new EventTarget();
    const onTab = vi.fn();
    const disconnect = connectNativeTabSelection(onTab, target);

    const valid = new Event(NATIVE_CHROME_TAB_EVENT);
    Object.defineProperty(valid, "detail", {
      value: { tab: "account" },
    });
    target.dispatchEvent(valid);

    const invalid = new Event(NATIVE_CHROME_TAB_EVENT);
    Object.defineProperty(invalid, "detail", {
      value: { tab: "account", href: "/account" },
    });
    target.dispatchEvent(invalid);

    expect(onTab).toHaveBeenCalledOnce();
    expect(onTab).toHaveBeenCalledWith("account");

    disconnect();
    target.dispatchEvent(valid);
    expect(onTab).toHaveBeenCalledOnce();
  });
});

describe("native chrome tab navigation synchronization", () => {
  it("honors a return to the current tab while another tab is in flight", async () => {
    let current: NativeTabHref = "/";
    const first = deferred<void>();
    const navigate = vi.fn(
      async (href: NativeTabHref): Promise<void> => {
        if (href === "/home") {
          await first.promise;
        }
        current = href;
      },
    );
    const sync = createNativeTabNavigationSync(
      (href) => current === href,
      navigate,
    );

    expect(sync.request("/")).toBe(false);
    expect(sync.request("/home")).toBe(true);
    expect(sync.request("/")).toBe(true);

    first.resolve();
    await sync.flush();

    expect(navigate.mock.calls.map(([href]) => href)).toEqual([
      "/home",
      "/",
    ]);
    expect(current).toBe("/");
  });

  it("retains only the latest successor and stops after disposal", async () => {
    let current: NativeTabHref = "/";
    const first = deferred<void>();
    const navigate = vi.fn(
      async (href: NativeTabHref): Promise<void> => {
        if (href === "/home") {
          await first.promise;
        }
        current = href;
      },
    );
    const sync = createNativeTabNavigationSync(
      (href) => current === href,
      navigate,
    );

    expect(sync.request("/home")).toBe(true);
    expect(sync.request("/account")).toBe(true);
    expect(sync.request("/create")).toBe(true);
    first.resolve();
    await sync.flush();

    expect(navigate.mock.calls.map(([href]) => href)).toEqual([
      "/home",
      "/create",
    ]);
    sync.dispose();
    expect(sync.request("/account")).toBe(false);
  });
});

describe("native chrome serialized state sync", () => {
  it("deduplicates and retains only the latest queued state", async () => {
    const first = deferred<unknown>();
    const invokeCommand = vi
      .fn<NativeChromeTransport["invoke"]>()
      .mockImplementationOnce(() => first.promise)
      .mockImplementation(async (_command, args) => {
        const state = (args?.payload ?? null) as NativeChromeState;
        return statusFor(state);
      });
    const seen: NativeChromeStatus[] = [];
    const sync = createNativeChromeStateSync(
      (status) => seen.push(status),
      transport(invokeCommand),
    );
    const home: NativeChromeState = {
      ...LIBRARY_STATE,
      selectedTab: "home",
    };
    const account: NativeChromeState = {
      ...LIBRARY_STATE,
      selectedTab: "account",
      appearance: "dark",
    };

    expect(sync.update(LIBRARY_STATE)).toBe(true);
    expect(sync.update(LIBRARY_STATE)).toBe(false);
    expect(sync.update(home)).toBe(true);
    expect(sync.update(account)).toBe(true);
    expect(invokeCommand).toHaveBeenCalledOnce();

    first.resolve(statusFor(LIBRARY_STATE));
    await sync.flush();

    expect(invokeCommand).toHaveBeenCalledTimes(2);
    expect(invokeCommand).toHaveBeenNthCalledWith(
      2,
      NATIVE_CHROME_SET_STATE_COMMAND,
      { payload: account },
    );
    expect(seen).toEqual([
      statusFor(LIBRARY_STATE),
      statusFor(account),
    ]);
    expect(sync.update(account)).toBe(false);
  });

  it("cancels a queued successor when the latest state returns to the in-flight state", async () => {
    const first = deferred<unknown>();
    const invokeCommand = vi
      .fn<NativeChromeTransport["invoke"]>()
      .mockImplementationOnce(() => first.promise);
    const sync = createNativeChromeStateSync(
      undefined,
      transport(invokeCommand),
    );
    const minimized: NativeChromeState = {
      ...LIBRARY_STATE,
      minimized: true,
    };

    expect(sync.update(LIBRARY_STATE)).toBe(true);
    expect(sync.update(minimized)).toBe(true);
    expect(sync.update(LIBRARY_STATE)).toBe(true);

    first.resolve(statusFor(LIBRARY_STATE));
    await sync.flush();
    expect(invokeCommand).toHaveBeenCalledOnce();
  });

  it("drops queued work and callbacks after disposal", async () => {
    const first = deferred<unknown>();
    const invokeCommand = vi
      .fn<NativeChromeTransport["invoke"]>()
      .mockImplementationOnce(() => first.promise);
    const onStatus = vi.fn();
    const sync = createNativeChromeStateSync(
      onStatus,
      transport(invokeCommand),
    );

    expect(sync.update(LIBRARY_STATE)).toBe(true);
    sync.dispose();
    expect(sync.update(LIBRARY_STATE)).toBe(false);
    await sync.flush();

    first.resolve(statusFor(LIBRARY_STATE));
    await Promise.resolve();
    await Promise.resolve();
    expect(onStatus).not.toHaveBeenCalled();
  });

  it("retries once and never caches a failed application as complete", async () => {
    const invokeCommand = vi
      .fn<NativeChromeTransport["invoke"]>()
      .mockRejectedValueOnce(new Error("transient"))
      .mockRejectedValueOnce(new Error("still unavailable"))
      .mockResolvedValue(statusFor(LIBRARY_STATE));
    const seen: NativeChromeStatus[] = [];
    const sync = createNativeChromeStateSync(
      (status) => seen.push(status),
      transport(invokeCommand),
    );

    expect(sync.update(LIBRARY_STATE)).toBe(true);
    await sync.flush();
    expect(invokeCommand).toHaveBeenCalledTimes(2);
    expect(seen).toEqual([
      UNSUPPORTED_NATIVE_CHROME_STATUS,
      UNSUPPORTED_NATIVE_CHROME_STATUS,
    ]);

    expect(sync.update(LIBRARY_STATE)).toBe(true);
    await sync.flush();
    expect(invokeCommand).toHaveBeenCalledTimes(3);
    expect(seen.at(-1)).toEqual(statusFor(LIBRARY_STATE));
    expect(sync.update(LIBRARY_STATE)).toBe(false);
  });

  it("caches a valid platform-unsupported response without retrying it", async () => {
    const invokeCommand = vi
      .fn<NativeChromeTransport["invoke"]>()
      .mockResolvedValue({ ...UNSUPPORTED_NATIVE_CHROME_STATUS });
    const sync = createNativeChromeStateSync(
      undefined,
      transport(invokeCommand),
    );

    expect(sync.update(LIBRARY_STATE)).toBe(true);
    await sync.flush();
    expect(invokeCommand).toHaveBeenCalledOnce();
    expect(sync.update(LIBRARY_STATE)).toBe(false);
  });
});
