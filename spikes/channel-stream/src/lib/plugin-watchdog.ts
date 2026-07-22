export const PLUGIN_WATCHDOG_LIMITS = Object.freeze({
  minPingTimeoutMs: 1,
  maxPingTimeoutMs: 60_000,
  minMissedDeadlineThreshold: 1,
  maxMissedDeadlineThreshold: 100,
  maxSessionIdLength: 128,
  maxDisableDetailLength: 256,
});

export type MonotonicClock = {
  now: () => number;
};

export type PluginWatchdogStatus =
  | "booting"
  | "healthy"
  | "unresponsive"
  | "disabled";

export type PluginWatchdogDisableReason =
  | {
      code: "missed-deadline-threshold";
      missedDeadlines: number;
      threshold: number;
    }
  | { code: "manual"; detail: string }
  | { code: "sequence-exhausted" };

export type PluginWatchdogState =
  | { status: "booting" }
  | { status: "healthy" }
  | { status: "unresponsive" }
  | { status: "disabled"; reason: PluginWatchdogDisableReason };

export type WatchdogPingMessage = {
  type: "lorepia:watchdog:ping";
  sessionId: string;
  seq: number;
};

export type WatchdogPongMessage = {
  type: "lorepia:watchdog:pong";
  sessionId: string;
  seq: number;
};

export type IssuedWatchdogPing = {
  message: WatchdogPingMessage;
  issuedAtMs: number;
  deadlineAtMs: number;
};

export type PluginWatchdogSnapshot = {
  sessionId: string;
  state: PluginWatchdogState;
  lastIssuedSeq: number;
  lastAcceptedSeq: number;
  pendingSeq: number | null;
  pendingDeadlineAtMs: number | null;
  consecutiveMissedDeadlines: number;
  missedDeadlineThreshold: number;
};

export type PluginWatchdogDisabledEvent = {
  sessionId: string;
  reason: PluginWatchdogDisableReason;
  snapshot: PluginWatchdogSnapshot;
};

export type PluginWatchdogOptions = {
  clock: MonotonicClock;
  sessionIdFactory: () => string;
  pingTimeoutMs: number;
  missedDeadlineThreshold: number;
  onDisabled?: (event: PluginWatchdogDisabledEvent) => void;
};

export type IssuePingResult =
  | { issued: true; ping: IssuedWatchdogPing }
  | { issued: false; reason: "pending" | "disabled" };

export type PongRejectionReason =
  | "malformed"
  | "wrong-session"
  | "future-seq"
  | "stale-seq"
  | "disabled";

export type ReceivePongResult =
  | {
      accepted: true;
      seq: number;
      snapshot: PluginWatchdogSnapshot;
    }
  | {
      accepted: false;
      reason: PongRejectionReason;
      snapshot: PluginWatchdogSnapshot;
    };

export type DeadlineCheckResult = {
  missed: boolean;
  expiredSeq: number | null;
  snapshot: PluginWatchdogSnapshot;
};

type PendingPing = {
  seq: number;
  deadlineAtMs: number;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function assertBoundedInteger(
  name: string,
  value: number,
  minimum: number,
  maximum: number,
): void {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new RangeError(
      `${name} must be a safe integer between ${minimum} and ${maximum}`,
    );
  }
}

function assertSessionId(value: unknown): asserts value is string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > PLUGIN_WATCHDOG_LIMITS.maxSessionIdLength
  ) {
    throw new RangeError(
      `sessionId must contain 1-${PLUGIN_WATCHDOG_LIMITS.maxSessionIdLength} characters`,
    );
  }
}

function cloneDisableReason(
  reason: PluginWatchdogDisableReason,
): PluginWatchdogDisableReason {
  return { ...reason };
}

function parsePong(value: unknown): WatchdogPongMessage | null {
  if (
    !isRecord(value) ||
    value.type !== "lorepia:watchdog:pong" ||
    typeof value.sessionId !== "string" ||
    value.sessionId.length === 0 ||
    value.sessionId.length > PLUGIN_WATCHDOG_LIMITS.maxSessionIdLength ||
    !Number.isSafeInteger(value.seq) ||
    (value.seq as number) < 1
  ) {
    return null;
  }

  return value as WatchdogPongMessage;
}

/**
 * DOM-independent iframe watchdog state machine.
 *
 * The adapter owns timers and transport. It calls `issuePing()`, schedules a
 * wake-up for the returned deadline, forwards pong payloads to `receivePong()`,
 * and calls `checkDeadline()` when its timer fires. Only one ping may be
 * outstanding at a time.
 */
