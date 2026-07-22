import { describe, expect, it } from "vitest";

import {
  createTextReceipt,
  createStreamContractState,
  isValidStreamSnapshotPayload,
  validateReleaseStreamResponse,
  validateStreamEvent,
  validateTerminalRecoverySnapshot,
  validateTerminalSnapshot,
  type EventValidation,
  type StreamContractState,
} from "./stream-contract";
import type { StreamEvent, StreamSnapshot } from "./stream-protocol";

const requestId = "request-1";

const started = (id = requestId): StreamEvent => ({
  type: "started",
  requestId: id,
  seq: 0,
  batchWindowMs: 32,
  maxInFlight: 2,
});

const delta = (seq: number, id = requestId, text = "조각"): StreamEvent => ({
  type: "delta",
  requestId: id,
  seq,
  text,
});

function accept(
  state: StreamContractState,
  event: StreamEvent,
): Extract<EventValidation, { accepted: true }> {
  const result = validateStreamEvent(state, event);
  expect(result.accepted, "event should satisfy the stream contract").toBe(true);
  if (!result.accepted) throw new Error(result.error);
  expect(result.shouldAcknowledge).toBe(true);
  return result;
}

function reject(
  state: StreamContractState,
  event: unknown,
): Extract<EventValidation, { accepted: false }> {
  const result = validateStreamEvent(state, event);
  expect(result.accepted, "event should violate the stream contract").toBe(false);
  if (result.accepted) throw new Error("expected the event to be rejected");
  expect(result.shouldAcknowledge).toBe(false);
  expect(result.nextState).toBe(state);
  return result;
}

