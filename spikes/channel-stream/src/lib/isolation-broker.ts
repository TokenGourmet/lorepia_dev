import {
  ISOLATION_LIMITS,
  ISOLATION_PROTOCOL_VERSION,
  OPAQUE_SANDBOX_ORIGIN,
  isReservedObjectKey,
  isValidDottedIdentifier,
  isValidSessionNonce,
  parseIsolationRequest,
  validateJsonValue,
  type IsolationErrorCode,
  type IsolationErrorResponse,
  type IsolationResponse,
  type IsolationSuccessResponse,
  type JsonObject,
  type JsonValue,
} from "./isolation-protocol";

export type IsolationMessageEvent = {
  source: unknown;
  origin: string;
  data: unknown;
};

export type IsolationHandlerContext = Readonly<{
  sessionNonce: string;
  requestId: string;
  method: string;
  permission: string;
}>;

export type PayloadParseResult<TPayload> =
  | { ok: true; value: TPayload }
  | { ok: false; message: string };

export type IsolationHandlerDefinition<
  TPayload,
  TResult extends JsonValue,
> = {
  permission: string;
  network?: boolean;
  payloadKeys: readonly string[];
  parsePayload: (payload: JsonObject) => PayloadParseResult<TPayload>;
  handle: (
    payload: TPayload,
    context: IsolationHandlerContext,
  ) => TResult | Promise<TResult>;
};

export type RegisteredIsolationHandler = Readonly<{
  permission: string;
  network: boolean;
  payloadKeys: readonly string[];
  parsePayload: (
    payload: JsonObject,
  ) => PayloadParseResult<unknown>;
  handle: (
    payload: unknown,
    context: IsolationHandlerContext,
  ) => Promise<JsonValue>;
}>;

export type IsolationRateLimit = Readonly<{
  maxRequests: number;
  windowMs: number;
}>;

export type IsolationBrokerOptions = Readonly<{
  expectedSource: object;
  sessionNonce: string;
  manifestPermissions: readonly string[];
  approvedPermissions: readonly string[];
  handlers: Readonly<Record<string, RegisteredIsolationHandler>>;
  networkPolicy?: "deny" | "allow";
  rateLimit?: IsolationRateLimit;
  clock?: () => number;
  maxRequestHistory?: number;
}>;

export type IsolationBroker = Readonly<{
  handleEvent: (event: IsolationMessageEvent) => Promise<IsolationResponse | null>;
}>;

const DEFAULT_RATE_LIMIT: IsolationRateLimit = Object.freeze({
  maxRequests: 64,
  windowMs: 1_000,
});
const DEFAULT_MAX_REQUEST_HISTORY = 4_096;
const MAX_REQUEST_HISTORY = 65_536;

function configurationError(message: string): never {
  throw new Error(`Invalid isolation broker configuration: ${message}`);
}

function validatePositiveSafeInteger(value: number, name: string): void {
  if (!Number.isSafeInteger(value) || value <= 0) {
    configurationError(`${name} must be a positive safe integer.`);
  }
}

function validatePermissionList(
  permissions: readonly string[],
  name: string,
): Set<string> {
  const result = new Set<string>();
  for (const permission of permissions) {
    if (!isValidDottedIdentifier(permission)) {
      configurationError(`${name} contains an invalid permission.`);
    }
    result.add(permission);
  }
  return result;
}

function validateHandler(
  method: string,
  handler: RegisteredIsolationHandler,
): void {
  if (!isValidDottedIdentifier(method)) {
    configurationError(`handler method ${method} is invalid.`);
  }
  if (!isValidDottedIdentifier(handler.permission)) {
    configurationError(`handler ${method} has an invalid permission.`);
  }
  if (!Array.isArray(handler.payloadKeys)) {
    configurationError(`handler ${method} must declare payload keys.`);
  }

  const uniqueKeys = new Set<string>();
  for (const key of handler.payloadKeys) {
    if (
      typeof key !== "string" ||
      key.length === 0 ||
      new TextEncoder().encode(key).byteLength >
        ISOLATION_LIMITS.objectKeyBytesMax ||
      isReservedObjectKey(key) ||
      uniqueKeys.has(key)
    ) {
      configurationError(`handler ${method} has invalid payload keys.`);
    }
    uniqueKeys.add(key);
  }
}

