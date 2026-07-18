export const ISOLATION_PROTOCOL_VERSION = 1 as const;
export const OPAQUE_SANDBOX_ORIGIN = "null" as const;

export const ISOLATION_LIMITS = Object.freeze({
  sessionNonceHexCharsMin: 32,
  sessionNonceHexCharsMax: 128,
  requestIdCharsMax: 64,
  methodBytesMax: 96,
  objectKeyBytesMax: 128,
  stringBytesMax: 16 * 1024,
  payloadBytesMax: 64 * 1024,
  requestBytesMax: 72 * 1024,
  resultBytesMax: 64 * 1024,
  containerEntriesMax: 1_024,
  totalEntriesMax: 4_096,
  totalNodesMax: 4_097,
  nestingDepthMax: 16,
  errorMessageCharsMax: 256,
});

export type JsonPrimitive = string | number | boolean | null;
export type JsonObject = { [key: string]: JsonValue };
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];

export type IsolationRequest = {
  version: typeof ISOLATION_PROTOCOL_VERSION;
  type: "request";
  sessionNonce: string;
  requestId: string;
  method: string;
  payload: JsonObject;
};

export type IsolationErrorCode =
  | "MALFORMED_REQUEST"
  | "REPLAYED_REQUEST"
  | "RATE_LIMITED"
  | "SESSION_EXHAUSTED"
  | "UNKNOWN_METHOD"
  | "PERMISSION_DENIED"
  | "NETWORK_DENIED"
  | "INVALID_PAYLOAD"
  | "HANDLER_FAILED"
  | "INVALID_HANDLER_RESULT";

export type IsolationSuccessResponse<TResult extends JsonValue = JsonValue> = {
  version: typeof ISOLATION_PROTOCOL_VERSION;
  type: "response";
  sessionNonce: string;
  requestId: string;
  ok: true;
  result: TResult;
};

export type IsolationErrorResponse = {
  version: typeof ISOLATION_PROTOCOL_VERSION;
  type: "response";
  sessionNonce: string;
  requestId: string | null;
  ok: false;
  error: {
    code: IsolationErrorCode;
    message: string;
  };
};

export type IsolationResponse<TResult extends JsonValue = JsonValue> =
  | IsolationSuccessResponse<TResult>
  | IsolationErrorResponse;

export type IsolationRequestParseResult =
  | { ok: true; request: IsolationRequest }
  | {
      ok: false;
      requestId: string | null;
      error: { code: "MALFORMED_REQUEST"; message: string };
    };

export type SafeJsonValidation<T extends JsonValue = JsonValue> =
  | { ok: true; value: T; bytes: number }
  | { ok: false; message: string };

const REQUEST_KEYS = [
  "version",
  "type",
  "sessionNonce",
  "requestId",
  "method",
  "payload",
] as const;
const DANGEROUS_KEYS = new Set(["__proto__", "prototype", "constructor"]);
const UTF8 = new TextEncoder();
const SESSION_NONCE_PATTERN = /^[0-9a-f]{32,128}$/;
const REQUEST_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;
const DOTTED_IDENTIFIER_PATTERN =
  /^[a-z][a-z0-9]*(?:\.[a-z][a-z0-9_-]*){1,7}$/;

function byteLength(value: string): number {
  return UTF8.encode(value).byteLength;
}

function malformed(
  message: string,
  requestId: string | null,
): IsolationRequestParseResult {
  return {
    ok: false,
    requestId,
    error: { code: "MALFORMED_REQUEST", message },
  };
}

function isOrdinaryObject(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  return Object.getPrototypeOf(value) === Object.prototype;
}

function ownDataValue(
  value: Record<string, unknown>,
  key: string,
): unknown {
  const descriptor = Object.getOwnPropertyDescriptor(value, key);
  if (
    descriptor === undefined ||
    !("value" in descriptor) ||
    descriptor.enumerable !== true
  ) {
    return undefined;
  }
  return descriptor.value;
}

function requestIdHint(value: unknown): string | null {
  try {
    if (!isOrdinaryObject(value)) return null;
    const requestId = ownDataValue(value, "requestId");
    return isValidRequestId(requestId) ? requestId : null;
  } catch {
    return null;
  }
}