describe("stream event contract", () => {
  it("accepts started seq 0, contiguous deltas, and exactly one terminal", () => {
    let state = createStreamContractState();

    const startResult = accept(state, started());
    state = startResult.nextState;
    expect(state).toEqual({
      requestId,
      lastSeq: 0,
      text: "",
      terminalSeen: false,
      expectedTerminal: null,
    });

    state = accept(state, delta(1, requestId, "하나")).nextState;
    state = accept(state, delta(2, requestId, "둘")).nextState;

    const completed: StreamEvent = {
      type: "completed",
      requestId,
      seq: 3,
    };
    const terminalResult = accept(state, completed);
    state = terminalResult.nextState;

    expect(terminalResult.terminalExpectation).toEqual({
      requestId,
      status: "completed",
      lastSeq: 3,
      acceptedText: "하나둘",
      error: null,
    });
    expect(state.terminalSeen).toBe(true);
    expect(state.expectedTerminal).toEqual(terminalResult.terminalExpectation);

    const secondTerminal = reject(state, {
      type: "cancelled",
      requestId,
      seq: 4,
    });
    expect(secondTerminal.error).toContain("종료 이벤트 이후");
  });

  it("rejects an event before started and a started event whose seq is not zero", () => {
    const initial = createStreamContractState();

    expect(reject(initial, delta(1)).error).toContain("started 이벤트보다");
    expect(
      reject(initial, {
        type: "started",
        requestId,
        seq: 1,
        batchWindowMs: 32,
        maxInFlight: 2,
      }).error,
    ).toContain("started seq 0");
  });

  it("rejects skipped and duplicate sequence numbers without advancing state", () => {
    const afterStart = accept(createStreamContractState(), started()).nextState;

    expect(reject(afterStart, delta(2)).error).toContain("seq 1을 기대");

    const afterFirstDelta = accept(afterStart, delta(1)).nextState;
    expect(reject(afterFirstDelta, delta(1)).error).toContain("seq 2을 기대");
  });

  it("rejects a duplicate started event", () => {
    const afterStart = accept(createStreamContractState(), started()).nextState;

    expect(reject(afterStart, started()).error).toContain("두 번");
  });

  it("rejects an event from another request", () => {
    const afterStart = accept(createStreamContractState(), started()).nextState;

    expect(reject(afterStart, delta(1, "request-2")).error).toContain(
      "다른 요청 request-2",
    );
  });

  it("rejects post-terminal events and explicitly forbids their ACK", () => {
    const afterStart = accept(createStreamContractState(), started()).nextState;
    const afterDelta = accept(afterStart, delta(1, requestId, "일부")).nextState;
    const terminal = accept(afterDelta, {
      type: "cancelled",
      requestId,
      seq: 2,
    }).nextState;

    const result = reject(terminal, delta(3));
    expect(result.shouldAcknowledge).toBe(false);
    expect(result.error).toContain("ACK하지 않았습니다");
  });

  it("rejects malformed runtime payloads without allowing an ACK", () => {
    const initial = createStreamContractState();
    const result = reject(initial, {
      type: "started",
      requestId,
      seq: 0,
      batchWindowMs: 0,
      maxInFlight: 2,
    });

    expect(result.shouldAcknowledge).toBe(false);
    expect(result.error).toContain("배칭 설정");
  });

  it("rejects an empty delta without advancing or acknowledging it", () => {
    const afterStart = accept(createStreamContractState(), started()).nextState;
    const result = reject(afterStart, delta(1, requestId, ""));

    expect(result.error).toContain("비어 있지 않은 문자열");
    expect(result.nextState).toBe(afterStart);
    expect(result.shouldAcknowledge).toBe(false);
  });

  it.each([
    { batchWindowMs: 15, maxInFlight: 2 },
    { batchWindowMs: 51, maxInFlight: 2 },
    { batchWindowMs: 32, maxInFlight: 1 },
    { batchWindowMs: 32, maxInFlight: 65 },
  ])(
    "rejects started limits outside the batching contract: %o",
    ({ batchWindowMs, maxInFlight }) => {
      const result = reject(createStreamContractState(), {
        type: "started",
        requestId,
        seq: 0,
        batchWindowMs,
        maxInFlight,
      });

      expect(result.error).toContain("배칭 설정");
    },
  );

  it.each([
    {
      name: "completed",
      terminal: { type: "completed", requestId, seq: 2 },
      status: "completed",
      error: null,
    },
    {
      name: "cancelled",
      terminal: { type: "cancelled", requestId, seq: 2 },
      status: "cancelled",
      error: null,
    },
    {
      name: "failed",
      terminal: {
        type: "failed",
        requestId,
        seq: 2,
        error: { code: "MOCK_FAILURE", message: "의도된 실패" },
      },
      status: "failed",
      error: { code: "MOCK_FAILURE", message: "의도된 실패" },
    },
  ] as const)(
    "derives the $name snapshot text from accepted deltas",
    ({ terminal, status, error }) => {
      const afterStart = accept(createStreamContractState(), started()).nextState;
      const afterDelta = accept(afterStart, delta(1, requestId, "누적값")).nextState;
      const result = accept(afterDelta, terminal);

      expect(result.terminalExpectation).toEqual({
        requestId,
        status,
        lastSeq: 2,
        acceptedText: "누적값",
        error,
      });
      expect(result.nextState).toMatchObject({
        text: "누적값",
        terminalSeen: true,
        expectedTerminal: result.terminalExpectation,
      });
    },
  );

  it.each([
    {
      name: "completed.text",
      terminal: { type: "completed", requestId, seq: 2, text: "누적값" },
    },
    {
      name: "cancelled.partialText",
      terminal: { type: "cancelled", requestId, seq: 2, partialText: "누적값" },
    },
    {
      name: "failed.partialText",
      terminal: {
        type: "failed",
        requestId,
        seq: 2,
        partialText: "누적값",
        error: { code: "MOCK_FAILURE", message: "의도된 실패" },
      },
    },
  ])("rejects legacy terminal payload field $name", ({ terminal }) => {
    const afterStart = accept(createStreamContractState(), started()).nextState;
    const afterDelta = accept(afterStart, delta(1, requestId, "누적값")).nextState;
    const result = reject(afterDelta, terminal);

    expect(result.error).toContain("허용되지 않은 필드");
    expect(result.nextState.text).toBe("누적값");
  });

  it("uses an empty accepted-delta value for a terminal with no deltas", () => {
    const afterStart = accept(createStreamContractState(), started()).nextState;
    const result = accept(afterStart, {
      type: "completed",
      requestId,
      seq: 1,
    });

    expect(result.terminalExpectation?.acceptedText).toBe("");
  });
});

