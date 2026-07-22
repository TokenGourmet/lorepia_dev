import { describe, expect, it, vi } from "vitest";

import {
  PLUGIN_WATCHDOG_LIMITS,
  PluginWatchdog,
  type MonotonicClock,
  type WatchdogPongMessage,
} from "./plugin-watchdog";

class FakeClock implements MonotonicClock {
  value = 0;

  now(): number {
    return this.value;
  }

  advance(milliseconds: number): void {
    this.value += milliseconds;
  }
}

function sessionFactory(...sessionIds: string[]): () => string {
  let index = 0;
  return () => sessionIds[index++] ?? `session-${index}`;
}

function createWatchdog(
  overrides: Partial<ConstructorParameters<typeof PluginWatchdog>[0]> = {},
): { watchdog: PluginWatchdog; clock: FakeClock } {
  const clock = new FakeClock();
  const watchdog = new PluginWatchdog({
    clock,
    sessionIdFactory: sessionFactory("session-a", "session-b", "session-c"),
    pingTimeoutMs: 100,
    missedDeadlineThreshold: 3,
    ...overrides,
  });
  return { watchdog, clock };
}

function issue(watchdog: PluginWatchdog) {
  const result = watchdog.issuePing();
  expect(result.issued).toBe(true);
  if (!result.issued) throw new Error(`ping was not issued: ${result.reason}`);
  return result.ping;
}

function pongFor(
  sessionId: string,
  seq: number,
): WatchdogPongMessage {
  return { type: "lorepia:watchdog:pong", sessionId, seq };
}

describe("PluginWatchdog ping/pong contract", () => {
  it("moves from booting to healthy only after the matching pong", () => {
    const { watchdog, clock } = createWatchdog();

    expect(watchdog.snapshot.state).toEqual({ status: "booting" });
    const ping = issue(watchdog);
    expect(ping).toEqual({
      message: {
        type: "lorepia:watchdog:ping",
        sessionId: "session-a",
        seq: 1,
      },
      issuedAtMs: 0,
      deadlineAtMs: 100,
    });

    clock.advance(99);
    const result = watchdog.receivePong(pongFor("session-a", 1));
    expect(result.accepted).toBe(true);
    expect(result.snapshot).toMatchObject({
      state: { status: "healthy" },
      lastIssuedSeq: 1,
      lastAcceptedSeq: 1,
      pendingSeq: null,
      consecutiveMissedDeadlines: 0,
    });
  });

  it("allows only one outstanding ping", () => {
    const { watchdog } = createWatchdog();
    issue(watchdog);

    expect(watchdog.issuePing()).toEqual({ issued: false, reason: "pending" });
    expect(watchdog.snapshot.lastIssuedSeq).toBe(1);
  });

  it("rejects wrong-session, future, and stale pongs without advancing state", () => {
    const { watchdog } = createWatchdog();
    issue(watchdog);

    expect(watchdog.receivePong(pongFor("session-old", 1))).toMatchObject({
      accepted: false,
      reason: "wrong-session",
    });
    expect(watchdog.receivePong(pongFor("session-a", 2))).toMatchObject({
      accepted: false,
      reason: "future-seq",
    });

    expect(watchdog.receivePong(pongFor("session-a", 1)).accepted).toBe(true);
    expect(watchdog.receivePong(pongFor("session-a", 1))).toMatchObject({
      accepted: false,
      reason: "stale-seq",
    });
    expect(watchdog.snapshot).toMatchObject({
      state: { status: "healthy" },
      lastAcceptedSeq: 1,
      consecutiveMissedDeadlines: 0,
    });
  });

  it("treats the exact deadline as missed and rejects its late pong", () => {
    const { watchdog, clock } = createWatchdog();
    issue(watchdog);
    clock.advance(100);

    const result = watchdog.receivePong(pongFor("session-a", 1));
    expect(result).toMatchObject({
      accepted: false,
      reason: "stale-seq",
      snapshot: {
        state: { status: "unresponsive" },
        pendingSeq: null,
        consecutiveMissedDeadlines: 1,
      },
    });
  });

  it("resets consecutive misses after a valid recovery pong", () => {
    const { watchdog, clock } = createWatchdog();

    issue(watchdog);
    clock.advance(100);
    expect(watchdog.checkDeadline()).toMatchObject({ missed: true, expiredSeq: 1 });
    expect(watchdog.snapshot.state).toEqual({ status: "unresponsive" });

    const recoveryPing = issue(watchdog);
    expect(recoveryPing.message.seq).toBe(2);
    expect(watchdog.receivePong(pongFor("session-a", 2)).accepted).toBe(true);
    expect(watchdog.snapshot).toMatchObject({
      state: { status: "healthy" },
      consecutiveMissedDeadlines: 0,
      lastAcceptedSeq: 2,
    });
  });
});

