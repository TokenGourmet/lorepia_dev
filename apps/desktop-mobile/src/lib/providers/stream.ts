import { Channel, invoke } from "@tauri-apps/api/core";

import type { ActiveProviderProfile } from "./active-profile.svelte";
import { buildFirstChatCommand } from "./first-chat-request";

const MAX_BUFFERED_EVENTS = 8;
const MAX_VISIBLE_OUTPUT_BYTES = 1024 * 1024;
const MAX_DELTA_BYTES = 512;
const MAX_IDENTIFIER_BYTES = 256;
export const FIRST_CHAT_RECOVERY_POLL_MS = 10_000;
const FIRST_CHAT_CANCEL_RECOVERY_MS = 1_000;

export const FIRST_CHAT_STREAM_UNKNOWN_ERROR =
  "응답을 받지 못했습니다. 잠시 후 다시 시도해 주세요.";
export const FIRST_CHAT_STREAM_PROTOCOL_ERROR =
  "응답 연결의 무결성을 확인하지 못해 요청을 중단했습니다.";

export type FirstChatTerminal = "completed" | "cancelled" | "failed";
export type FirstChatDeltaKind = "text" | "refusal";

export interface FirstChatStreamCallbacks {
  onStarted?(): void;
  onDelta(text: string, kind: FirstChatDeltaKind): void;
  onTerminal(terminal: FirstChatTerminal): void;
  onError(message: string): void;
}

export interface FirstChatStreamHandle {
  readonly done: Promise<FirstChatTerminal>;
  cancel(): Promise<void>;
}

export interface ProviderStreamOwnerReset {
  cancelled: number;
  terminalized: number;
}

export type CommandInvoker = (
  command: string,
  args: Record<string, unknown>,
) => Promise<unknown>;

export type StreamChannelFactory = (
  onMessage: (event: unknown) => void,
) => unknown;

export interface FirstChatStreamDependencies {
  invokeCommand: CommandInvoker;
  createChannel: StreamChannelFactory;
}

type StreamAuthorization = {
  requestId: string;
  controlToken: string;
};

type ProviderCommandError = {
  code: string;
  httpStatus: number | null;
};

type NativeTerminalEvent = Extract<
  NativeEvent,
  { type: "completed" | "cancelled" | "failed" }
>;

type NativeTerminalReceipt =
  | { type: "completed"; seq: number }
  | { type: "cancelled"; seq: number }
  | {
      type: "failed";
      seq: number;
      error: ProviderCommandError;
    };

type StreamSnapshot = {
  requestId: string;
  lastSentSeq: number;
  acknowledgedThrough: number | null;
  inFlight: number;
  cancelRequested: boolean;
  terminal: NativeTerminalReceipt | null;
};

type NativeEvent =
  | {
      type: "started";
      requestId: string;
      seq: number;
      maxInFlight: number;
    }
  | {
      type: "provider_response_id";
      requestId: string;
      seq: number;
      id: string;
    }
  | {
      type: "text_delta" | "reasoning_delta" | "refusal_delta";
      requestId: string;
      seq: number;
      text: string;
    }
  | {
      type: "usage";
      requestId: string;
      seq: number;
    }
  | {
      type: "completed";
      requestId: string;
      seq: number;
    }
  | {
      type: "cancelled";
      requestId: string;
      seq: number;
    }
  | {
      type: "failed";
      requestId: string;
      seq: number;
      error: ProviderCommandError;
    };

const defaultDependencies: FirstChatStreamDependencies = {
  invokeCommand(command, args) {
    return invoke<unknown>(command, args);
  },
  createChannel(onMessage) {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
): boolean {
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  return (
    actual.length === wanted.length &&
    actual.every((key, index) => key === wanted[index])
  );
}

function isSafeSequence(value: unknown): value is number {
  return Number.isSafeInteger(value) && typeof value === "number" && value >= 0;
}

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function isBoundedString(value: unknown, maxBytes: number): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    !value.includes("\0") &&
    utf8Length(value) <= maxBytes
  );
}

function parseStartResponse(value: unknown): StreamAuthorization {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["requestId", "controlToken"]) ||
    !isBoundedString(value.requestId, MAX_IDENTIFIER_BYTES) ||
    !/^[a-z0-9-]+$/.test(value.requestId) ||
    !isBoundedString(value.controlToken, 64) ||
    !/^[a-f0-9]+$/.test(value.controlToken)
  ) {
    throw new Error("INVALID_START_RESPONSE");
  }
  return {
    requestId: value.requestId,
    controlToken: value.controlToken,
  };
}