function exactPayloadKeys(
  payload: JsonObject,
  expected: readonly string[],
): boolean {
  const keys = Object.keys(payload);
  return (
    keys.length === expected.length &&
    expected.every((key) => Object.prototype.hasOwnProperty.call(payload, key))
  );
}

function boundedMessage(message: unknown, fallback: string): string {
  if (typeof message !== "string" || message.length === 0) return fallback;
  return message.slice(0, ISOLATION_LIMITS.errorMessageCharsMax);
}

function errorResponse(
  sessionNonce: string,
  requestId: string | null,
  code: IsolationErrorCode,
  message: string,
): IsolationErrorResponse {
  return {
    version: ISOLATION_PROTOCOL_VERSION,
    type: "response",
    sessionNonce,
    requestId,
    ok: false,
    error: { code, message: boundedMessage(message, code) },
  };
}

function successResponse(
  sessionNonce: string,
  requestId: string,
  result: JsonValue,
): IsolationSuccessResponse {
  return {
    version: ISOLATION_PROTOCOL_VERSION,
    type: "response",
    sessionNonce,
    requestId,
    ok: true,
    result,
  };
}

class FixedWindowRateLimiter {
  private windowStartedAt: number | null = null;
  private used = 0;

  constructor(private readonly limit: IsolationRateLimit) {}

  tryConsume(now: number): boolean {
    if (!Number.isFinite(now)) return false;

    if (
      this.windowStartedAt === null ||
      (now >= this.windowStartedAt &&
        now - this.windowStartedAt >= this.limit.windowMs)
    ) {
      this.windowStartedAt = now;
      this.used = 0;
    }

    if (this.used >= this.limit.maxRequests) return false;
    this.used += 1;
    return true;
  }
}

export function defineIsolationHandler<TPayload, TResult extends JsonValue>(
  definition: IsolationHandlerDefinition<TPayload, TResult>,
): RegisteredIsolationHandler {
  return Object.freeze({
    permission: definition.permission,
    network: definition.network === true,
    payloadKeys: Object.freeze([...definition.payloadKeys]),
    parsePayload: (payload: JsonObject): PayloadParseResult<unknown> =>
      definition.parsePayload(payload),
    handle: async (
      payload: unknown,
      context: IsolationHandlerContext,
    ): Promise<JsonValue> => definition.handle(payload as TPayload, context),
  });
}

