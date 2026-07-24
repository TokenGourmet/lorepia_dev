import { invoke, isTauri } from "@tauri-apps/api/core";

export const NATIVE_CHROME_SET_STATE_COMMAND =
  "plugin:native-chrome|set_state";
export const NATIVE_CHROME_STATUS_COMMAND =
  "plugin:native-chrome|status";
export const NATIVE_CHROME_TAB_EVENT = "lorepia:native-tab";

export const NATIVE_TAB_IDS = [
  "home",
  "library",
  "create",
  "account",
] as const;
export type NativeTabId = (typeof NATIVE_TAB_IDS)[number];

export const NATIVE_CHROME_APPEARANCES = [
  "system",
  "light",
  "dark",
] as const;
export type NativeChromeAppearance =
  (typeof NATIVE_CHROME_APPEARANCES)[number];

export type NativeTabHref =
  | "/home"
  | "/"
  | "/create"
  | "/account";

export interface NativeChromeState {
  visible: boolean;
  selectedTab: NativeTabId;
  minimized: boolean;
  appearance: NativeChromeAppearance;
  compact: boolean;
}

export interface NativeChromeStatus {
  supported: boolean;
  active: boolean;
  compact: boolean;
  visible: boolean;
  selectedTab: NativeTabId;
  minimized: boolean;
}

export interface NativeChromeTransport {
  isTauri(): boolean;
  invoke(
    command: string,
    args?: Record<string, unknown>,
  ): Promise<unknown>;
}

export interface NativeChromeStateSync {
  /**
   * Queues the latest distinct state. Returns false for malformed, disposed,
   * or already-current input.
   */
  update(state: NativeChromeState): boolean;
  /** Resolves when the current command and its latest queued successor finish. */
  flush(): Promise<void>;
  /** Stops callbacks and discards state that has not started yet. */
  dispose(): void;
}

export interface NativeTabNavigationSync {
  /**
   * Retains the latest tab destination while an earlier navigation settles.
   * Returns false for disposed, duplicate, or already-current input.
   */
  request(href: NativeTabHref): boolean;
  /** Resolves when the current navigation and latest queued successor finish. */
  flush(): Promise<void>;
  /** Stops new navigation and discards a successor that has not started. */
  dispose(): void;
}

const NATIVE_TAB_TO_HREF: Readonly<
  Record<NativeTabId, NativeTabHref>
> = Object.freeze({
  home: "/home",
  library: "/",
  create: "/create",
  account: "/account",
});

const NATIVE_HREF_TO_TAB: Readonly<
  Record<NativeTabHref, NativeTabId>
> = Object.freeze({
  "/home": "home",
  "/": "library",
  "/create": "create",
  "/account": "account",
});

const STATUS_KEYS = [
  "supported",
  "active",
  "compact",
  "visible",
  "selectedTab",
  "minimized",
] as const;
const STATE_KEYS = [
  "visible",
  "selectedTab",
  "minimized",
  "appearance",
  "compact",
] as const;

export const UNSUPPORTED_NATIVE_CHROME_STATUS: NativeChromeStatus =
  Object.freeze({
    supported: false,
    active: false,
    compact: false,
    visible: false,
    selectedTab: "library",
    minimized: false,
  });

const DEFAULT_TRANSPORT: NativeChromeTransport = Object.freeze({
  isTauri,
  invoke: (command: string, args?: Record<string, unknown>) =>
    invoke<unknown>(command, args),
});

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
): boolean {
  const keys = Object.keys(value);
  return (
    keys.length === expected.length &&
    expected.every((key) =>
      Object.prototype.hasOwnProperty.call(value, key),
    )
  );
}

function isNativeTabId(value: unknown): value is NativeTabId {
  return (
    typeof value === "string" &&
    (NATIVE_TAB_IDS as readonly string[]).includes(value)
  );
}

function isNativeChromeAppearance(
  value: unknown,
): value is NativeChromeAppearance {
  return (
    typeof value === "string" &&
    (NATIVE_CHROME_APPEARANCES as readonly string[]).includes(value)
  );
}

export function hrefForNativeTab(
  value: unknown,
): NativeTabHref | null {
  return isNativeTabId(value) ? NATIVE_TAB_TO_HREF[value] : null;
}

export function nativeTabForHref(
  value: unknown,
): NativeTabId | null {
  if (typeof value !== "string") return null;
  return Object.prototype.hasOwnProperty.call(NATIVE_HREF_TO_TAB, value)
    ? NATIVE_HREF_TO_TAB[value as NativeTabHref]
    : null;
}

/**
 * Native chrome owns whether the compact bar is available, but Web routing
 * remains authoritative. Any widened, contradictory, or partial response
 * falls back to the Web dock as one unit rather than trusting fields
 * independently.
 */