export class PluginWatchdog {
  readonly #clock: MonotonicClock;
  readonly #sessionIdFactory: () => string;
  readonly #pingTimeoutMs: number;
  readonly #missedDeadlineThreshold: number;
  readonly #onDisabled: ((event: PluginWatchdogDisabledEvent) => void) | undefined;
  readonly #issuedSessionIds = new Set<string>();

  #sessionId: string;
  #state: PluginWatchdogState = { status: "booting" };
  #lastIssuedSeq = 0;
  #lastAcceptedSeq = 0;
  #pending: PendingPing | null = null;
  #consecutiveMissedDeadlines = 0;
  #disableCallbackFired = false;
  #lastObservedNowMs: number;

  constructor(options: PluginWatchdogOptions) {
    if (!options || typeof options !== "object") {
      throw new TypeError("watchdog options are required");
    }
    if (!options.clock || typeof options.clock.now !== "function") {
      throw new TypeError("clock.now must be a function");
    }
    if (typeof options.sessionIdFactory !== "function") {
      throw new TypeError("sessionIdFactory must be a function");
    }

    assertBoundedInteger(
      "pingTimeoutMs",
      options.pingTimeoutMs,
      PLUGIN_WATCHDOG_LIMITS.minPingTimeoutMs,
      PLUGIN_WATCHDOG_LIMITS.maxPingTimeoutMs,
    );
    assertBoundedInteger(
      "missedDeadlineThreshold",
      options.missedDeadlineThreshold,
      PLUGIN_WATCHDOG_LIMITS.minMissedDeadlineThreshold,
      PLUGIN_WATCHDOG_LIMITS.maxMissedDeadlineThreshold,
    );
    if (options.onDisabled !== undefined && typeof options.onDisabled !== "function") {
      throw new TypeError("onDisabled must be a function");
    }

    this.#clock = options.clock;
    this.#sessionIdFactory = options.sessionIdFactory;
    this.#pingTimeoutMs = options.pingTimeoutMs;
    this.#missedDeadlineThreshold = options.missedDeadlineThreshold;
    this.#onDisabled = options.onDisabled;

    this.#lastObservedNowMs = this.#readInitialNow();
    this.#sessionId = this.#createUniqueSessionId();
    this.#issuedSessionIds.add(this.#sessionId);
  }

  get snapshot(): PluginWatchdogSnapshot {
    return this.#snapshot();
  }

  issuePing(): IssuePingResult {
    const nowMs = this.#readNow();
    this.#expirePendingIfDue(nowMs);

    if (this.#state.status === "disabled") {
      return { issued: false, reason: "disabled" };
    }
    if (this.#pending !== null) {
      return { issued: false, reason: "pending" };
    }
    if (this.#lastIssuedSeq === Number.MAX_SAFE_INTEGER) {
      this.#transitionToDisabled({ code: "sequence-exhausted" });
      return { issued: false, reason: "disabled" };
    }

    const seq = this.#lastIssuedSeq + 1;
    const deadlineAtMs = nowMs + this.#pingTimeoutMs;
    if (!Number.isFinite(deadlineAtMs) || deadlineAtMs <= nowMs) {
      throw new RangeError("clock value is too large to calculate a deadline");
    }

    this.#lastIssuedSeq = seq;
    this.#pending = { seq, deadlineAtMs };

    return {
      issued: true,
      ping: {
        message: {
          type: "lorepia:watchdog:ping",
          sessionId: this.#sessionId,
          seq,
        },
        issuedAtMs: nowMs,
        deadlineAtMs,
      },
    };
  }