function parseOwnerResetResponse(value: unknown): ProviderStreamOwnerReset {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["cancelled", "terminalized"]) ||
    !isSafeSequence(value.cancelled) ||
    !isSafeSequence(value.terminalized) ||
    value.cancelled > 128 ||
    value.terminalized > 128
  ) {
    throw new Error("INVALID_STREAM_OWNER_RESET_RESPONSE");
  }
  return {
    cancelled: value.cancelled,
    terminalized: value.terminalized,
  };
}

export async function resetProviderStreamOwner(
  dependencies: Pick<FirstChatStreamDependencies, "invokeCommand"> =
    defaultDependencies,
): Promise<ProviderStreamOwnerReset> {
  const value = await dependencies.invokeCommand(
    "reset_provider_stream_owner",
    {},
  );
  return parseOwnerResetResponse(value);
}

function parseBaseEvent(
  value: Record<string, unknown>,
): { requestId: string; seq: number } {
  if (
    !isBoundedString(value.requestId, MAX_IDENTIFIER_BYTES) ||
    !isSafeSequence(value.seq)
  ) {
    throw new Error("INVALID_STREAM_EVENT");
  }
  return { requestId: value.requestId, seq: value.seq };
}

function parseProviderError(value: unknown): ProviderCommandError {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["code", "message", "httpStatus", "retriable"]) ||
    !isBoundedString(value.code, 64) ||
    typeof value.message !== "string" ||
    utf8Length(value.message) > 512 ||
    !(
      value.httpStatus === null ||
      (Number.isInteger(value.httpStatus) &&
        typeof value.httpStatus === "number" &&
        value.httpStatus >= 100 &&
        value.httpStatus <= 599)
    ) ||
    typeof value.retriable !== "boolean"
  ) {
    throw new Error("INVALID_PROVIDER_ERROR");
  }
  return { code: value.code, httpStatus: value.httpStatus };
}

function parseEvent(value: unknown): NativeEvent {
  if (!isRecord(value) || typeof value.type !== "string") {
    throw new Error("INVALID_STREAM_EVENT");
  }
  const base = parseBaseEvent(value);

  switch (value.type) {
    case "started":
      if (
        !hasExactKeys(value, ["type", "requestId", "seq", "maxInFlight"]) ||
        !isSafeSequence(value.maxInFlight) ||
        value.maxInFlight < 1 ||
        value.maxInFlight > 64
      ) {
        throw new Error("INVALID_STREAM_EVENT");
      }
      return { ...base, type: value.type, maxInFlight: value.maxInFlight };
    case "provider_response_id":
      if (
        !hasExactKeys(value, ["type", "requestId", "seq", "id"]) ||
        !isBoundedString(value.id, MAX_IDENTIFIER_BYTES)
      ) {
        throw new Error("INVALID_STREAM_EVENT");
      }
      return { ...base, type: value.type, id: value.id };
    case "text_delta":
    case "reasoning_delta":
    case "refusal_delta":
      if (
        !hasExactKeys(value, ["type", "requestId", "seq", "text"]) ||
        typeof value.text !== "string" ||
        value.text.includes("\0") ||
        utf8Length(value.text) > MAX_DELTA_BYTES
      ) {
        throw new Error("INVALID_STREAM_EVENT");
      }
      return { ...base, type: value.type, text: value.text };
    case "usage":
      if (
        !hasExactKeys(value, ["type", "requestId", "seq", "usage"]) ||
        !isRecord(value.usage)
      ) {
        throw new Error("INVALID_STREAM_EVENT");
      }
      return { ...base, type: value.type };
    case "completed":
      if (
        !hasExactKeys(value, [
          "type",
          "requestId",
          "seq",
          "reason",
          "usage",
        ])
      ) {
        throw new Error("INVALID_STREAM_EVENT");
      }
      return { ...base, type: value.type };
    case "cancelled":
      if (!hasExactKeys(value, ["type", "requestId", "seq"])) {
        throw new Error("INVALID_STREAM_EVENT");
      }
      return { ...base, type: value.type };
    case "failed": {
      if (
        !hasExactKeys(value, ["type", "requestId", "seq", "error"]) ||
        !isRecord(value.error)
      ) {
        throw new Error("INVALID_STREAM_EVENT");
      }
      return {
        ...base,
        type: value.type,
        error: parseProviderError(value.error),
      };
    }
    default:
      throw new Error("INVALID_STREAM_EVENT");
  }
}