function parseNativeChromeStatus(
  value: unknown,
): NativeChromeStatus | null {
  if (!isRecord(value) || !hasExactKeys(value, STATUS_KEYS)) {
    return null;
  }

  const {
    supported,
    active,
    compact,
    visible,
    selectedTab,
    minimized,
  } = value;
  if (
    typeof supported !== "boolean" ||
    typeof active !== "boolean" ||
    typeof compact !== "boolean" ||
    typeof visible !== "boolean" ||
    !isNativeTabId(selectedTab) ||
    typeof minimized !== "boolean" ||
    active !== (supported && compact) ||
    (visible && !active)
  ) {
    return null;
  }

  return Object.freeze({
    supported,
    active,
    compact,
    visible,
    selectedTab,
    minimized,
  });
}

export function normalizeNativeChromeStatus(
  value: unknown,
): NativeChromeStatus {
  return (
    parseNativeChromeStatus(value) ??
    UNSUPPORTED_NATIVE_CHROME_STATUS
  );
}

function normalizeNativeChromeState(
  value: unknown,
): NativeChromeState | null {
  if (!isRecord(value) || !hasExactKeys(value, STATE_KEYS)) {
    return null;
  }

  const {
    visible,
    selectedTab,
    minimized,
    appearance,
    compact,
  } = value;
  if (
    typeof visible !== "boolean" ||
    !isNativeTabId(selectedTab) ||
    typeof minimized !== "boolean" ||
    !isNativeChromeAppearance(appearance) ||
    typeof compact !== "boolean"
  ) {
    return null;
  }

  return Object.freeze({
    visible,
    selectedTab,
    minimized,
    appearance,
    compact,
  });
}

function transportIsAvailable(
  transport: NativeChromeTransport,
): boolean {
  try {
    return transport.isTauri() === true;
  } catch {
    return false;
  }
}

export async function setNativeChromeState(
  state: NativeChromeState,
  transport: NativeChromeTransport = DEFAULT_TRANSPORT,
): Promise<NativeChromeStatus> {
  return (await applyNativeChromeState(state, transport)).status;
}

interface NativeChromeStateApplication {
  status: NativeChromeStatus;
  completed: boolean;
}

async function applyNativeChromeState(
  state: NativeChromeState,
  transport: NativeChromeTransport,
): Promise<NativeChromeStateApplication> {
  const normalized = normalizeNativeChromeState(state);
  if (normalized === null || !transportIsAvailable(transport)) {
    return {
      status: UNSUPPORTED_NATIVE_CHROME_STATUS,
      completed: true,
    };
  }

  try {
    const status = parseNativeChromeStatus(
      await transport.invoke(NATIVE_CHROME_SET_STATE_COMMAND, {
        payload: normalized,
      }),
    );
    return status === null
      ? {
          status: UNSUPPORTED_NATIVE_CHROME_STATUS,
          completed: false,
        }
      : { status, completed: true };
  } catch {
    return {
      status: UNSUPPORTED_NATIVE_CHROME_STATUS,
      completed: false,
    };
  }
}

export async function getNativeChromeStatus(
  transport: NativeChromeTransport = DEFAULT_TRANSPORT,
): Promise<NativeChromeStatus> {
  if (!transportIsAvailable(transport)) {
    return UNSUPPORTED_NATIVE_CHROME_STATUS;
  }

  try {
    return normalizeNativeChromeStatus(
      await transport.invoke(NATIVE_CHROME_STATUS_COMMAND),
    );
  } catch {
    return UNSUPPORTED_NATIVE_CHROME_STATUS;
  }
}

export function normalizeNativeTabEventDetail(
  value: unknown,
): Readonly<{ tab: NativeTabId }> | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["tab"]) ||
    !isNativeTabId(value.tab)
  ) {
    return null;
  }
  return Object.freeze({ tab: value.tab });
}

export function connectNativeTabSelection(
  onTab: (tab: NativeTabId) => void,
  target?: EventTarget,
): () => void {
  const eventTarget =
    target ??
    (typeof window === "undefined" ? null : window);
  if (eventTarget === null) {
    return () => undefined;
  }

  const handleSelection = (event: Event): void => {
    let detail: unknown;
    try {
      detail = (event as CustomEvent<unknown>).detail;
    } catch {
      return;
    }
    const selection = normalizeNativeTabEventDetail(detail);
    if (selection !== null) {
      onTab(selection.tab);
    }
  };

  eventTarget.addEventListener(
    NATIVE_CHROME_TAB_EVENT,
    handleSelection,
  );
  return () => {
    eventTarget.removeEventListener(
      NATIVE_CHROME_TAB_EVENT,
      handleSelection,
    );
  };
}

/**
 * Serializes SvelteKit tab navigations while retaining the newest destination.
 * A tap for the currently committed route must still be queued when another
 * route is in flight: it represents the user's request to return to that tab.
 */
