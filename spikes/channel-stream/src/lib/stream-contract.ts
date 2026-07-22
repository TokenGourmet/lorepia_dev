import type {
  StreamEvent,
  StreamSnapshot,
  StreamSnapshotStatus,
} from "./stream-protocol";

export type TerminalStreamEvent = Exclude<
  StreamEvent,
  { type: "started" | "delta" }
>;

export type ExpectedTerminalSnapshot = {
  requestId: string;
  status: Extract<StreamSnapshotStatus, "completed" | "cancelled" | "failed">;
  lastSeq: number;
  acceptedText: string;
  error: { code: string; message: string } | null;
};

export type TextReceipt = {
  textBytes: number;
  textSha256: string;
};

export type StreamContractState = {
  requestId: string | null;
  lastSeq: number | null;
  text: string;
  terminalSeen: boolean;
  expectedTerminal: ExpectedTerminalSnapshot | null;
};

export type AcceptedEvent = {
  accepted: true;
  shouldAcknowledge: true;
  event: StreamEvent;
  nextState: StreamContractState;
  terminalExpectation: ExpectedTerminalSnapshot | null;
};

export type RejectedEvent = {
  accepted: false;
  shouldAcknowledge: false;
  error: string;
  nextState: StreamContractState;
};

export type EventValidation = AcceptedEvent | RejectedEvent;

export type SnapshotValidation =
  | { accepted: true; snapshot: StreamSnapshot }
  | { accepted: false; error: string; mismatches: string[] };

export type TerminalRecoveryValidation =
  | {
      accepted: true;
      snapshot: StreamSnapshot;
      expected: ExpectedTerminalSnapshot;
      terminalEventDelivered: boolean;
    }
  | { accepted: false; error: string; mismatches: string[] };

export type ReleaseValidation =
  | { accepted: true }
  | { accepted: false; error: string };