function commandErrorCode(error: unknown): string | null {
  return isRecord(error) && typeof error.code === "string" ? error.code : null;
}

function commandHttpStatus(error: unknown): number | null {
  if (!isRecord(error) || typeof error.httpStatus !== "number") return null;
  return Number.isInteger(error.httpStatus) ? error.httpStatus : null;
}

export function publicStreamErrorMessage(error: unknown): string {
  const code = commandErrorCode(error);
  const httpStatus = commandHttpStatus(error);
  if (
    code === "CREDENTIAL_NOT_CONFIGURED" ||
    code === "CREDENTIAL_UNSUPPORTED"
  ) {
    return "설정에서 API 키와 모델 ID를 먼저 준비해 주세요.";
  }
  if (
    code === "CREDENTIAL_STORE_LOCKED" ||
    code === "CREDENTIAL_STORE_UNAVAILABLE" ||
    code === "CREDENTIAL_STORE_FAILED"
  ) {
    return "기기 보안 저장소를 사용할 수 없습니다. 잠금을 해제한 뒤 다시 시도해 주세요.";
  }
  if (code === "VERTEX_OAUTH_NOT_CONFIGURED") {
    return "Vertex AI 로그인은 아직 지원되지 않습니다.";
  }
  if (
    code === "DNS_TIMEOUT" ||
    code === "DNS_RESOLUTION_FAILED" ||
    code === "DNS_NO_ADDRESSES"
  ) {
    return "제공자 서버 주소를 확인하지 못했습니다. 네트워크 연결을 확인해 주세요.";
  }
  if (
    code === "OVERALL_TIMEOUT" ||
    code === "RESPONSE_HEADER_TIMEOUT" ||
    code === "STREAM_IDLE_TIMEOUT" ||
    code === "STREAM_ACK_TIMEOUT" ||
    code === "EXACT_TOKEN_COUNT_TIMEOUT"
  ) {
    return "응답 시간이 초과되었습니다. 잠시 후 다시 시도해 주세요.";
  }
  if (httpStatus === 401 || httpStatus === 403) {
    return "API 인증에 실패했습니다. 저장된 키를 확인해 주세요.";
  }
  if (httpStatus === 429) {
    return "제공자 요청 한도에 도달했습니다. 잠시 후 다시 시도해 주세요.";
  }
  return FIRST_CHAT_STREAM_UNKNOWN_ERROR;
}

function parseAckResponse(
  value: unknown,
  requestId: string,
  seq: number,
): number {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["requestId", "acknowledgedThrough", "inFlight"]) ||
    value.requestId !== requestId ||
    value.acknowledgedThrough !== seq ||
    !isSafeSequence(value.inFlight)
  ) {
    throw new Error("INVALID_ACK_RESPONSE");
  }
  return value.inFlight;
}

function parseCancelResponse(value: unknown, requestId: string): boolean {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["requestId", "accepted"]) ||
    value.requestId !== requestId ||
    typeof value.accepted !== "boolean"
  ) {
    throw new Error("INVALID_CANCEL_RESPONSE");
  }
  return value.accepted;
}

function parseTerminalReceipt(value: unknown): NativeTerminalReceipt | null {
  if (value === null) return null;
  if (!isRecord(value) || typeof value.type !== "string") {
    throw new Error("INVALID_TERMINAL_RECEIPT");
  }
  if (value.type === "completed") {
    if (
      !hasExactKeys(value, ["type", "seq", "reason", "usage"]) ||
      !isSafeSequence(value.seq)
    ) {
      throw new Error("INVALID_TERMINAL_RECEIPT");
    }
    return { type: value.type, seq: value.seq };
  }
  if (value.type === "cancelled") {
    if (!hasExactKeys(value, ["type", "seq"]) || !isSafeSequence(value.seq)) {
      throw new Error("INVALID_TERMINAL_RECEIPT");
    }
    return { type: value.type, seq: value.seq };
  }
  if (value.type === "failed") {
    if (
      !hasExactKeys(value, ["type", "seq", "error"]) ||
      !isSafeSequence(value.seq)
    ) {
      throw new Error("INVALID_TERMINAL_RECEIPT");
    }
    return {
      type: value.type,
      seq: value.seq,
      error: parseProviderError(value.error),
    };
  }
  throw new Error("INVALID_TERMINAL_RECEIPT");
}

