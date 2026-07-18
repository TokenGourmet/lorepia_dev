import {
  ISOLATION_LIMITS,
  ISOLATION_PROTOCOL_VERSION,
  parseIsolationRequest,
  safeIsolationRequestIdHint,
  validateJsonValue,
  type IsolationErrorCode,
  type IsolationErrorResponse,
  type IsolationResponse,
  type JsonValue,
} from "./isolation-protocol";
import { createFixedWindowAdmission } from "./fixed-window-admission";

export const HOST_BROKER_MAX_IN_FLIGHT = 8;
export const HOST_BROKER_MAX_ATTEMPTS_PER_WINDOW = 64;
export const HOST_BROKER_ATTEMPT_WINDOW_MS = 1_000;

export type HostBrokerForwarderOptions = Readonly<{
  maxInFlight?: number;
  maxAttemptsPerWindow?: number;
  attemptWindowMs?: number;
  clock?: () => number;
}>;

export type NativeBrokerForwardRequest = Readonly<{
  requestId: string;
  method: string;
  requestJson: string;
}>;

export type NativeBrokerForwardOutcome =
  | { ok: true; result: JsonValue }
  | {
      ok: false;
      error: { code: IsolationErrorCode; message: string };
    };

export type NativeBrokerForward = (
  request: NativeBrokerForwardRequest,
) => Promise<NativeBrokerForwardOutcome>;

export type HostBrokerForwarder = Readonly<{
  forward: (
    value: unknown,
    sessionNonce: string,
    invokeNative: NativeBrokerForward,
  ) => Promise<IsolationResponse | null>;
  readonly inFlightCount: number;
}>;

const UTF8 = new TextEncoder();
const EMPTY_PAYLOAD_METHODS = new Set([
  "state.read",
  "probe.increment",
  "secret.read",
]);
const NETWORK_URL_BYTES_MAX = 2_048;

function boundedMessage(message: string): string {
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
    error: { code, message: boundedMessage(message) },
  };
}

function hasExactKeys(
  value: Record<string, JsonValue>,
  expected: readonly string[],
): boolean {
  const keys = Object.keys(value);
  return (
    keys.length === expected.length &&
    expected.every((key) => Object.prototype.hasOwnProperty.call(value, key))
  );
}

function payloadProblem(
  method: string,
  payload: Record<string, JsonValue>,
): string | null {
  if (EMPTY_PAYLOAD_METHODS.has(method)) {
    return hasExactKeys(payload, [])
      ? null
      : "The method requires an empty payload.";
  }

  if (method === "render.sanitize") {
    return hasExactKeys(payload, ["html"]) && typeof payload.html === "string"
      ? null
      : "render.sanitize requires exactly one string html field.";
  }

  if (method === "network.fetch") {
    if (!hasExactKeys(payload, ["url"]) || typeof payload.url !== "string") {
      return "network.fetch requires exactly one string url field.";
    }
    return UTF8.encode(payload.url).byteLength <= NETWORK_URL_BYTES_MAX
      ? null
      : "The network URL exceeds the encoded byte limit.";
  }

  return "UNKNOWN_METHOD";
}

