import { describe, expect, it, vi } from "vitest";

import {
  createIsolationBroker,
  defineIsolationHandler,
  type IsolationBrokerOptions,
  type RegisteredIsolationHandler,
} from "./isolation-broker";
import {
  ISOLATION_LIMITS,
  ISOLATION_PROTOCOL_VERSION,
  type IsolationErrorResponse,
  type IsolationRequest,
} from "./isolation-protocol";

const sessionNonce = "0123456789abcdef0123456789abcdef";
const otherSessionNonce = "fedcba9876543210fedcba9876543210";
const source = {};

function echoHandler() {
  return defineIsolationHandler({
    permission: "conversation.read",
    payloadKeys: ["text"],
    parsePayload(payload) {
      return typeof payload.text === "string"
        ? { ok: true as const, value: { text: payload.text } }
        : { ok: false as const, message: "text must be a string" };
    },
    handle(payload, context) {
      return { echo: payload.text, method: context.method };
    },
  });
}

function networkHandler() {
  return defineIsolationHandler({
    permission: "network.fetch",
    network: true,
    payloadKeys: ["url"],
    parsePayload(payload) {
      return typeof payload.url === "string"
        ? { ok: true as const, value: payload.url }
        : { ok: false as const, message: "url must be a string" };
    },
    handle(url) {
      return { fetched: url };
    },
  });
}

function createBroker(
  overrides: Partial<IsolationBrokerOptions> = {},
  handlers: Record<string, RegisteredIsolationHandler> = {
    "conversation.echo": echoHandler(),
  },
) {
  return createIsolationBroker({
    expectedSource: source,
    sessionNonce,
    manifestPermissions: ["conversation.read"],
    approvedPermissions: ["conversation.read"],
    handlers,
    rateLimit: { maxRequests: 100, windowMs: 1_000 },
    clock: () => 0,
    ...overrides,
  });
}

function request(
  requestId: string,
  overrides: Partial<IsolationRequest> = {},
): IsolationRequest {
  return {
    version: ISOLATION_PROTOCOL_VERSION,
    type: "request",
    sessionNonce,
    requestId,
    method: "conversation.echo",
    payload: { text: "hello" },
    ...overrides,
  };
}

function event(data: unknown, overrides: { source?: unknown; origin?: string } = {}) {
  return {
    source: overrides.source ?? source,
    origin: overrides.origin ?? "null",
    data,
  };
}

function expectError(
  response: Awaited<ReturnType<ReturnType<typeof createBroker>["handleEvent"]>>,
  code: IsolationErrorResponse["error"]["code"],
): IsolationErrorResponse {
  expect(response).not.toBeNull();
  expect(response?.ok).toBe(false);
  if (response === null || response.ok) throw new Error("expected broker error");
  expect(response.error.code).toBe(code);
  expect(Object.keys(response).sort()).toEqual(
    ["error", "ok", "requestId", "sessionNonce", "type", "version"].sort(),
  );
  expect(Object.keys(response.error).sort()).toEqual(["code", "message"]);
  return response;
}

describe("iframe identity and session boundary", () => {
  it("accepts only the exact iframe source identity at the opaque null origin", async () => {
    const broker = createBroker();

    await expect(
      broker.handleEvent(event(request("wrong-source"), { source: {} })),
    ).resolves.toBeNull();
    await expect(
      broker.handleEvent(
        event(request("wrong-origin"), { origin: "https://plugin.example" }),
      ),
    ).resolves.toBeNull();

    const response = await broker.handleEvent(event(request("exact-boundary")));
    expect(response).toMatchObject({ ok: true, requestId: "exact-boundary" });
  });

  it("silently ignores a well-formed request from another session", async () => {
    const broker = createBroker();

    await expect(
      broker.handleEvent(
        event(request("other-session", { sessionNonce: otherSessionNonce })),
      ),
    ).resolves.toBeNull();
  });

  it("requires a nonce representation of at least 128 bits", async () => {
    expect(() =>
      createBroker({ sessionNonce: "0123456789abcdef" }),
    ).toThrow(/32-128/);

    const response = await createBroker().handleEvent(
      event(request("short-nonce", { sessionNonce: "0123456789abcdef" })),
    );
    expectError(response, "MALFORMED_REQUEST");
  });
});