describe("text receipt", () => {
  it.each([
    {
      text: "",
      textBytes: 0,
      textSha256:
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    },
    {
      text: "하나둘",
      textBytes: 9,
      textSha256:
        "28377ce0871248295ea61e8deb878de0cbc9b66b68d64437149928da276e8ada",
    },
  ])("calculates the UTF-8 SHA-256 receipt for '$text'", async (expected) => {
    await expect(createTextReceipt(expected.text)).resolves.toEqual({
      textBytes: expected.textBytes,
      textSha256: expected.textSha256,
    });
  });
});

describe("terminal snapshot contract", () => {
  const expected = {
    requestId,
    status: "completed" as const,
    lastSeq: 3,
    acceptedText: "하나둘",
    error: null,
  };

  const snapshot: StreamSnapshot = {
    requestId,
    status: "completed",
    lastSeq: 3,
    lastAckedSeq: 3,
    inFlight: 0,
    textBytes: 9,
    textSha256:
      "28377ce0871248295ea61e8deb878de0cbc9b66b68d64437149928da276e8ada",
    error: null,
    batchWindowMs: 32,
    effectiveBatchWindowMs: 50,
    maxInFlight: 2,
  };

  it("accepts an exact terminal snapshot receipt", async () => {
    const result = await validateTerminalSnapshot(snapshot, expected);

    expect(result).toEqual({ accepted: true, snapshot });
  });

  it("reports every terminal-field mismatch", async () => {
    const result = await validateTerminalSnapshot(
      {
        ...snapshot,
        requestId: "request-2",
        status: "failed",
        lastSeq: 4,
        lastAckedSeq: 2,
        inFlight: 2,
        textBytes: 3,
        textSha256: "0".repeat(64),
        error: { code: "MOCK_FAILURE", message: "failed" },
      },
      expected,
    );

    expect(result).toEqual({
      accepted: false,
      error:
        "최종 스냅샷 불일치: requestId, status, lastSeq, lastAckedSeq, inFlight, textBytes, textSha256, error",
      mismatches: [
        "requestId",
        "status",
        "lastSeq",
        "lastAckedSeq",
        "inFlight",
        "textBytes",
        "textSha256",
        "error",
      ],
    });
  });

  it("rejects a malformed snapshot payload", async () => {
    const result = await validateTerminalSnapshot(
      { ...snapshot, effectiveBatchWindowMs: -1 },
      expected,
    );

    expect(result).toEqual({
      accepted: false,
      error: "최종 스냅샷 데이터 형식이 유효하지 않습니다.",
      mismatches: ["payload"],
    });
  });

  it.each([
    { name: "negative textBytes", change: { textBytes: -1 } },
    { name: "fractional textBytes", change: { textBytes: 1.5 } },
    { name: "short textSha256", change: { textSha256: "abc" } },
    { name: "uppercase textSha256", change: { textSha256: "A".repeat(64) } },
    { name: "legacy raw text", change: { text: "하나둘" } },
  ])("rejects malformed or raw-text receipt payload: $name", async ({ change }) => {
    const result = await validateTerminalSnapshot(
      { ...snapshot, ...change },
      expected,
    );

    expect(result).toMatchObject({
      accepted: false,
      mismatches: ["payload"],
    });
  });

  it.each([
    { batchWindowMs: 15 },
    { batchWindowMs: 51 },
    { batchWindowMs: 40, effectiveBatchWindowMs: 39 },
    { effectiveBatchWindowMs: 51 },
    { maxInFlight: 0 },
    { maxInFlight: 65 },
    { maxInFlight: 2, inFlight: 3 },
  ])("rejects snapshot bounds outside the stream contract: %o", async (change) => {
    const result = await validateTerminalSnapshot(
      { ...snapshot, ...change },
      expected,
    );

    expect(result).toMatchObject({
      accepted: false,
      mismatches: ["payload"],
    });
  });

  it("matches failed terminal error data exactly", async () => {
    const failedExpected = {
      requestId,
      status: "failed" as const,
      lastSeq: 2,
      acceptedText: "일부",
      error: { code: "MOCK_FAILURE", message: "의도된 실패" },
    };
    const failedSnapshot: StreamSnapshot = {
      ...snapshot,
      status: "failed",
      lastSeq: 2,
      lastAckedSeq: 2,
      textBytes: 6,
      textSha256:
        "32555f5ba712b1fdaf7403eff6d1a96ae377fd2bf903937b5ac1adc275c9b0e8",
      error: { code: "MOCK_FAILURE", message: "의도된 실패" },
    };

    await expect(
      validateTerminalSnapshot(failedSnapshot, failedExpected),
    ).resolves.toMatchObject({ accepted: true });
    expect(
      await validateTerminalSnapshot(
        {
          ...failedSnapshot,
          error: { code: "MOCK_FAILURE", message: "다른 오류" },
        },
        failedExpected,
      ),
    ).toMatchObject({ accepted: false, mismatches: ["error"] });
  });

  it("fails closed when Web Crypto cannot calculate the expected receipt", async () => {
    const rejectingDigest = {
      digest: () => Promise.reject(new Error("digest unavailable")),
    } as Pick<SubtleCrypto, "digest">;

    await expect(
      validateTerminalSnapshot(snapshot, expected, rejectingDigest),
    ).rejects.toThrow("digest unavailable");
  });
});