export function createNativeTabNavigationSync(
  isCurrent: (href: NativeTabHref) => boolean,
  navigate: (href: NativeTabHref) => Promise<unknown>,
): NativeTabNavigationSync {
  let disposed = false;
  let inFlight = false;
  let inFlightHref: NativeTabHref | null = null;
  let queuedHref: NativeTabHref | null = null;
  const idleWaiters = new Set<() => void>();

  const resolveIdleWaiters = (): void => {
    if (inFlight || queuedHref !== null) return;
    for (const resolve of idleWaiters) resolve();
    idleWaiters.clear();
  };

  const drain = async (): Promise<void> => {
    if (disposed || inFlight) return;
    inFlight = true;
    try {
      while (!disposed && queuedHref !== null) {
        const next = queuedHref;
        queuedHref = null;
        inFlightHref = next;

        let current = false;
        try {
          current = isCurrent(next);
        } catch {
          // A broken route observer must not strand the navigation queue.
        }
        if (!current) {
          try {
            await navigate(next);
          } catch {
            // A rejected navigation leaves later user input eligible to run.
          }
        }
        inFlightHref = null;
      }
    } finally {
      inFlight = false;
      inFlightHref = null;
      if (!disposed && queuedHref !== null) {
        void drain();
      } else {
        resolveIdleWaiters();
      }
    }
  };

  return {
    request(href: NativeTabHref): boolean {
      if (disposed || queuedHref === href) return false;
      if (inFlightHref === href) {
        if (queuedHref === null) return false;
        queuedHref = null;
        return true;
      }

      let current = false;
      try {
        current = isCurrent(href);
      } catch {
        // Route observation is advisory; navigation remains the authority.
      }
      if (!inFlight && queuedHref === null && current) {
        return false;
      }

      queuedHref = href;
      void drain();
      return true;
    },

    flush(): Promise<void> {
      if (disposed || (!inFlight && queuedHref === null)) {
        return Promise.resolve();
      }
      return new Promise<void>((resolve) => {
        idleWaiters.add(resolve);
      });
    },

    dispose(): void {
      if (disposed) return;
      disposed = true;
      queuedHref = null;
      for (const resolve of idleWaiters) resolve();
      idleWaiters.clear();
    },
  };
}

function stateKey(state: NativeChromeState): string {
  return [
    state.visible ? "1" : "0",
    state.selectedTab,
    state.minimized ? "1" : "0",
    state.appearance,
    state.compact ? "1" : "0",
  ].join("|");
}

/**
 * Serializes native commands while retaining only the newest state that has
 * not started. This keeps scroll-driven minimize changes from overtaking a
 * route or appearance update across the async Tauri boundary.
 */
export function createNativeChromeStateSync(
  onStatus?: (status: NativeChromeStatus) => void,
  transport: NativeChromeTransport = DEFAULT_TRANSPORT,
): NativeChromeStateSync {
  let disposed = false;
  let inFlight = false;
  let inFlightKey: string | null = null;
  let queued: NativeChromeState | null = null;
  let queuedKey: string | null = null;
  let lastCompletedKey: string | null = null;
  let retryKey: string | null = null;
  const idleWaiters = new Set<() => void>();

  const resolveIdleWaiters = (): void => {
    if (inFlight || queued !== null) return;
    for (const resolve of idleWaiters) resolve();
    idleWaiters.clear();
  };

  const drain = async (): Promise<void> => {
    if (disposed || inFlight) return;
    inFlight = true;
    try {
      while (!disposed && queued !== null) {
        const next = queued;
        const nextKey = queuedKey!;
        queued = null;
        queuedKey = null;
        inFlightKey = nextKey;

        const application = await applyNativeChromeState(
          next,
          transport,
        );
        const { status } = application;
        if (application.completed) {
          lastCompletedKey = nextKey;
          retryKey = null;
        } else {
          // One immediate retry covers a transient bridge failure. A second
          // failure remains uncached so the next reactive update can retry,
          // while a genuinely unsupported platform cannot spin forever.
          lastCompletedKey = null;
          if (queued === null && retryKey !== nextKey) {
            retryKey = nextKey;
            queued = next;
            queuedKey = nextKey;
          } else if (retryKey === nextKey) {
            retryKey = null;
          }
        }
        inFlightKey = null;
        if (!disposed) {
          try {
            onStatus?.(status);
          } catch {
            // A consumer callback cannot break future native synchronization.
          }
        }
      }
    } finally {
      inFlight = false;
      inFlightKey = null;
      if (!disposed && queued !== null) {
        void drain();
      } else {
        resolveIdleWaiters();
      }
    }
  };

  return {
    update(state: NativeChromeState): boolean {
      if (disposed) return false;
      const normalized = normalizeNativeChromeState(state);
      if (normalized === null) return false;
      const key = stateKey(normalized);

      if (key === queuedKey) return false;
      if (key === inFlightKey) {
        if (queued === null) return false;
        // The latest request returned to the state already being applied, so
        // discard the now-stale successor.
        queued = null;
        queuedKey = null;
        return true;
      }
      if (
        !inFlight &&
        queued === null &&
        key === lastCompletedKey
      ) {
        return false;
      }

      queued = normalized;
      queuedKey = key;
      void drain();
      return true;
    },

    flush(): Promise<void> {
      if (disposed || (!inFlight && queued === null)) {
        return Promise.resolve();
      }
      return new Promise<void>((resolve) => {
        idleWaiters.add(resolve);
      });
    },

    dispose(): void {
      if (disposed) return;
      disposed = true;
      queued = null;
      queuedKey = null;
      for (const resolve of idleWaiters) resolve();
      idleWaiters.clear();
    },
  };
}