describe("strict request and payload schemas", () => {
  it("rejects missing and unknown envelope fields", async () => {
    const broker = createBroker();
    const withUnknown = { ...request("unknown-envelope"), surprise: true };
    const missing = { ...request("missing-envelope") } as Record<string, unknown>;
    delete missing.method;

    expectError(
      await broker.handleEvent(event(withUnknown)),
      "MALFORMED_REQUEST",
    );
    expectError(await broker.handleEvent(event(missing)), "MALFORMED_REQUEST");
  });

  it("rejects invalid request ids, methods, and bounded strings", async () => {
    const broker = createBroker();

    expectError(
      await broker.handleEvent(event(request("contains space"))),
      "MALFORMED_REQUEST",
    );
    expectError(
      await broker.handleEvent(
        event(request("bad-method", { method: "Not A Method" })),
      ),
      "MALFORMED_REQUEST",
    );
    expectError(
      await broker.handleEvent(
        event(
          request("oversized-string", {
            payload: { text: "x".repeat(ISOLATION_LIMITS.stringBytesMax + 1) },
          }),
        ),
      ),
      "MALFORMED_REQUEST",
    );
  });

  it("enforces each handler's exact payload keys before parsing", async () => {
    const broker = createBroker();

    expectError(
      await broker.handleEvent(
        event(
          request("payload-unknown", {
            payload: { text: "hello", extra: true },
          }),
        ),
      ),
      "INVALID_PAYLOAD",
    );
    expectError(
      await broker.handleEvent(
        event(request("payload-missing", { payload: {} })),
      ),
      "INVALID_PAYLOAD",
    );
    expectError(
      await broker.handleEvent(
        event(request("payload-type", { payload: { text: 123 } })),
      ),
      "INVALID_PAYLOAD",
    );
  });

  it("rejects custom prototypes, accessors, and prototype-pollution keys", async () => {
    const broker = createBroker();
    const customPrototype = Object.assign(Object.create({ inherited: true }),
      request("custom-prototype"),
    );
    const accessor = request("accessor");
    Object.defineProperty(accessor.payload, "text", {
      enumerable: true,
      get: () => "do not execute",
    });
    const reserved = request("reserved", {
      payload: JSON.parse('{"text":"hello","__proto__":{"admin":true}}'),
    });

    expectError(
      await broker.handleEvent(event(customPrototype)),
      "MALFORMED_REQUEST",
    );
    expectError(await broker.handleEvent(event(accessor)), "MALFORMED_REQUEST");
    expectError(await broker.handleEvent(event(reserved)), "MALFORMED_REQUEST");
  });
});

describe("permission and network policy", () => {
  it("requires the handler permission in both manifest and user approval", async () => {
    const declaredOnly = createBroker({ approvedPermissions: [] });
    const approvedOnly = createBroker({ manifestPermissions: [] });
    const both = createBroker();

    expectError(
      await declaredOnly.handleEvent(event(request("declared-only"))),
      "PERMISSION_DENIED",
    );
    expectError(
      await approvedOnly.handleEvent(event(request("approved-only"))),
      "PERMISSION_DENIED",
    );
    await expect(
      both.handleEvent(event(request("declared-and-approved"))),
    ).resolves.toMatchObject({ ok: true });
  });

  it("denies network by default after permission checks and allows it explicitly", async () => {
    const handlers = { "network.fetch": networkHandler() };
    const base = {
      manifestPermissions: ["network.fetch"],
      approvedPermissions: ["network.fetch"],
    } as const;
    const denied = createBroker(base, handlers);
    const allowed = createBroker({ ...base, networkPolicy: "allow" }, handlers);
    const networkRequest = request("network-default-deny", {
      method: "network.fetch",
      payload: { url: "https://example.invalid" },
    });

    expectError(
      await denied.handleEvent(event(networkRequest)),
      "NETWORK_DENIED",
    );
    await expect(
      allowed.handleEvent(
        event({ ...networkRequest, requestId: "network-explicit-allow" }),
      ),
    ).resolves.toMatchObject({
      ok: true,
      result: { fetched: "https://example.invalid" },
    });
  });
});