export function createIsolationBroker(
  options: IsolationBrokerOptions,
): IsolationBroker {
  if (typeof options.expectedSource !== "object" || options.expectedSource === null) {
    configurationError("expectedSource must be a non-null object identity.");
  }
  if (!isValidSessionNonce(options.sessionNonce)) {
    configurationError("sessionNonce must be 32-128 lowercase hexadecimal characters.");
  }

  const declared = validatePermissionList(
    options.manifestPermissions,
    "manifestPermissions",
  );
  const approved = validatePermissionList(
    options.approvedPermissions,
    "approvedPermissions",
  );
  const handlers = new Map<string, RegisteredIsolationHandler>();
  for (const [method, handler] of Object.entries(options.handlers)) {
    validateHandler(method, handler);
    handlers.set(method, handler);
  }

  const rateLimit = options.rateLimit ?? DEFAULT_RATE_LIMIT;
  validatePositiveSafeInteger(rateLimit.maxRequests, "rateLimit.maxRequests");
  validatePositiveSafeInteger(rateLimit.windowMs, "rateLimit.windowMs");
  const limiter = new FixedWindowRateLimiter(rateLimit);
  const clock = options.clock ?? Date.now;

  const maxRequestHistory =
    options.maxRequestHistory ?? DEFAULT_MAX_REQUEST_HISTORY;
  validatePositiveSafeInteger(maxRequestHistory, "maxRequestHistory");
  if (maxRequestHistory > MAX_REQUEST_HISTORY) {
    configurationError(`maxRequestHistory cannot exceed ${MAX_REQUEST_HISTORY}.`);
  }

  const requestHistory = new Set<string>();
  const networkPolicy = options.networkPolicy ?? "deny";
  if (networkPolicy !== "deny" && networkPolicy !== "allow") {
    configurationError("networkPolicy must be deny or allow.");
  }

  return Object.freeze({
    handleEvent: async (
      event: IsolationMessageEvent,
    ): Promise<IsolationResponse | null> => {
      if (
        event.source !== options.expectedSource ||
        event.origin !== OPAQUE_SANDBOX_ORIGIN
      ) {
        return null;
      }

      const parsed = parseIsolationRequest(event.data);
      if (!parsed.ok) {
        return errorResponse(
          options.sessionNonce,
          parsed.requestId,
          parsed.error.code,
          parsed.error.message,
        );
      }

      const request = parsed.request;
      if (request.sessionNonce !== options.sessionNonce) {
        return null;
      }

      if (requestHistory.has(request.requestId)) {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "REPLAYED_REQUEST",
          "The request id was already consumed in this session.",
        );
      }
      if (requestHistory.size >= maxRequestHistory) {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "SESSION_EXHAUSTED",
          "The bounded request history is exhausted; create a new session.",
        );
      }
      requestHistory.add(request.requestId);

      let now: number;
      try {
        now = clock();
      } catch {
        now = Number.NaN;
      }
      if (!limiter.tryConsume(now)) {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "RATE_LIMITED",
          "The session request rate limit was exceeded.",
        );
      }

      const handler = handlers.get(request.method);
      if (handler === undefined) {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "UNKNOWN_METHOD",
          "No handler is registered for the requested method.",
        );
      }

      if (
        !declared.has(handler.permission) ||
        !approved.has(handler.permission)
      ) {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "PERMISSION_DENIED",
          "The permission is not both declared and approved.",
        );
      }

      if (handler.network && networkPolicy !== "allow") {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "NETWORK_DENIED",
          "Network access is denied by the session policy.",
        );
      }

      if (!exactPayloadKeys(request.payload, handler.payloadKeys)) {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "INVALID_PAYLOAD",
          "The payload schema contains missing or unknown fields.",
        );
      }

      let payload: unknown;
      try {
        const payloadResult = handler.parsePayload(request.payload);
        if (!payloadResult.ok) {
          return errorResponse(
            options.sessionNonce,
            request.requestId,
            "INVALID_PAYLOAD",
            boundedMessage(payloadResult.message, "The payload is invalid."),
          );
        }
        payload = payloadResult.value;
      } catch {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "INVALID_PAYLOAD",
          "The payload parser failed.",
        );
      }

      const context: IsolationHandlerContext = Object.freeze({
        sessionNonce: options.sessionNonce,
        requestId: request.requestId,
        method: request.method,
        permission: handler.permission,
      });

      let result: JsonValue;
      try {
        result = await handler.handle(payload, context);
      } catch {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "HANDLER_FAILED",
          "The handler failed without exposing internal details.",
        );
      }

      const validatedResult = validateJsonValue(
        result,
        ISOLATION_LIMITS.resultBytesMax,
      );
      if (!validatedResult.ok) {
        return errorResponse(
          options.sessionNonce,
          request.requestId,
          "INVALID_HANDLER_RESULT",
          "The handler returned an unsafe or oversized result.",
        );
      }

      return successResponse(
        options.sessionNonce,
        request.requestId,
        validatedResult.value,
      );
    },
  });
}