export function safeIsolationRequestIdHint(value: unknown): string | null {
  return requestIdHint(value);
}

export function safeIsolationMessageTypeHint(value: unknown): string | null {
  try {
    if (!isOrdinaryObject(value)) return null;
    const messageType = ownDataValue(value, "type");
    return typeof messageType === "string" ? messageType : null;
  } catch {
    return null;
  }
}

function hasExactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
): boolean {
  const keys = Object.keys(value);
  return (
    keys.length === expected.length &&
    expected.every((key) => Object.prototype.hasOwnProperty.call(value, key))
  );
}

function inspectJsonValue(
  value: unknown,
  depth: number,
  seen: WeakSet<object>,
  budget: {
    entries: number;
    nodes: number;
    stringAndKeyBytes: number;
    byteLimit: number;
  },
): string | null {
  budget.nodes += 1;
  if (budget.nodes > ISOLATION_LIMITS.totalNodesMax) {
    return "The JSON value exceeds the total node limit.";
  }

  if (value === null || typeof value === "boolean") return null;

  if (typeof value === "string") {
    const bytes = byteLength(value);
    if (bytes > ISOLATION_LIMITS.stringBytesMax) {
      return "A string exceeds the protocol size limit.";
    }
    budget.stringAndKeyBytes += bytes;
    if (budget.stringAndKeyBytes > budget.byteLimit) {
      return "Strings and keys exceed the JSON byte limit.";
    }
    return null;
  }

  if (typeof value === "number") {
    return Number.isFinite(value) ? null : "JSON numbers must be finite.";
  }

  if (typeof value !== "object") {
    return "Only JSON values are allowed.";
  }

  if (depth > ISOLATION_LIMITS.nestingDepthMax) {
    return "The JSON value exceeds the nesting limit.";
  }

  if (seen.has(value)) return "Cyclic or aliased object graphs are not allowed.";
  seen.add(value);

  try {
    if (Array.isArray(value)) {
      if (Object.getPrototypeOf(value) !== Array.prototype) {
        return "Arrays must use the ordinary Array prototype.";
      }
      if (value.length > ISOLATION_LIMITS.containerEntriesMax) {
        return "An array exceeds the entry limit.";
      }
      budget.entries += value.length;
      if (budget.entries > ISOLATION_LIMITS.totalEntriesMax) {
        return "The JSON value exceeds the total entry limit.";
      }

      const keys = Reflect.ownKeys(value);
      if (keys.length !== value.length + 1 || !keys.includes("length")) {
        return "Arrays must be dense and cannot have custom properties.";
      }

      for (let index = 0; index < value.length; index += 1) {
        const descriptor = Object.getOwnPropertyDescriptor(value, String(index));
        if (
          descriptor === undefined ||
          !("value" in descriptor) ||
          descriptor.enumerable !== true
        ) {
          return "Arrays must contain only enumerable data elements.";
        }
        const problem = inspectJsonValue(
          descriptor.value,
          depth + 1,
          seen,
          budget,
        );
        if (problem !== null) return problem;
      }
      return null;
    }

    if (!isOrdinaryObject(value)) {
      return "Objects must use the ordinary Object prototype.";
    }

    const keys = Reflect.ownKeys(value);
    if (keys.length > ISOLATION_LIMITS.containerEntriesMax) {
      return "An object exceeds the entry limit.";
    }
    budget.entries += keys.length;
    if (budget.entries > ISOLATION_LIMITS.totalEntriesMax) {
      return "The JSON value exceeds the total entry limit.";
    }

    for (const key of keys) {
      if (typeof key !== "string") return "Symbol keys are not allowed.";
      if (DANGEROUS_KEYS.has(key)) {
        return `The reserved key ${key} is not allowed.`;
      }
      const keyBytes = byteLength(key);
      if (keyBytes > ISOLATION_LIMITS.objectKeyBytesMax) {
        return "An object key exceeds the size limit.";
      }
      budget.stringAndKeyBytes += keyBytes;
      if (budget.stringAndKeyBytes > budget.byteLimit) {
        return "Strings and keys exceed the JSON byte limit.";
      }

      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (
        descriptor === undefined ||
        !("value" in descriptor) ||
        descriptor.enumerable !== true
      ) {
        return "Objects may contain only enumerable data properties.";
      }
      const problem = inspectJsonValue(
        descriptor.value,
        depth + 1,
        seen,
        budget,
      );
      if (problem !== null) return problem;
    }
    return null;
  } catch {
    return "The JSON value could not be inspected safely.";
  }
}