describe("snapshot lifecycle and recovery contract", () => {
  const emptySha =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
  const textSha =
    "28377ce0871248295ea61e8deb878de0cbc9b66b68d64437149928da276e8ada";
  const base = {
    requestId,
    textBytes: 9,
    textSha256: textSha,
    batchWindowMs: 32,
    effectiveBatchWindowMs: 32,
    maxInFlight: 2,
  };

  function stateAfterAcceptedText(): StreamContractState {
    const afterStart = accept(createStreamContractState(), started()).nextState;
    return accept(afterStart, delta(1, requestId, "하나둘")).nextState;
  }

  it("accepts a queued null/null snapshot and rejects phantom sequence state", () => {
    const queued: StreamSnapshot = {
      requestId,
      status: "queued",
      lastSeq: null,
      lastAckedSeq: null,
      inFlight: 0,
      textBytes: 0,
      textSha256: emptySha,
      error: null,
      batchWindowMs: 32,
      effectiveBatchWindowMs: 32,
      maxInFlight: 2,
    };

    expect(isValidStreamSnapshotPayload(queued)).toBe(true);
    expect(isValidStreamSnapshotPayload({ ...queued, lastSeq: 0 })).toBe(false);
    expect(
      isValidStreamSnapshotPayload({
        ...queued,
        status: "streaming",
      }),
    ).toBe(false);
  });

  it("enforces sequence accounting and status/error coupling", () => {
    const completed: StreamSnapshot = {
      ...base,
      status: "completed",
      lastSeq: 2,
      lastAckedSeq: 1,
      inFlight: 1,
      error: null,
    };

    expect(isValidStreamSnapshotPayload(completed)).toBe(true);
    expect(isValidStreamSnapshotPayload({ ...completed, inFlight: 0 })).toBe(false);
    expect(
      isValidStreamSnapshotPayload({
        ...completed,
        error: { code: "UNEXPECTED", message: "not allowed" },
      }),
    ).toBe(false);
    expect(
      isValidStreamSnapshotPayload({ ...completed, status: "failed" }),
    ).toBe(false);
    expect(
      isValidStreamSnapshotPayload({
        ...completed,
        lastSeq: Number.MAX_SAFE_INTEGER + 1,
      }),
    ).toBe(false);
  });

  it("recovers an omitted terminal event only at the next contiguous sequence", async () => {
    const state = stateAfterAcceptedText();
    const snapshot: StreamSnapshot = {
      ...base,
      status: "completed",
      lastSeq: 2,
      lastAckedSeq: 1,
      inFlight: 1,
      error: null,
    };

    await expect(validateTerminalRecoverySnapshot(snapshot, state)).resolves.toEqual({
      accepted: true,
      snapshot,
      terminalEventDelivered: true,
      expected: {
        requestId,
        status: "completed",
        lastSeq: 2,
        acceptedText: "하나둘",
        error: null,
      },
    });
  });

  it("accepts an already observed terminal without requiring its ACK first", async () => {
    const beforeTerminal = stateAfterAcceptedText();
    const terminalState = accept(beforeTerminal, {
      type: "completed",
      requestId,
      seq: 2,
    }).nextState;
    const snapshot: StreamSnapshot = {
      ...base,
      status: "completed",
      lastSeq: 2,
      lastAckedSeq: 1,
      inFlight: 1,
      error: null,
    };

    await expect(
      validateTerminalRecoverySnapshot(snapshot, terminalState),
    ).resolves.toMatchObject({ accepted: true, terminalEventDelivered: true });
  });

  it("recovers a control-plane-only failure without inventing a terminal sequence", async () => {
    const state = stateAfterAcceptedText();
    const error = { code: "CHANNEL_DELIVERY_FAILED", message: "closed channel" };
    const snapshot: StreamSnapshot = {
      ...base,
      status: "failed",
      lastSeq: 1,
      lastAckedSeq: 1,
      inFlight: 0,
      error,
    };

    await expect(validateTerminalRecoverySnapshot(snapshot, state)).resolves.toEqual({
      accepted: true,
      snapshot,
      terminalEventDelivered: false,
      expected: {
        requestId,
        status: "failed",
        lastSeq: 1,
        acceptedText: "하나둘",
        error,
      },
    });
  });

  it.each([
    {
      name: "missed delta sequence gap",
      change: { lastSeq: 3, lastAckedSeq: 1, inFlight: 2 },
      mismatch: "lastSeq",
    },
    {
      name: "text receipt mismatch",
      change: { textSha256: "0".repeat(64) },
      mismatch: "textSha256",
    },
    {
      name: "wrong request",
      change: { requestId: "request-2" },
      mismatch: "requestId",
    },
  ])("fails closed for $name", async ({ change, mismatch }) => {
    const state = stateAfterAcceptedText();
    const snapshot = {
      ...base,
      status: "completed",
      lastSeq: 2,
      lastAckedSeq: 1,
      inFlight: 1,
      error: null,
      ...change,
    };

    await expect(validateTerminalRecoverySnapshot(snapshot, state)).resolves.toMatchObject({
      accepted: false,
      mismatches: expect.arrayContaining([mismatch]),
    });
  });

  it("rejects a nonterminal recovery snapshot", async () => {
    const state = stateAfterAcceptedText();
    await expect(
      validateTerminalRecoverySnapshot(
        {
          ...base,
          status: "streaming",
          lastSeq: 1,
          lastAckedSeq: 1,
          inFlight: 0,
          error: null,
        },
        state,
      ),
    ).resolves.toMatchObject({ accepted: false, mismatches: ["status"] });
  });

  it("validates the exact release response schema", () => {
    expect(
      validateReleaseStreamResponse({ requestId, released: true }, requestId),
    ).toEqual({ accepted: true });
    expect(
      validateReleaseStreamResponse(
        { requestId, released: true, ignored: true },
        requestId,
      ),
    ).toMatchObject({ accepted: false });
    expect(
      validateReleaseStreamResponse(
        { requestId: "request-2", released: true },
        requestId,
      ),
    ).toMatchObject({ accepted: false });
  });
});