  receivePong(value: unknown): ReceivePongResult {
    const pong = parsePong(value);
    const nowMs = this.#readNow();
    this.#expirePendingIfDue(nowMs);

    if (pong === null) return this.#rejectPong("malformed");
    if (pong.sessionId !== this.#sessionId) {
      return this.#rejectPong("wrong-session");
    }
    if (this.#state.status === "disabled") {
      return this.#rejectPong("disabled");
    }
    if (pong.seq > this.#lastIssuedSeq) {
      return this.#rejectPong("future-seq");
    }
    if (this.#pending === null || pong.seq !== this.#pending.seq) {
      return this.#rejectPong("stale-seq");
    }

    this.#lastAcceptedSeq = pong.seq;
    this.#pending = null;
    this.#consecutiveMissedDeadlines = 0;
    this.#state = { status: "healthy" };

    return { accepted: true, seq: pong.seq, snapshot: this.#snapshot() };
  }

  checkDeadline(): DeadlineCheckResult {
    const nowMs = this.#readNow();
    const expiredSeq = this.#expirePendingIfDue(nowMs);
    return {
      missed: expiredSeq !== null,
      expiredSeq,
      snapshot: this.#snapshot(),
    };
  }

  disable(detail: string): PluginWatchdogSnapshot {
    if (
      typeof detail !== "string" ||
      detail.length === 0 ||
      detail.length > PLUGIN_WATCHDOG_LIMITS.maxDisableDetailLength
    ) {
      throw new RangeError(
        `disable detail must contain 1-${PLUGIN_WATCHDOG_LIMITS.maxDisableDetailLength} characters`,
      );
    }

    this.#transitionToDisabled({ code: "manual", detail });
    return this.#snapshot();
  }

  reload(): PluginWatchdogSnapshot {
    const nowMs = this.#readNow();
    const nextSessionId = this.#createUniqueSessionId();

    this.#issuedSessionIds.add(nextSessionId);
    this.#sessionId = nextSessionId;
    this.#state = { status: "booting" };
    this.#lastIssuedSeq = 0;
    this.#lastAcceptedSeq = 0;
    this.#pending = null;
    this.#consecutiveMissedDeadlines = 0;
    this.#disableCallbackFired = false;
    this.#lastObservedNowMs = nowMs;

    return this.#snapshot();
  }

  #readInitialNow(): number {
    const nowMs = this.#clock.now();
    if (!Number.isFinite(nowMs) || nowMs < 0) {
      throw new RangeError("clock.now() must return a finite nonnegative number");
    }
    return nowMs;
  }

  #readNow(): number {
    const nowMs = this.#clock.now();
    if (!Number.isFinite(nowMs) || nowMs < 0) {
      throw new RangeError("clock.now() must return a finite nonnegative number");
    }
    if (nowMs < this.#lastObservedNowMs) {
      throw new RangeError("clock.now() must be monotonic");
    }
    this.#lastObservedNowMs = nowMs;
    return nowMs;
  }

  #createUniqueSessionId(): string {
    const sessionId = this.#sessionIdFactory();
    assertSessionId(sessionId);
    if (this.#issuedSessionIds.has(sessionId)) {
      throw new Error("sessionIdFactory returned a previously issued session ID");
    }
    return sessionId;
  }

  #expirePendingIfDue(nowMs: number): number | null {
    if (this.#pending === null || nowMs < this.#pending.deadlineAtMs) return null;

    const expiredSeq = this.#pending.seq;
    this.#pending = null;
    this.#consecutiveMissedDeadlines += 1;

    if (this.#consecutiveMissedDeadlines >= this.#missedDeadlineThreshold) {
      this.#transitionToDisabled({
        code: "missed-deadline-threshold",
        missedDeadlines: this.#consecutiveMissedDeadlines,
        threshold: this.#missedDeadlineThreshold,
      });
    } else {
      this.#state = { status: "unresponsive" };
    }

    return expiredSeq;
  }

  #transitionToDisabled(reason: PluginWatchdogDisableReason): void {
    if (this.#state.status === "disabled") return;

    this.#pending = null;
    this.#state = { status: "disabled", reason: cloneDisableReason(reason) };

    if (!this.#disableCallbackFired) {
      this.#disableCallbackFired = true;
      this.#onDisabled?.({
        sessionId: this.#sessionId,
        reason: cloneDisableReason(reason),
        snapshot: this.#snapshot(),
      });
    }
  }

  #rejectPong(reason: PongRejectionReason): ReceivePongResult {
    return { accepted: false, reason, snapshot: this.#snapshot() };
  }

  #snapshot(): PluginWatchdogSnapshot {
    const state: PluginWatchdogState =
      this.#state.status === "disabled"
        ? {
            status: "disabled",
            reason: cloneDisableReason(this.#state.reason),
          }
        : { status: this.#state.status };

    return {
      sessionId: this.#sessionId,
      state,
      lastIssuedSeq: this.#lastIssuedSeq,
      lastAcceptedSeq: this.#lastAcceptedSeq,
      pendingSeq: this.#pending?.seq ?? null,
      pendingDeadlineAtMs: this.#pending?.deadlineAtMs ?? null,
      consecutiveMissedDeadlines: this.#consecutiveMissedDeadlines,
      missedDeadlineThreshold: this.#missedDeadlineThreshold,
    };
  }
}
