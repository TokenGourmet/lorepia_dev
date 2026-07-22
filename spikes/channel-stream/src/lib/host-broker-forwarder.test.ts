import { describe, expect, it, vi } from "vitest";

import {
  createHostBrokerForwarder,
  type NativeBrokerForward,
} from "./host-broker-forwarder";
import {
  ISOLATION_LIMITS,
  ISOLATION_PROTOCOL_VERSION,
  safeIsolationMessageTypeHint,
  type IsolationRequest,
} from "./isolation-protocol";

const nonce = "0123456789abcdef0123456789abcdef";
const nextNonce = "fedcba9876543210fedcba9876543210";

function request(
  requestId: string,
  overrides: Partial<IsolationRequest> = {},
): IsolationRequest {
  return {
    version: ISOLATION_PROTOCOL_VERSION,
    type: "request",
    sessionNonce: nonce,
    requestId,
    method: "state.read",
    payload: {},
    ...overrides,
  };
}

function errorCode(value: unknown): string | null {
  return typeof value === "object" &&
    value !== null &&
    "ok" in value &&
    value.ok === false &&
    "error" in value &&
    typeof value.error === "object" &&
    value.error !== null &&
    "code" in value.error &&
    typeof value.error.code === "string"
    ? value.error.code
    : null;
}