describe("replay and deterministic rate limits", () => {
  it("consumes a request id before async handler execution and rejects replay", async () => {
    const handle = vi.fn((text: string) => ({ text }));
    const handler = defineIsolationHandler({
      permission: "conversation.read",
      payloadKeys: ["text"],
      parsePayload: (payload) => ({ ok: true as const, value: String(payload.text) }),
      handle,
    });
    const broker = createBroker({}, { "conversation.echo": handler });
    const replayed = request("one-shot");

    await expect(broker.handleEvent(event(replayed))).resolves.toMatchObject({
      ok: true,
    });
    expectError(
      await broker.handleEvent(event(replayed)),
      "REPLAYED_REQUEST",
    );
    expect(handle).toHaveBeenCalledTimes(1);
  });

  it("uses an injected clock for a deterministic fixed window", async () => {
    let now = 0;
    const broker = createBroker({
      rateLimit: { maxRequests: 2, windowMs: 100 },
      clock: () => now,
    });

    await expect(
      broker.handleEvent(event(request("window-1"))),
    ).resolves.toMatchObject({ ok: true });
    await expect(
      broker.handleEvent(event(request("window-2"))),
    ).resolves.toMatchObject({ ok: true });
    expectError(
      await broker.handleEvent(event(request("window-3"))),
      "RATE_LIMITED",
    );

    now = 100;
    await expect(
      broker.handleEvent(event(request("next-window"))),
    ).resolves.toMatchObject({ ok: true });
  });

  it("keeps replay prevention bounded by closing an exhausted session", async () => {
    const broker = createBroker({ maxRequestHistory: 1 });
    await broker.handleEvent(event(request("history-1")));

    expectError(
      await broker.handleEvent(event(request("history-2"))),
      "SESSION_EXHAUSTED",
    );
  });
});

describe("handler outcomes", () => {
  it("returns an exact typed success response and injects immutable context", async () => {
    const observedContext: unknown[] = [];
    const handler = defineIsolationHandler({
      permission: "conversation.read",
      payloadKeys: ["text"],
      parsePayload: (payload) => ({ ok: true as const, value: payload.text }),
      handle(text, context) {
        observedContext.push(context);
        return { text };
      },
    });
    const broker = createBroker({}, { "conversation.echo": handler });

    const response = await broker.handleEvent(event(request("success")));
    expect(response).toEqual({
      version: ISOLATION_PROTOCOL_VERSION,
      type: "response",
      sessionNonce,
      requestId: "success",
      ok: true,
      result: { text: "hello" },
    });
    expect(observedContext).toEqual([
      {
        sessionNonce,
        requestId: "success",
        method: "conversation.echo",
        permission: "conversation.read",
      },
    ]);
    expect(Object.isFrozen(observedContext[0])).toBe(true);
  });

  it("maps handler exceptions to a generic structured failure", async () => {
    const handler = defineIsolationHandler({
      permission: "conversation.read",
      payloadKeys: ["text"],
      parsePayload: (payload) => ({ ok: true as const, value: payload.text }),
      handle() {
        throw new Error("secret backend detail");
      },
    });
    const broker = createBroker({}, { "conversation.echo": handler });

    const response = expectError(
      await broker.handleEvent(event(request("handler-failure"))),
      "HANDLER_FAILED",
    );
    expect(response.error.message).not.toContain("secret backend detail");
  });

  it("rejects prototype-bearing or non-JSON handler results", async () => {
    const handler = defineIsolationHandler({
      permission: "conversation.read",
      payloadKeys: ["text"],
      parsePayload: (payload) => ({ ok: true as const, value: payload.text }),
      handle() {
        return Object.assign(Object.create({ inherited: true }), { safe: true });
      },
    });
    const broker = createBroker({}, { "conversation.echo": handler });

    expectError(
      await broker.handleEvent(event(request("unsafe-result"))),
      "INVALID_HANDLER_RESULT",
    );
  });
});