function validateJson(
  value: unknown,
  byteLimit: number,
): SafeJsonValidation {
  const problem = inspectJsonValue(value, 0, new WeakSet<object>(), {
    entries: 0,
    nodes: 0,
    stringAndKeyBytes: 0,
    byteLimit,
  });
  if (problem !== null) return { ok: false, message: problem };

  try {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) {
      return { ok: false, message: "Only JSON values are allowed." };
    }
    const bytes = byteLength(encoded);
    if (bytes > byteLimit) {
      return { ok: false, message: "The JSON value exceeds the byte limit." };
    }
    return { ok: true, value: value as JsonValue, bytes };
  } catch {
    return { ok: false, message: "The JSON value could not be encoded." };
  }
}

export function isValidSessionNonce(value: unknown): value is string {
  return typeof value === "string" && SESSION_NONCE_PATTERN.test(value);
}

export function isValidRequestId(value: unknown): value is string {
  return typeof value === "string" && REQUEST_ID_PATTERN.test(value);
}

export function isValidDottedIdentifier(value: unknown): value is string {
  return (
    typeof value === "string" &&
    byteLength(value) <= ISOLATION_LIMITS.methodBytesMax &&
    DOTTED_IDENTIFIER_PATTERN.test(value)
  );
}

export function isReservedObjectKey(value: string): boolean {
  return DANGEROUS_KEYS.has(value);
}

export function validateJsonValue(
  value: unknown,
  byteLimit = ISOLATION_LIMITS.resultBytesMax,
): SafeJsonValidation {
  return validateJson(value, byteLimit);
}

export function validateJsonObject(
  value: unknown,
  byteLimit = ISOLATION_LIMITS.payloadBytesMax,
): SafeJsonValidation<JsonObject> {
  const validated = validateJson(value, byteLimit);
  if (!validated.ok) return validated;
  if (!isOrdinaryObject(validated.value)) {
    return { ok: false, message: "The payload must be an object." };
  }
  return {
    ok: true,
    value: validated.value as JsonObject,
    bytes: validated.bytes,
  };
}

export function parseIsolationRequest(
  value: unknown,
): IsolationRequestParseResult {
  const hint = requestIdHint(value);

  try {
    const envelope = validateJson(value, ISOLATION_LIMITS.requestBytesMax);
    if (!envelope.ok) return malformed(envelope.message, hint);
    if (!isOrdinaryObject(envelope.value)) {
      return malformed("The request must be an object.", hint);
    }
    if (!hasExactKeys(envelope.value, REQUEST_KEYS)) {
      return malformed("The request schema contains missing or unknown fields.", hint);
    }

    const version = ownDataValue(envelope.value, "version");
    const type = ownDataValue(envelope.value, "type");
    const sessionNonce = ownDataValue(envelope.value, "sessionNonce");
    const requestId = ownDataValue(envelope.value, "requestId");
    const method = ownDataValue(envelope.value, "method");
    const payload = ownDataValue(envelope.value, "payload");

    if (version !== ISOLATION_PROTOCOL_VERSION || type !== "request") {
      return malformed("The protocol version or message type is invalid.", hint);
    }
    if (!isValidSessionNonce(sessionNonce)) {
      return malformed("The session nonce must encode at least 128 bits.", hint);
    }
    if (!isValidRequestId(requestId)) {
      return malformed("The request id has an invalid format.", null);
    }
    if (!isValidDottedIdentifier(method)) {
      return malformed("The method has an invalid format.", requestId);
    }

    const checkedPayload = validateJsonObject(
      payload,
      ISOLATION_LIMITS.payloadBytesMax,
    );
    if (!checkedPayload.ok) {
      return malformed(checkedPayload.message, requestId);
    }

    return {
      ok: true,
      request: {
        version,
        type,
        sessionNonce,
        requestId,
        method,
        payload: checkedPayload.value,
      },
    };
  } catch {
    return malformed("The request could not be inspected safely.", hint);
  }
}