describe("host broker forwarding boundary", () => {
  it("encodes only the validated native request schema", async () => {
    const invokeNative = vi.fn<NativeBrokerForward>(async (native) => {
      expect(JSON.parse(native.requestJson)).toEqual({
        request_id: "valid-1",
        method: "render.sanitize",
        payload: { html: "<p>safe</p>" },
      });
      return { ok: true, result: { type: "render_sanitize", html: "<p>safe</p>" } };
    });

    await expect(
      createHostBrokerForwarder().forward(
        request("valid-1", {
          method: "render.sanitize",
          payload: { html: "<p>safe</p>" },
        }),
        nonce,
        invokeNative,
      ),
    ).resolves.toMatchObject({ ok: true, requestId: "valid-1" });
    expect(invokeNative).toHaveBeenCalledTimes(1);
  });

  it.each([
    [
      "unknown envelope",
      { ...request("bad-envelope"), extra: true },
      "MALFORMED_REQUEST",
    ],
    ["invalid id", request("contains space"), "MALFORMED_REQUEST"],
    [
      "method encoded-byte limit",
      request("long-method", { method: `scope.${"a".repeat(96)}` }),
      "MALFORMED_REQUEST",
    ],
    [
      "unknown method",
      request("unknown-method", { method: "unknown.method" }),
      "UNKNOWN_METHOD",
    ],
    [
      "unknown payload key",
      request("unknown-key", { payload: { unexpected: true } }),
      "INVALID_PAYLOAD",
    ],
    [
      "encoded string limit",
      request("long-string", {
        method: "render.sanitize",
        payload: { html: "가".repeat(ISOLATION_LIMITS.stringBytesMax) },
      }),
      "MALFORMED_REQUEST",
    ],
    [
      "encoded payload byte limit",
      request("large-payload", {
        payload: {
          chunks: Array.from({ length: 5 }, () => "x".repeat(14 * 1_024)),
        },
      }),
      "MALFORMED_REQUEST",
    ],
    [
      "entry limit",
      request("many-entries", {
        payload: Object.fromEntries(
          Array.from(
            { length: ISOLATION_LIMITS.containerEntriesMax + 1 },
            (_, index) => [`k${index}`, null],
          ),
        ),
      }),
      "MALFORMED_REQUEST",
    ],
    [
      "depth limit",
      request("deep", {
        payload: JSON.parse(
          `${'{"x":'.repeat(ISOLATION_LIMITS.nestingDepthMax + 2)}null${"}".repeat(ISOLATION_LIMITS.nestingDepthMax + 2)}`,
        ),
      }),
      "MALFORMED_REQUEST",
    ],
    [
      "global entry and node budget",
      request("global-budget", {
        payload: {
          branches: Array.from({ length: 5 }, () => Array(900).fill(null)),
        },
      }),
      "MALFORMED_REQUEST",
    ],
  ])("rejects %s before native invocation", async (_label, value, expectedCode) => {
    const invokeNative = vi.fn<NativeBrokerForward>();
    const response = await createHostBrokerForwarder().forward(
      value,
      nonce,
      invokeNative,
    );

    expect(response?.ok).toBe(false);
    expect(errorCode(response)).toBe(expectedCode);
    expect(invokeNative).not.toHaveBeenCalled();
  });

  it("shares one synchronous in-flight cap across iframe generations", async () => {
    const resolvers: Array<() => void> = [];
    const invokeNative = vi.fn<NativeBrokerForward>(
      () =>
        new Promise((resolve) => {
          resolvers.push(() => resolve({ ok: true, result: { state: "ready" } }));
        }),
    );
    const forwarder = createHostBrokerForwarder(2);

    const first = forwarder.forward(request("old-1"), nonce, invokeNative);
    const second = forwarder.forward(request("old-2"), nonce, invokeNative);
    const third = await forwarder.forward(
      request("new-1", { sessionNonce: nextNonce }),
      nextNonce,
      invokeNative,
    );

    expect(errorCode(third)).toBe("RATE_LIMITED");
    expect(invokeNative).toHaveBeenCalledTimes(2);
    expect(forwarder.inFlightCount).toBe(2);

    for (const resolve of resolvers) resolve();
    await Promise.all([first, second]);
    expect(forwarder.inFlightCount).toBe(0);
  });

  it("rejects cumulative string bytes before allocating a full JSON encoding", async () => {
    const invokeNative = vi.fn<NativeBrokerForward>();
    const stringify = vi.spyOn(JSON, "stringify");
    try {
      const response = await createHostBrokerForwarder().forward(
        request("cumulative-strings", {
          payload: {
            chunks: Array.from({ length: 6 }, () => "x".repeat(13 * 1_024)),
          },
        }),
        nonce,
        invokeNative,
      );

      expect(errorCode(response)).toBe("MALFORMED_REQUEST");
      expect(stringify).not.toHaveBeenCalled();
      expect(invokeNative).not.toHaveBeenCalled();
    } finally {
      stringify.mockRestore();
    }
  });

  it("rejects cumulative key bytes before allocating a full JSON encoding", async () => {
    const invokeNative = vi.fn<NativeBrokerForward>();
    const stringify = vi.spyOn(JSON, "stringify");
    const payload = Object.fromEntries(
      Array.from({ length: 650 }, (_, index) => [
        `k${index.toString().padStart(4, "0")}${"x".repeat(114)}`,
        null,
      ]),
    );
    try {
      const response = await createHostBrokerForwarder().forward(
        request("cumulative-keys", { payload }),
        nonce,
        invokeNative,
      );

      expect(errorCode(response)).toBe("MALFORMED_REQUEST");
      expect(stringify).not.toHaveBeenCalled();
      expect(invokeNative).not.toHaveBeenCalled();
    } finally {
      stringify.mockRestore();
    }
  });

  it("applies an O(1) attempt quota before parsing attacker data", async () => {
    let now = 0;
    let ownKeysCalls = 0;
    const forwarder = createHostBrokerForwarder({
      maxAttemptsPerWindow: 1,
      attemptWindowMs: 100,
      clock: () => now,
    });
    const invokeNative = vi.fn<NativeBrokerForward>(async () => ({
      ok: true,
      result: { state: "ready" },
    }));

    await forwarder.forward(request("allowed"), nonce, invokeNative);
    const capped = new Proxy(request("quota-hint"), {
      ownKeys(target) {
        ownKeysCalls += 1;
        return Reflect.ownKeys(target);
      },
    });
    const response = await forwarder.forward(capped, nonce, invokeNative);

    expect(errorCode(response)).toBe("RATE_LIMITED");
    expect(response).toMatchObject({ requestId: "quota-hint" });
    expect(ownKeysCalls).toBe(0);
    expect(invokeNative).toHaveBeenCalledTimes(1);

    now = 100;
    await expect(
      forwarder.forward(request("new-window"), nonce, invokeNative),
    ).resolves.toMatchObject({ ok: true });
    expect(invokeNative).toHaveBeenCalledTimes(2);
  });

  it("uses a null request hint when a capped attempt has an unsafe ID", async () => {
    const forwarder = createHostBrokerForwarder({ maxAttemptsPerWindow: 1 });
    const invokeNative = vi.fn<NativeBrokerForward>(async () => ({
      ok: true,
      result: { state: "ready" },
    }));
    await forwarder.forward(request("allowed"), nonce, invokeNative);

    await expect(
      forwarder.forward(request("contains space"), nonce, invokeNative),
    ).resolves.toMatchObject({
      ok: false,
      requestId: null,
      error: { code: "RATE_LIMITED" },
    });
    expect(invokeNative).toHaveBeenCalledTimes(1);
  });

  it("routes an unknown message type without enumerating attacker keys", () => {
    let ownKeysCalls = 0;
    const value = new Proxy(
      { type: "attacker:unknown", padding: Array(10_000).fill(null) },
      {
        ownKeys(target) {
          ownKeysCalls += 1;
          return Reflect.ownKeys(target);
        },
      },
    );

    expect(safeIsolationMessageTypeHint(value)).toBe("attacker:unknown");
    expect(ownKeysCalls).toBe(0);
  });

  it("does not execute an accessor while reading a message type hint", () => {
    const getter = vi.fn(() => "request");
    const value = Object.defineProperty({}, "type", {
      enumerable: true,
      get: getter,
    });

    expect(safeIsolationMessageTypeHint(value)).toBeNull();
    expect(getter).not.toHaveBeenCalled();
  });
});
