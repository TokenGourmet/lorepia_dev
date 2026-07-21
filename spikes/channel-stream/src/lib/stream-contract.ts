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
  text: string;
  error: { code: string; message: string } | null;
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
    case "completed":
      if (typeof value.text !== "string") {
        return {
          accepted: false,
          error: `${value.type} 이벤트의 text가 문자열이 아닙니다.`,
        };
      }
      return { accepted: true, event: value as StreamEvent };
    case "cancelled":
      if (typeof value.partialText !== "string") {
        return {
          accepted: false,
          error: "cancelled 이벤트의 partialText가 문자열이 아닙니다.",
        };
      }
      return { accepted: true, event: value as StreamEvent };
    case "failed":
      if (
        typeof value.partialText !== "string" ||
        !isRecord(value.error) ||
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
  event: TerminalStreamEvent,
): ExpectedTerminalSnapshot {
  if (event.type === "completed") {
    return {
      requestId: event.requestId,
      status: "completed",
      lastSeq: event.seq,
      text: event.text,
      error: null,
    };
  }

  if (event.type === "cancelled") {
    return {
      requestId: event.requestId,
      status: "cancelled",
      lastSeq: event.seq,
      text: event.partialText,
      error: null,
    };
  }

  return {
    requestId: event.requestId,
    status: "failed",
    lastSeq: event.seq,
    text: event.partialText,
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
      ? expectedSnapshotFrom(event)
      : null;

  if (terminalExpectation !== null && terminalExpectation.text !== state.text) {
    return {
      accepted: false,
      shouldAcknowledge: false,
      error: `${event.type} 이벤트의 최종 텍스트가 accepted delta 누적값과 일치하지 않습니다.`,
      nextState: state,
    };
  }

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

function hasValidSnapshotPayload(value: unknown): value is StreamSnapshot {
  if (!isRecord(value)) return false;

  const statusIsValid =
    value.status === "queued" ||
    value.status === "streaming" ||
    value.status === "completed" ||
    value.status === "cancelled" ||
    value.status === "failed";
  if (!statusIsValid) return false;

  const errorIsValid =
    value.error === null ||
    (isRecord(value.error) &&
      typeof value.error.code === "string" &&
      typeof value.error.message === "string");
  const statusErrorIsValid = value.status === "failed" ? value.error !== null : value.error === null;

  const lastSeq =
    value.lastSeq === null
      ? null
      : isNonnegativeSafeInteger(value.lastSeq)
        ? value.lastSeq
        : undefined;
  const lastAckedSeq =
    value.lastAckedSeq === null
      ? null
      : isNonnegativeSafeInteger(value.lastAckedSeq)
        ? value.lastAckedSeq
        : undefined;
  const inFlight = isNonnegativeSafeInteger(value.inFlight) ? value.inFlight : null;
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

  if (lastSeq === undefined || lastAckedSeq === undefined || inFlight === null) return false;

  const expectedInFlight =
    lastSeq === null
      ? 0
      : lastAckedSeq === null
        ? lastSeq + 1
        : lastSeq - lastAckedSeq;
  const sequenceStateIsValid =
    value.status === "queued"
      ? lastSeq === null && lastAckedSeq === null && inFlight === 0
      : lastSeq !== null &&
        (lastAckedSeq === null || lastAckedSeq <= lastSeq) &&
        Number.isSafeInteger(expectedInFlight) &&
        expectedInFlight >= 0 &&
        inFlight === expectedInFlight;

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
    maxInFlight !== null && maxInFlightIsValid && inFlight <= maxInFlight;

  return (
    typeof value.requestId === "string" &&
    value.requestId.length > 0 &&
    typeof value.text === "string" &&
    errorIsValid &&
    statusErrorIsValid &&
    sequenceStateIsValid &&
    inFlightIsValid &&
    batchWindowIsValid &&
    effectiveBatchWindowIsValid &&
    maxInFlightIsValid
  );
}

function terminalContentMismatches(
  snapshot: StreamSnapshot,
  expected: ExpectedTerminalSnapshot,
): string[] {
  const mismatches: string[] = [];
  if (snapshot.requestId !== expected.requestId) mismatches.push("requestId");
  if (snapshot.status !== expected.status) mismatches.push("status");
  if (snapshot.lastSeq !== expected.lastSeq) mismatches.push("lastSeq");
  if (snapshot.text !== expected.text) mismatches.push("text");

  const errorMatches =
    (snapshot.error === null && expected.error === null) ||
    (snapshot.error !== null &&
      expected.error !== null &&
      snapshot.error.code === expected.error.code &&
      snapshot.error.message === expected.error.message);
  if (!errorMatches) mismatches.push("error");
  return mismatches;
}

export function expectedTerminalSnapshotFromSnapshot(
  snapshot: StreamSnapshot,
): ExpectedTerminalSnapshot | null {
  if (
    snapshot.lastSeq === null ||
    (snapshot.status !== "completed" &&
      snapshot.status !== "cancelled" &&
      snapshot.status !== "failed")
  ) {
    return null;
  }

  return {
    requestId: snapshot.requestId,
    status: snapshot.status,
    lastSeq: snapshot.lastSeq,
    text: snapshot.text,
    error: snapshot.error,
  };
}

export function validateTerminalRecoverySnapshot(
  value: unknown,
  state: StreamContractState,
  expectedRequestId: string,
): SnapshotValidation {
  if (!hasValidSnapshotPayload(value)) {
    return {
      accepted: false,
      error: "종료 복구 스냅샷 데이터 형식이 유효하지 않습니다.",
      mismatches: ["payload"],
    };
  }

  const terminalExpectation = expectedTerminalSnapshotFromSnapshot(value);
  if (terminalExpectation === null) {
    return {
      accepted: false,
      error: "종료 복구 명령이 비종료 스냅샷을 반환했습니다.",
      mismatches: ["status"],
    };
  }

  const mismatches: string[] = [];
  if (value.requestId !== expectedRequestId) mismatches.push("requestId");
  if (state.requestId !== null && value.requestId !== state.requestId) {
    if (!mismatches.includes("requestId")) mismatches.push("requestId");
  }
  if (state.lastSeq !== null && terminalExpectation.lastSeq < state.lastSeq) {
    mismatches.push("lastSeq");
  }
  if (!terminalExpectation.text.startsWith(state.text)) mismatches.push("text");
  if (state.expectedTerminal !== null) {
    for (const mismatch of terminalContentMismatches(value, state.expectedTerminal)) {
      if (!mismatches.includes(mismatch)) mismatches.push(mismatch);
    }
  }

  if (mismatches.length > 0) {
    return {
      accepted: false,
      error: `종료 복구 스냅샷 불일치: ${mismatches.join(", ")}`,
      mismatches,
    };
  }

  return { accepted: true, snapshot: value };
}

export function snapshotMismatches(
  snapshot: StreamSnapshot,
  expected: ExpectedTerminalSnapshot,
): string[] {
  const mismatches: string[] = [];

  if (snapshot.requestId !== expected.requestId) mismatches.push("requestId");
  if (snapshot.status !== expected.status) mismatches.push("status");
  if (snapshot.lastSeq !== expected.lastSeq) mismatches.push("lastSeq");
  if (snapshot.lastAckedSeq !== expected.lastSeq) mismatches.push("lastAckedSeq");
  if (snapshot.inFlight !== 0) mismatches.push("inFlight");
  if (snapshot.text !== expected.text) mismatches.push("text");

  const errorMatches =
    (snapshot.error === null && expected.error === null) ||
    (snapshot.error !== null &&
      expected.error !== null &&
      snapshot.error.code === expected.error.code &&
      snapshot.error.message === expected.error.message);
  if (!errorMatches) mismatches.push("error");

  return mismatches;
}

export function validateTerminalSnapshot(
  value: unknown,
  expected: ExpectedTerminalSnapshot,
): SnapshotValidation {
  if (!hasValidSnapshotPayload(value)) {
    return {
      accepted: false,
      error: "최종 스냅샷 데이터 형식이 유효하지 않습니다.",
      mismatches: ["payload"],
    };
  }

  const mismatches = snapshotMismatches(value, expected);
  if (mismatches.length > 0) {
    return {
      accepted: false,
      error: `최종 스냅샷 불일치: ${mismatches.join(", ")}`,
      mismatches,
    };
  }

  return { accepted: true, snapshot: value };
}