function parseSnapshot(value: unknown, requestId: string): StreamSnapshot {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "requestId",
      "lastSentSeq",
      "acknowledgedThrough",
      "inFlight",
      "cancelRequested",
      "terminal",
    ]) ||
    value.requestId !== requestId ||
    !isSafeSequence(value.lastSentSeq) ||
    !(
      value.acknowledgedThrough === null ||
      isSafeSequence(value.acknowledgedThrough)
    ) ||
    !isSafeSequence(value.inFlight) ||
    typeof value.cancelRequested !== "boolean"
  ) {
    throw new Error("INVALID_STREAM_SNAPSHOT");
  }
  if (
    value.acknowledgedThrough !== null &&
    value.acknowledgedThrough > value.lastSentSeq
  ) {
    throw new Error("INVALID_STREAM_SNAPSHOT");
  }
  const acknowledgedCount =
    value.acknowledgedThrough === null ? 0 : value.acknowledgedThrough + 1;
  if (value.inFlight !== value.lastSentSeq + 1 - acknowledgedCount) {
    throw new Error("INVALID_STREAM_SNAPSHOT");
  }
  const terminal = parseTerminalReceipt(value.terminal);
  if (terminal !== null && terminal.seq !== value.lastSentSeq) {
    throw new Error("INVALID_STREAM_SNAPSHOT");
  }
  return {
    requestId: value.requestId,
    lastSentSeq: value.lastSentSeq,
    acknowledgedThrough: value.acknowledgedThrough,
    inFlight: value.inFlight,
    cancelRequested: value.cancelRequested,
    terminal,
  };
}

function terminalReceiptMatchesEvent(
  receipt: NativeTerminalReceipt,
  event: NativeTerminalEvent,
): boolean {
  if (receipt.type !== event.type || receipt.seq !== event.seq) return false;
  if (receipt.type !== "failed" || event.type !== "failed") return true;
  return (
    receipt.error.code === event.error.code &&
    receipt.error.httpStatus === event.error.httpStatus
  );
}

function validateTerminalSnapshot(
  snapshot: StreamSnapshot,
  event: NativeTerminalEvent,
): void {
  if (
    snapshot.terminal === null ||
    !terminalReceiptMatchesEvent(snapshot.terminal, event) ||
    snapshot.lastSentSeq !== event.seq ||
    snapshot.cancelRequested !== (event.type === "cancelled")
  ) {
    throw new Error("INVALID_STREAM_SNAPSHOT");
  }
  if (
    snapshot.acknowledgedThrough !== event.seq - 1 ||
    snapshot.inFlight !== 1
  ) {
    throw new Error("INVALID_STREAM_SNAPSHOT");
  }
}