export function createHostBrokerForwarder(
  options: number | HostBrokerForwarderOptions = {},
): HostBrokerForwarder {
  const maxInFlight =
    typeof options === "number"
      ? options
      : (options.maxInFlight ?? HOST_BROKER_MAX_IN_FLIGHT);
  const maxAttemptsPerWindow =
    typeof options === "number"
      ? HOST_BROKER_MAX_ATTEMPTS_PER_WINDOW
      : (options.maxAttemptsPerWindow ?? HOST_BROKER_MAX_ATTEMPTS_PER_WINDOW);
  const attemptWindowMs =
    typeof options === "number"
      ? HOST_BROKER_ATTEMPT_WINDOW_MS
      : (options.attemptWindowMs ?? HOST_BROKER_ATTEMPT_WINDOW_MS);
  const clock =
    typeof options === "number" ? Date.now : (options.clock ?? Date.now);

  if (!Number.isSafeInteger(maxInFlight) || maxInFlight <= 0) {
    throw new RangeError("maxInFlight must be a positive safe integer");
  }
  if (!Number.isSafeInteger(maxAttemptsPerWindow) || maxAttemptsPerWindow <= 0) {
    throw new RangeError("maxAttemptsPerWindow must be a positive safe integer");
  }
  if (!Number.isSafeInteger(attemptWindowMs) || attemptWindowMs <= 0) {
    throw new RangeError("attemptWindowMs must be a positive safe integer");
  }

  let inFlightCount = 0;
  const attemptAdmission = createFixedWindowAdmission({
    maxAttempts: maxAttemptsPerWindow,
    windowMs: attemptWindowMs,
    clock,
  });

  return Object.freeze({
    get inFlightCount() {
      return inFlightCount;
    },

    async forward(value, sessionNonce, invokeNative) {
      const requestIdHint = safeIsolationRequestIdHint(value);
      if (!attemptAdmission.consume()) {
        return errorResponse(
          sessionNonce,
          requestIdHint,
          "RATE_LIMITED",
          "The global native broker attempt limit was reached.",
        );
      }

      const parsed = parseIsolationRequest(value);
      if (!parsed.ok) {
        return errorResponse(
          sessionNonce,
          parsed.requestId,
          parsed.error.code,
          parsed.error.message,
        );
      }

      const request = parsed.request;
      if (request.sessionNonce !== sessionNonce) return null;

      const problem = payloadProblem(request.method, request.payload);
      if (problem === "UNKNOWN_METHOD") {
        return errorResponse(
          sessionNonce,
          request.requestId,
          "UNKNOWN_METHOD",
          "No native broker method is registered for this request.",
        );
      }
      if (problem !== null) {
        return errorResponse(
          sessionNonce,
          request.requestId,
          "INVALID_PAYLOAD",
          problem,
        );
      }

      let requestJson: string;
      try {
        requestJson = JSON.stringify({
          request_id: request.requestId,
          method: request.method,
          payload: request.payload,
        });
      } catch {
        return errorResponse(
          sessionNonce,
          request.requestId,
          "MALFORMED_REQUEST",
          "The native broker request could not be encoded.",
        );
      }
      if (UTF8.encode(requestJson).byteLength > ISOLATION_LIMITS.requestBytesMax) {
        return errorResponse(
          sessionNonce,
          request.requestId,
          "MALFORMED_REQUEST",
          "The encoded native broker request exceeds the byte limit.",
        );
      }

      if (inFlightCount >= maxInFlight) {
        return errorResponse(
          sessionNonce,
          request.requestId,
          "RATE_LIMITED",
          "The global native broker in-flight limit was reached.",
        );
      }

      // Admission is synchronous and this counter belongs to the trusted host,
      // not to an iframe generation. Reloading the plugin cannot reset it.
      inFlightCount += 1;
      try {
        let outcome: NativeBrokerForwardOutcome;
        try {
          outcome = await invokeNative({
            requestId: request.requestId,
            method: request.method,
            requestJson,
          });
        } catch {
          outcome = {
            ok: false,
            error: {
              code: "HANDLER_FAILED",
              message: "The native host broker failed without exposing details.",
            },
          };
        }

        if (!outcome.ok) {
          return errorResponse(
            sessionNonce,
            request.requestId,
            outcome.error.code,
            outcome.error.message,
          );
        }

        const result = validateJsonValue(
          outcome.result,
          ISOLATION_LIMITS.resultBytesMax,
        );
        if (!result.ok) {
          return errorResponse(
            sessionNonce,
            request.requestId,
            "INVALID_HANDLER_RESULT",
            "The native broker returned an unsafe or oversized result.",
          );
        }

        return {
          version: ISOLATION_PROTOCOL_VERSION,
          type: "response",
          sessionNonce,
          requestId: request.requestId,
          ok: true,
          result: result.value,
        };
      } finally {
        inFlightCount -= 1;
      }
    },
  });
}