describe("PluginWatchdog disable and reload behavior", () => {
  it("disables at the consecutive missed-deadline threshold and calls back once", () => {
    const onDisabled = vi.fn();
    const { watchdog, clock } = createWatchdog({
      missedDeadlineThreshold: 2,
      onDisabled,
    });

    issue(watchdog);
    clock.advance(100);
    watchdog.checkDeadline();
    issue(watchdog);
    clock.advance(100);
    const deadline = watchdog.checkDeadline();

    expect(deadline.snapshot.state).toEqual({
      status: "disabled",
      reason: {
        code: "missed-deadline-threshold",
        missedDeadlines: 2,
        threshold: 2,
      },
    });
    expect(onDisabled).toHaveBeenCalledTimes(1);
    expect(onDisabled).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: "session-a",
        reason: {
          code: "missed-deadline-threshold",
          missedDeadlines: 2,
          threshold: 2,
        },
      }),
    );

    watchdog.checkDeadline();
    watchdog.disable("ignored after automatic disable");
    expect(watchdog.issuePing()).toEqual({ issued: false, reason: "disabled" });
    expect(watchdog.receivePong(pongFor("session-a", 2))).toMatchObject({
      accepted: false,
      reason: "disabled",
    });
    expect(onDisabled).toHaveBeenCalledTimes(1);
  });

  it("preserves the first manual disable reason and fires the callback once", () => {
    const onDisabled = vi.fn();
    const { watchdog } = createWatchdog({ onDisabled });

    watchdog.disable("frame violated its sandbox contract");
    watchdog.disable("second reason must not replace the first");

    expect(watchdog.snapshot.state).toEqual({
      status: "disabled",
      reason: {
        code: "manual",
        detail: "frame violated its sandbox contract",
      },
    });
    expect(onDisabled).toHaveBeenCalledTimes(1);
  });

  it("reloads into a fresh booting session and resets every counter", () => {
    const { watchdog, clock } = createWatchdog();
    issue(watchdog);
    clock.advance(100);
    watchdog.checkDeadline();

    const snapshot = watchdog.reload();
    expect(snapshot).toEqual({
      sessionId: "session-b",
      state: { status: "booting" },
      lastIssuedSeq: 0,
      lastAcceptedSeq: 0,
      pendingSeq: null,
      pendingDeadlineAtMs: null,
      consecutiveMissedDeadlines: 0,
      missedDeadlineThreshold: 3,
    });

    expect(issue(watchdog).message).toEqual({
      type: "lorepia:watchdog:ping",
      sessionId: "session-b",
      seq: 1,
    });
  });

  it("never lets late messages from an old frame revive the reloaded session", () => {
    const { watchdog } = createWatchdog();
    issue(watchdog);
    watchdog.reload();
    issue(watchdog);

    const oldFramePong = watchdog.receivePong(pongFor("session-a", 1));
    expect(oldFramePong).toMatchObject({
      accepted: false,
      reason: "wrong-session",
      snapshot: { state: { status: "booting" }, pendingSeq: 1 },
    });

    expect(watchdog.receivePong(pongFor("session-b", 1)).accepted).toBe(true);
    expect(watchdog.snapshot.state).toEqual({ status: "healthy" });
  });

  it("allows one disable callback in each fresh reload session", () => {
    const onDisabled = vi.fn();
    const { watchdog } = createWatchdog({ onDisabled });

    watchdog.disable("first session");
    watchdog.reload();
    watchdog.disable("second session");

    expect(onDisabled).toHaveBeenCalledTimes(2);
    expect(onDisabled.mock.calls.map(([event]) => event.sessionId)).toEqual([
      "session-a",
      "session-b",
    ]);
  });

  it("rejects a reused session ID without mutating the current session", () => {
    const { watchdog } = createWatchdog({
      sessionIdFactory: sessionFactory("same-session", "same-session"),
    });
    const before = watchdog.snapshot;

    expect(() => watchdog.reload()).toThrow(/previously issued/);
    expect(watchdog.snapshot).toEqual(before);
  });
});

describe("PluginWatchdog runtime validation", () => {
  it.each([
    ["pingTimeoutMs", 0],
    ["pingTimeoutMs", PLUGIN_WATCHDOG_LIMITS.maxPingTimeoutMs + 1],
    ["pingTimeoutMs", 1.5],
    ["missedDeadlineThreshold", 0],
    [
      "missedDeadlineThreshold",
      PLUGIN_WATCHDOG_LIMITS.maxMissedDeadlineThreshold + 1,
    ],
    ["missedDeadlineThreshold", Number.NaN],
  ] as const)("rejects invalid %s value %s", (field, value) => {
    expect(() => createWatchdog({ [field]: value })).toThrow(RangeError);
  });

  it.each(["", "x".repeat(PLUGIN_WATCHDOG_LIMITS.maxSessionIdLength + 1), 12])(
    "rejects an invalid generated session ID: %s",
    (sessionId) => {
      expect(() =>
        createWatchdog({ sessionIdFactory: () => sessionId as string }),
      ).toThrow(RangeError);
    },
  );

  it.each([
    null,
    {},
    { type: "lorepia:watchdog:pong", sessionId: "session-a", seq: 0 },
    { type: "lorepia:watchdog:pong", sessionId: "session-a", seq: 1.5 },
    {
      type: "lorepia:watchdog:pong",
      sessionId: "session-a",
      seq: Number.MAX_SAFE_INTEGER + 1,
    },
    { type: "wrong-type", sessionId: "session-a", seq: 1 },
  ])("rejects a malformed pong without consuming the pending ping: %o", (value) => {
    const { watchdog } = createWatchdog();
    issue(watchdog);

    expect(watchdog.receivePong(value)).toMatchObject({
      accepted: false,
      reason: "malformed",
    });
    expect(watchdog.snapshot.pendingSeq).toBe(1);
  });

  it("rejects invalid manual disable details", () => {
    const { watchdog } = createWatchdog();

    expect(() => watchdog.disable("")).toThrow(RangeError);
    expect(() =>
      watchdog.disable("x".repeat(PLUGIN_WATCHDOG_LIMITS.maxDisableDetailLength + 1)),
    ).toThrow(RangeError);
    expect(watchdog.snapshot.state).toEqual({ status: "booting" });
  });

  it("detects a regressing or invalid injected clock", () => {
    const { watchdog, clock } = createWatchdog();
    clock.value = 10;
    issue(watchdog);
    clock.value = 9;
    expect(() => watchdog.checkDeadline()).toThrow(/monotonic/);

    expect(() =>
      createWatchdog({ clock: { now: () => Number.POSITIVE_INFINITY } }),
    ).toThrow(/finite nonnegative/);
  });
});