export function startFirstChatStream(
  profile: ActiveProviderProfile,
  chatId: string,
  userMessage: string,
  callbacks: FirstChatStreamCallbacks,
  dependencies: FirstChatStreamDependencies = defaultDependencies,
): FirstChatStreamHandle {
  const command = buildFirstChatCommand(profile, chatId, userMessage);

  let authorization: StreamAuthorization | null = null;
  let expectedSeq = 0;
  let bufferedEvents = 0;
  let visibleOutputBytes = 0;
  let unseenSnapshotCount = 0;
  let settled = false;
  let cancelRequested = false;
  let cancelPromise: Promise<void> | null = null;
  let processing = Promise.resolve();
  let recoveryTimer: ReturnType<typeof setTimeout> | null = null;
  let resolveReady: () => void = () => undefined;
  const ready = new Promise<void>((resolve) => {
    resolveReady = resolve;
  });
  let resolveDone: (terminal: FirstChatTerminal) => void = () => undefined;
  const done = new Promise<FirstChatTerminal>((resolve) => {
    resolveDone = resolve;
  });

  const clearRecoveryPoll = (): void => {
    if (recoveryTimer !== null) {
      clearTimeout(recoveryTimer);
      recoveryTimer = null;
    }
  };

  const finish = (terminal: FirstChatTerminal): void => {
    if (settled) return;
    settled = true;
    clearRecoveryPoll();
    authorization = null;
    resolveDone(terminal);
  };

  const notifyError = (message: string): void => {
    try {
      callbacks.onError(message);
    } catch {
      // A consumer callback cannot keep native stream resources alive.
    }
  };

  const notifyTerminal = (terminal: FirstChatTerminal): void => {
    try {
      callbacks.onTerminal(terminal);
    } catch {
      // Terminal ownership and cleanup do not depend on UI callback success.
    }
  };

  const cancelAuthorized = async (
    auth: StreamAuthorization,
  ): Promise<boolean> => {
    const value = await dependencies.invokeCommand("cancel_provider_stream", {
      requestId: auth.requestId,
      controlToken: auth.controlToken,
    });
    return parseCancelResponse(value, auth.requestId);
  };

  const completeVerifiedTerminal = (event: NativeTerminalEvent): void => {
    const terminal: FirstChatTerminal = event.type;
    if (event.type === "failed") {
      notifyError(publicStreamErrorMessage(event.error));
    }
    notifyTerminal(terminal);
    finish(terminal);
  };

  const abortProtocol = (): void => {
    if (settled) return;
    notifyError(FIRST_CHAT_STREAM_PROTOCOL_ERROR);
    notifyTerminal("failed");
    const auth = authorization;
    if (auth !== null) {
      void cancelAuthorized(auth).catch(() => undefined);
    }
    finish("failed");
  };

  const terminalEventFromReceipt = (
    receipt: NativeTerminalReceipt,
    requestId: string,
  ): NativeTerminalEvent => {
    if (receipt.type === "failed") {
      return { ...receipt, requestId };
    }
    return { ...receipt, requestId };
  };

  const defaultRecoveryDelay = (): number =>
    cancelRequested
      ? FIRST_CHAT_CANCEL_RECOVERY_MS
      : FIRST_CHAT_RECOVERY_POLL_MS;

  const recoverFromSnapshot = async (): Promise<void> => {
    if (settled) return;
    const auth = authorization;
    if (auth === null) return;
    const raw = await dependencies.invokeCommand(
      "get_provider_stream_snapshot",
      {
        requestId: auth.requestId,
        controlToken: auth.controlToken,
      },
    );
    const snapshot = parseSnapshot(raw, auth.requestId);
    const expectedAcknowledged = expectedSeq === 0 ? null : expectedSeq - 1;
    if (snapshot.acknowledgedThrough !== expectedAcknowledged) {
      throw new Error("STREAM_SNAPSHOT_ACK_MISMATCH");
    }

    if (snapshot.terminal === null) {
      if (snapshot.lastSentSeq >= expectedSeq) {
        unseenSnapshotCount += 1;
        if (unseenSnapshotCount >= 2) {
          throw new Error("STREAM_EVENT_LOST");
        }
        armRecoveryPoll(250);
        return;
      }
      if (snapshot.lastSentSeq !== expectedSeq - 1) {
        throw new Error("STREAM_SNAPSHOT_SEQUENCE_MISMATCH");
      }
      unseenSnapshotCount = 0;
      armRecoveryPoll(defaultRecoveryDelay());
      return;
    }

    if (snapshot.terminal.seq > expectedSeq) {
      unseenSnapshotCount += 1;
      if (unseenSnapshotCount >= 2) {
        throw new Error("STREAM_EVENT_LOST_BEFORE_TERMINAL");
      }
      armRecoveryPoll(250);
      return;
    }
    if (snapshot.terminal.seq !== expectedSeq) {
      throw new Error("STREAM_TERMINAL_SEQUENCE_MISMATCH");
    }

    const terminalEvent = terminalEventFromReceipt(
      snapshot.terminal,
      auth.requestId,
    );
    validateTerminalSnapshot(snapshot, terminalEvent);
    const ack = await dependencies.invokeCommand("ack_provider_stream", {
      requestId: auth.requestId,
      controlToken: auth.controlToken,
      seq: terminalEvent.seq,
    });
    if (parseAckResponse(ack, auth.requestId, terminalEvent.seq) !== 0) {
      throw new Error("TERMINAL_ACK_LEFT_EVENTS_IN_FLIGHT");
    }
    expectedSeq += 1;
    completeVerifiedTerminal(terminalEvent);
  };

  const armRecoveryPoll = (delay = FIRST_CHAT_RECOVERY_POLL_MS): void => {
    if (settled || authorization === null) return;
    clearRecoveryPoll();
    recoveryTimer = setTimeout(() => {
      recoveryTimer = null;
      processing = processing
        .then(recoverFromSnapshot)
        .catch(() => abortProtocol());
    }, delay);
    const timer = recoveryTimer as unknown as { unref?: () => void };
    timer.unref?.();
  };

  const processEvent = async (raw: unknown): Promise<void> => {
    await ready;
    if (settled) return;
    const auth = authorization;
    if (auth === null) return;

    const event = parseEvent(raw);
    if (event.requestId !== auth.requestId || event.seq !== expectedSeq) {
      throw new Error("STREAM_EVENT_IDENTITY_OR_SEQUENCE_MISMATCH");
    }
    if (event.type === "started" && event.seq !== 0) {
      throw new Error("STREAM_STARTED_SEQUENCE_MISMATCH");
    }
    if (event.type !== "started" && event.seq === 0) {
      throw new Error("STREAM_EVENT_SEQUENCE_MISMATCH");
    }

    if (event.type === "started") {
      try {
        callbacks.onStarted?.();
      } catch {
        // Native persistence and ACK ownership do not depend on UI callbacks.
      }
    }

    unseenSnapshotCount = 0;
    let terminalEvent: NativeTerminalEvent | null = null;
    if (event.type === "text_delta" || event.type === "refusal_delta") {
      visibleOutputBytes += utf8Length(event.text);
      if (visibleOutputBytes > MAX_VISIBLE_OUTPUT_BYTES) {
        throw new Error("STREAM_VISIBLE_OUTPUT_TOO_LARGE");
      }
      callbacks.onDelta(
        event.text,
        event.type === "text_delta" ? "text" : "refusal",
      );
    } else if (
      event.type === "completed" ||
      event.type === "cancelled" ||
      event.type === "failed"
    ) {
      terminalEvent = event;
    }

    if (terminalEvent !== null) {
      const snapshot = await dependencies.invokeCommand(
        "get_provider_stream_snapshot",
        {
          requestId: auth.requestId,
          controlToken: auth.controlToken,
        },
      );
      const parsedSnapshot = parseSnapshot(snapshot, auth.requestId);
      validateTerminalSnapshot(parsedSnapshot, terminalEvent);
      const ack = await dependencies.invokeCommand("ack_provider_stream", {
        requestId: auth.requestId,
        controlToken: auth.controlToken,
        seq: event.seq,
      });
      if (parseAckResponse(ack, auth.requestId, event.seq) !== 0) {
        throw new Error("TERMINAL_ACK_LEFT_EVENTS_IN_FLIGHT");
      }
      expectedSeq += 1;
      completeVerifiedTerminal(terminalEvent);
    } else {
      const ack = await dependencies.invokeCommand("ack_provider_stream", {
        requestId: auth.requestId,
        controlToken: auth.controlToken,
        seq: event.seq,
      });
      parseAckResponse(ack, auth.requestId, event.seq);
      expectedSeq += 1;
      armRecoveryPoll(defaultRecoveryDelay());
    }
  };

  const channel = dependencies.createChannel((raw) => {
    if (settled) return;
    clearRecoveryPoll();
    bufferedEvents += 1;
    if (bufferedEvents > MAX_BUFFERED_EVENTS) {
      abortProtocol();
      return;
    }
    processing = processing
      .then(() => processEvent(raw))
      .catch(() => abortProtocol())
      .finally(() => {
        bufferedEvents -= 1;
      });
  });

  void dependencies
    .invokeCommand("start_provider_stream", {
      ...command,
      onEvent: channel,
    })
    .then(async (value) => {
      const parsed = parseStartResponse(value);
      if (settled) {
        resolveReady();
        await cancelAuthorized(parsed).catch(() => undefined);
        return;
      }
      authorization = parsed;
      resolveReady();
      armRecoveryPoll(defaultRecoveryDelay());
    })
    .catch((error: unknown) => {
      resolveReady();
      if (settled) return;
      notifyError(publicStreamErrorMessage(error));
      notifyTerminal("failed");
      finish("failed");
    });

  return Object.freeze({
    done,
    cancel(): Promise<void> {
      cancelRequested = true;
      cancelPromise ??= ready
        .then(async () => {
          if (settled) return;
          const auth = authorization;
          if (auth === null) return;
          const accepted = await cancelAuthorized(auth);
          armRecoveryPoll(
            accepted ? FIRST_CHAT_CANCEL_RECOVERY_MS : 0,
          );
        })
        .catch((error: unknown) => {
          if (!settled) {
            notifyError(publicStreamErrorMessage(error));
            notifyTerminal("failed");
            finish("failed");
          }
          throw error;
        });
      return cancelPromise;
    },
  });
}