export function createStreamContractState(): StreamContractState {
  return {
    requestId: null,
    lastSeq: null,
    text: "",
    terminalSeen: false,
    expectedTerminal: null,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isNonnegativeSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function hasOnlyKeys(value: Record<string, unknown>, allowedKeys: string[]): boolean {
  const allowed = new Set(allowedKeys);
  return Object.keys(value).every((key) => allowed.has(key));
}

function getWebCryptoDigest(): Pick<SubtleCrypto, "digest"> {
  const subtle = globalThis.crypto?.subtle;
  if (subtle === undefined) {
    throw new Error("Web Crypto SHA-256을 사용할 수 없습니다.");
  }
  return subtle;
}

export async function createTextReceipt(
  text: string,
  digestProvider: Pick<SubtleCrypto, "digest"> = getWebCryptoDigest(),
): Promise<TextReceipt> {
  const encoded = new TextEncoder().encode(text);
  const digest = await digestProvider.digest("SHA-256", encoded);
  const textSha256 = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");

  return { textBytes: encoded.byteLength, textSha256 };
}

function describeEventType(value: unknown): string {
  if (!isRecord(value) || !("type" in value)) return "형식 불명";
  return String(value.type ?? "형식 불명");
}

function parseStreamEvent(
  value: unknown,
): { accepted: true; event: StreamEvent } | { accepted: false; error: string } {
  if (!isRecord(value)) {
    return { accepted: false, error: "객체가 아닌 이벤트를 수신했습니다." };
  }

  if (typeof value.requestId !== "string" || value.requestId.length === 0) {
    return {
      accepted: false,
      error: "요청 ID가 없거나 문자열이 아닌 이벤트를 수신했습니다.",
    };
  }

  if (!isNonnegativeSafeInteger(value.seq)) {
    return {
      accepted: false,
      error: `유효하지 않은 seq ${String(value.seq)}를 수신했습니다.`,
    };
  }

  switch (value.type) {
    case "started":
      if (
        !Number.isSafeInteger(value.batchWindowMs) ||
        (value.batchWindowMs as number) < 16 ||
        (value.batchWindowMs as number) > 50 ||
        !Number.isSafeInteger(value.maxInFlight) ||
        (value.maxInFlight as number) < 2 ||
        (value.maxInFlight as number) > 64
      ) {
        return {
          accepted: false,
          error: "started 이벤트의 배칭 설정이 유효하지 않습니다.",
        };
      }
      return { accepted: true, event: value as StreamEvent };
    case "delta":
      if (typeof value.text !== "string" || value.text.length === 0) {
        return {
          accepted: false,
          error: "delta 이벤트의 text는 비어 있지 않은 문자열이어야 합니다.",
        };
      }
      return { accepted: true, event: value as StreamEvent };
    case "completed":
    case "cancelled":
      if (!hasOnlyKeys(value, ["type", "requestId", "seq"])) {
        return {
          accepted: false,
          error: `${value.type} 종료 이벤트에 허용되지 않은 필드가 있습니다.`,
        };
      }
      return { accepted: true, event: value as StreamEvent };
    case "failed":
      if (!hasOnlyKeys(value, ["type", "requestId", "seq", "error"])) {
        return {
          accepted: false,
          error: "failed 종료 이벤트에 허용되지 않은 필드가 있습니다.",
        };
      }
      if (
        !isRecord(value.error) ||
        !hasOnlyKeys(value.error, ["code", "message"]) ||
        typeof value.error.code !== "string" ||
        typeof value.error.message !== "string"
      ) {
        return {
          accepted: false,
          error: "failed 이벤트의 오류 데이터가 유효하지 않습니다.",
        };
      }
      return { accepted: true, event: value as StreamEvent };
    default:
      return {
        accepted: false,
        error: `알 수 없는 이벤트 타입 ${String(value.type)}을 수신했습니다.`,
      };
  }
}

export function expectedSnapshotFrom(
  state: StreamContractState,
  event: TerminalStreamEvent,
): ExpectedTerminalSnapshot {
  if (event.type === "completed") {
    return {
      requestId: event.requestId,
      status: "completed",
      lastSeq: event.seq,
      acceptedText: state.text,
      error: null,
    };
  }

  if (event.type === "cancelled") {
    return {
      requestId: event.requestId,
      status: "cancelled",
      lastSeq: event.seq,
      acceptedText: state.text,
      error: null,
    };
  }

  return {
    requestId: event.requestId,
    status: "failed",
    lastSeq: event.seq,
    acceptedText: state.text,
    error: event.error,
  };
}

export function validateStreamEvent(
  state: StreamContractState,
  value: unknown,
): EventValidation {
  if (state.terminalSeen) {
    return {
      accepted: false,
      shouldAcknowledge: false,
      error: `종료 이벤트 이후 ${describeEventType(value)} 이벤트를 추가로 수신했습니다. ACK하지 않았습니다.`,
      nextState: state,
    };
  }

  const parsed = parseStreamEvent(value);
  if (!parsed.accepted) {
    return {
      accepted: false,
      shouldAcknowledge: false,
      error: parsed.error,
      nextState: state,
    };
  }

  const event = parsed.event;

  if (state.requestId !== null && state.requestId !== event.requestId) {
    return {
      accepted: false,
      shouldAcknowledge: false,
      error: `요청 ${state.requestId} 처리 중 다른 요청 ${event.requestId}의 이벤트를 수신했습니다.`,
      nextState: state,
    };
  }

  if (event.type === "started") {
    if (state.lastSeq !== null) {
      return {
        accepted: false,
        shouldAcknowledge: false,
        error: "started 이벤트를 두 번 수신했습니다.",
        nextState: state,
      };
    }

    if (event.seq !== 0) {
      return {
        accepted: false,
        shouldAcknowledge: false,
        error: `started seq 0을 기대했지만 ${event.seq}를 수신했습니다.`,
        nextState: state,
      };
    }
  } else if (state.lastSeq === null) {
    return {
      accepted: false,
      shouldAcknowledge: false,
      error: `started 이벤트보다 ${event.type} seq ${event.seq}를 먼저 수신했습니다.`,
      nextState: state,
    };
  } else {
    const expectedSeq = state.lastSeq + 1;
    if (event.seq !== expectedSeq) {
      return {
        accepted: false,
        shouldAcknowledge: false,
        error: `${event.type} seq ${expectedSeq}을 기대했지만 ${event.seq}를 수신했습니다.`,
        nextState: state,
      };
    }
  }

  const terminalExpectation =
    event.type === "completed" || event.type === "cancelled" || event.type === "failed"
      ? expectedSnapshotFrom(state, event)
      : null;

  const nextState: StreamContractState = {
    requestId: state.requestId ?? event.requestId,
    lastSeq: event.seq,
    text: event.type === "delta" ? state.text + event.text : state.text,
    terminalSeen: terminalExpectation !== null,
    expectedTerminal: terminalExpectation,
  };

  return {
    accepted: true,
    shouldAcknowledge: true,
    event,
    nextState,
    terminalExpectation,
  };
}

export function isValidStreamSnapshotPayload(value: unknown): value is StreamSnapshot {
  if (!isRecord(value)) return false;

  const hasValidKeys = hasOnlyKeys(value, [
    "requestId",
    "status",
    "lastSeq",
    "lastAckedSeq",
    "inFlight",
    "textBytes",
    "textSha256",
    "error",
    "batchWindowMs",
    "effectiveBatchWindowMs",
    "maxInFlight",
  ]);

  const statusIsValid =
    value.status === "queued" ||
    value.status === "streaming" ||
    value.status === "completed" ||
    value.status === "cancelled" ||
    value.status === "failed";
  const errorIsValid =
    value.error === null ||
    (isRecord(value.error) &&
      hasOnlyKeys(value.error, ["code", "message"]) &&
      typeof value.error.code === "string" &&
      value.error.code.length > 0 &&
      value.error.code.length <= 64 &&
      typeof value.error.message === "string" &&
      value.error.message.length <= 512);

  const batchWindowMs = isNonnegativeSafeInteger(value.batchWindowMs)
    ? value.batchWindowMs
    : null;
  const effectiveBatchWindowMs = isNonnegativeSafeInteger(
    value.effectiveBatchWindowMs,
  )
    ? value.effectiveBatchWindowMs
    : null;
  const maxInFlight = isNonnegativeSafeInteger(value.maxInFlight)
    ? value.maxInFlight
    : null;
  const inFlight = isNonnegativeSafeInteger(value.inFlight) ? value.inFlight : null;
  const batchWindowIsValid =
    batchWindowMs !== null && batchWindowMs >= 16 && batchWindowMs <= 50;
  const effectiveBatchWindowIsValid =
    effectiveBatchWindowMs !== null &&
    batchWindowMs !== null &&
    batchWindowIsValid &&
    effectiveBatchWindowMs >= batchWindowMs &&
    effectiveBatchWindowMs <= 50;
  const maxInFlightIsValid =
    maxInFlight !== null && maxInFlight >= 2 && maxInFlight <= 64;
  const inFlightIsValid =
    inFlight !== null &&
    maxInFlight !== null &&
    maxInFlightIsValid &&
    inFlight <= maxInFlight;

  const lastSeq = value.lastSeq;
  const lastAckedSeq = value.lastAckedSeq;
  const sequencesAreValid =
    (lastSeq === null || isNonnegativeSafeInteger(lastSeq)) &&
    (lastAckedSeq === null || isNonnegativeSafeInteger(lastAckedSeq));
  const sequenceAccountingIsValid = (() => {
    if (!sequencesAreValid || inFlight === null) return false;
    if (lastSeq === null) {
      return (
        value.status === "queued" && lastAckedSeq === null && inFlight === 0
      );
    }
    if (value.status === "queued") return false;
    if (lastAckedSeq !== null && lastAckedSeq > lastSeq) return false;
    const expectedInFlight =
      BigInt(lastSeq) - BigInt(lastAckedSeq === null ? -1 : lastAckedSeq);
    return expectedInFlight === BigInt(inFlight);
  })();
  const errorMatchesStatus =
    value.status === "failed" ? value.error !== null : value.error === null;
  const queuedReceiptIsValid =
    value.status !== "queued" ||
    (value.textBytes === 0 &&
      value.textSha256 ===
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");

  return (
    hasValidKeys &&
    typeof value.requestId === "string" &&
    value.requestId.length > 0 &&
    statusIsValid &&
    sequenceAccountingIsValid &&
    inFlightIsValid &&
    isNonnegativeSafeInteger(value.textBytes) &&
    value.textBytes <= 1_048_576 &&
    typeof value.textSha256 === "string" &&
    /^[0-9a-f]{64}$/.test(value.textSha256) &&
    errorIsValid &&
    errorMatchesStatus &&
    queuedReceiptIsValid &&
    batchWindowIsValid &&
    effectiveBatchWindowIsValid &&
    maxInFlightIsValid
  );
}

function snapshotMismatches(
  snapshot: StreamSnapshot,
  expected: ExpectedTerminalSnapshot,
  expectedReceipt: TextReceipt,
): string[] {
  const mismatches: string[] = [];

  if (snapshot.requestId !== expected.requestId) mismatches.push("requestId");
  if (snapshot.status !== expected.status) mismatches.push("status");
  if (snapshot.lastSeq !== expected.lastSeq) mismatches.push("lastSeq");
  if (snapshot.lastAckedSeq !== expected.lastSeq) mismatches.push("lastAckedSeq");
  if (snapshot.inFlight !== 0) mismatches.push("inFlight");
  if (snapshot.textBytes !== expectedReceipt.textBytes) mismatches.push("textBytes");
  if (snapshot.textSha256 !== expectedReceipt.textSha256) {
    mismatches.push("textSha256");
  }

  const errorMatches =
    (snapshot.error === null && expected.error === null) ||
    (snapshot.error !== null &&
      expected.error !== null &&
      snapshot.error.code === expected.error.code &&
      snapshot.error.message === expected.error.message);
  if (!errorMatches) mismatches.push("error");

  return mismatches;
}

export async function validateTerminalSnapshot(
  value: unknown,
  expected: ExpectedTerminalSnapshot,
  digestProvider?: Pick<SubtleCrypto, "digest">,
): Promise<SnapshotValidation> {
  if (!isValidStreamSnapshotPayload(value)) {
    return {
      accepted: false,
      error: "최종 스냅샷 데이터 형식이 유효하지 않습니다.",
      mismatches: ["payload"],
    };
  }

  const expectedReceipt = await createTextReceipt(
    expected.acceptedText,
    digestProvider ?? getWebCryptoDigest(),
  );
  const mismatches = snapshotMismatches(value, expected, expectedReceipt);
  if (mismatches.length > 0) {
    return {
      accepted: false,
      error: `최종 스냅샷 불일치: ${mismatches.join(", ")}`,
      mismatches,
    };
  }

  return { accepted: true, snapshot: value };
}

function terminalErrorMatches(
  actual: StreamSnapshot["error"],
  expected: ExpectedTerminalSnapshot["error"],
): boolean {
  return (
    (actual === null && expected === null) ||
    (actual !== null &&
      expected !== null &&
      actual.code === expected.code &&
      actual.message === expected.message)
  );
}

export async function validateTerminalRecoverySnapshot(
  value: unknown,
  state: StreamContractState,
  digestProvider?: Pick<SubtleCrypto, "digest">,
): Promise<TerminalRecoveryValidation> {
  if (!isValidStreamSnapshotPayload(value)) {
    return {
      accepted: false,
      error: "종료 복구 스냅샷 데이터 형식이 유효하지 않습니다.",
      mismatches: ["payload"],
    };
  }
  if (
    value.status !== "completed" &&
    value.status !== "cancelled" &&
    value.status !== "failed"
  ) {
    return {
      accepted: false,
      error: "종료되지 않은 스냅샷을 종료 복구에 사용할 수 없습니다.",
      mismatches: ["status"],
    };
  }
  if (state.requestId === null || state.lastSeq === null || value.lastSeq === null) {
    return {
      accepted: false,
      error: "started 이벤트가 확인되지 않아 종료 상태를 복구할 수 없습니다.",
      mismatches: ["sequence"],
    };
  }

  const expectedReceipt = await createTextReceipt(
    state.text,
    digestProvider ?? getWebCryptoDigest(),
  );
  const mismatches: string[] = [];
  if (value.requestId !== state.requestId) mismatches.push("requestId");
  if (value.textBytes !== expectedReceipt.textBytes) mismatches.push("textBytes");
  if (value.textSha256 !== expectedReceipt.textSha256) mismatches.push("textSha256");

  let terminalEventDelivered = false;
  let expected: ExpectedTerminalSnapshot;
  if (state.terminalSeen) {
    if (state.expectedTerminal === null) {
      mismatches.push("expectedTerminal");
      expected = {
        requestId: state.requestId,
        status: value.status,
        lastSeq: value.lastSeq,
        acceptedText: state.text,
        error: value.error,
      };
    } else {
      expected = state.expectedTerminal;
      terminalEventDelivered = true;
      if (value.status !== expected.status) mismatches.push("status");
      if (value.lastSeq !== expected.lastSeq) mismatches.push("lastSeq");
      if (!terminalErrorMatches(value.error, expected.error)) mismatches.push("error");
    }
  } else {
    const nextSeq = state.lastSeq + 1;
    if (Number.isSafeInteger(nextSeq) && value.lastSeq === nextSeq) {
      terminalEventDelivered = true;
    } else if (value.status === "failed" && value.lastSeq === state.lastSeq) {
      terminalEventDelivered = false;
    } else {
      mismatches.push("lastSeq");
    }
    expected = {
      requestId: state.requestId,
      status: value.status,
      lastSeq: value.lastSeq,
      acceptedText: state.text,
      error: value.error,
    };
  }

  if (mismatches.length > 0) {
    return {
      accepted: false,
      error: `종료 복구 스냅샷 불일치: ${mismatches.join(", ")}`,
      mismatches,
    };
  }
  return { accepted: true, snapshot: value, expected, terminalEventDelivered };
}

export function validateReleaseStreamResponse(
  value: unknown,
  requestId: string,
): ReleaseValidation {
  if (
    !isRecord(value) ||
    !hasOnlyKeys(value, ["requestId", "released"]) ||
    value.requestId !== requestId ||
    value.released !== true
  ) {
    return {
      accepted: false,
      error: "스트림 해제 응답이 요청 ID와 정확히 일치하지 않습니다.",
    };
  }
  return { accepted: true };
}
