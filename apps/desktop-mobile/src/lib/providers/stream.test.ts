import { describe, expect, it, vi } from "vitest";

import {
  FIRST_CHAT_RECOVERY_POLL_MS,
  FIRST_CHAT_STREAM_PROTOCOL_ERROR,
  FIRST_CHAT_STREAM_UNKNOWN_ERROR,
  publicStreamErrorMessage,
  startFirstChatStream,
  type CommandInvoker,
  type FirstChatStreamCallbacks,
  type StreamChannelFactory,
} from "./stream";

const requestId = `provider-${"a".repeat(32)}`;
const controlToken = "b".repeat(32);
const profile = { providerId: "openai" as const, modelId: "model-example" };

function deferred<T>(): {
  promise: Promise<T>;
  resolve(value: T): void;
} {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

function ackResponse(seq: number): Record<string, unknown> {
  return {
    requestId,
    acknowledgedThrough: seq,
    inFlight: 0,
  };
}

function terminalSnapshot(
  type: "completed" | "cancelled" | "failed",
  seq: number,
): Record<string, unknown> {
  const terminal =
    type === "completed"
      ? { type, seq, reason: null, usage: null }
      : type === "failed"
        ? {
            type,
            seq,
            error: {
              code: "PROVIDER_FAILED",
              message: "bounded",
              httpStatus: null,
              retriable: false,
            },
          }
        : { type, seq };
  return {
    requestId,
    lastSentSeq: seq,
    acknowledgedThrough: seq - 1,
    inFlight: 1,
    cancelRequested: type === "cancelled",
    terminal,
  };
}

function callbacks(log: string[]): FirstChatStreamCallbacks {
  return {
    onDelta(text, kind) {
      log.push(`delta:${kind}:${text}`);
    },
    onTerminal(terminal) {
      log.push(`terminal:${terminal}`);
    },
    onError(message) {
      log.push(`error:${message}`);
    },
  };
}

describe("first chat provider stream", () => {
  it("buffers pre-start events and ACKs every applied event in sequence", async () => {
    const start = deferred<unknown>();
    const log: string[] = [];
    let deliver: (event: unknown) => void = () => undefined;
    const createChannel: StreamChannelFactory = (handler) => {
      deliver = handler;
      return { channel: "fake" };
    };
    const invokeCommand: CommandInvoker = vi.fn(async (command, args) => {
      log.push(
        command === "ack_provider_stream"
          ? `ack:${String(args.seq)}`
          : `command:${command}`,
      );
      if (command === "start_provider_stream") {
        deliver({ type: "started", requestId, seq: 0, maxInFlight: 4 });
        deliver({ type: "text_delta", requestId, seq: 1, text: "안녕" });
        deliver({
          type: "completed",
          requestId,
          seq: 2,
          reason: "stop",
          usage: null,
        });
        return start.promise;
      }
      if (command === "ack_provider_stream") {
        return ackResponse(args.seq as number);
      }
      if (command === "get_provider_stream_snapshot") {
        return terminalSnapshot("completed", 2);
      }
      throw new Error(`unexpected ${command}`);
    });

    const handle = startFirstChatStream(
      profile,
      "hello",
      callbacks(log),
      { invokeCommand, createChannel },
    );
    expect(log).toEqual(["command:start_provider_stream"]);

    start.resolve({ requestId, controlToken });
    await expect(handle.done).resolves.toBe("completed");

    expect(log).toEqual([
      "command:start_provider_stream",
      "ack:0",
      "delta:text:안녕",
      "ack:1",
      "command:get_provider_stream_snapshot",
      "ack:2",
      "terminal:completed",
    ]);
    expect(invokeCommand).toHaveBeenCalledWith("start_provider_stream", {
      profile: expect.objectContaining({
        providerId: "openai",
        modelId: "model-example",
      }),
      userMessage: "hello",
      onEvent: { channel: "fake" },
    });
  });

  it("keeps the control token private and sends exact cancellation arguments", async () => {
    let deliver: (event: unknown) => void = () => undefined;
    const createChannel: StreamChannelFactory = (handler) => {
      deliver = handler;
      return { channel: "fake" };
    };
    const invokeCommand: CommandInvoker = vi.fn(async (command) => {
      if (command === "start_provider_stream") {
        return { requestId, controlToken };
      }
      if (command === "cancel_provider_stream") {
        return { requestId, accepted: true };
      }
      throw new Error(`unexpected ${command}`);
    });

    const handle = startFirstChatStream(
      profile,
      "hello",
      callbacks([]),
      { invokeCommand, createChannel },
    );
    await handle.cancel();

    expect(invokeCommand).toHaveBeenCalledWith("cancel_provider_stream", {
      requestId,
      controlToken,
    });
    expect(
      vi.mocked(invokeCommand).mock.calls.filter(
        ([command]) => command === "cancel_provider_stream",
      ),
    ).toHaveLength(1);
    expect(JSON.stringify(handle)).not.toContain(controlToken);
    expect(deliver).toBeTypeOf("function");
  });

  it("keeps the short recovery interval after a post-cancel event", async () => {
    vi.useFakeTimers();
    try {
      let deliver: (event: unknown) => void = () => undefined;
      let snapshotCalls = 0;
      const invokeCommand: CommandInvoker = vi.fn(async (command, args) => {
        if (command === "start_provider_stream") {
          return { requestId, controlToken };
        }
        if (command === "cancel_provider_stream") {
          return { requestId, accepted: true };
        }
        if (command === "ack_provider_stream") {
          return ackResponse(args.seq as number);
        }
        if (command === "get_provider_stream_snapshot") {
          snapshotCalls += 1;
          return terminalSnapshot("cancelled", 1);
        }
        throw new Error(`unexpected ${command}`);
      });

      const handle = startFirstChatStream(profile, "hello", callbacks([]), {
        invokeCommand,
        createChannel(handler) {
          deliver = handler;
          return {};
        },
      });
      await Promise.resolve();
      await handle.cancel();

      deliver({ type: "started", requestId, seq: 0, maxInFlight: 4 });
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();

      await vi.advanceTimersByTimeAsync(999);
      expect(snapshotCalls).toBe(0);
      await vi.advanceTimersByTimeAsync(1);

      await expect(handle.done).resolves.toBe("cancelled");
      expect(snapshotCalls).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("fails closed and cancels on a forged request identity", async () => {
    const log: string[] = [];
    let deliver: (event: unknown) => void = () => undefined;
    const invokeCommand: CommandInvoker = vi.fn(async (command) => {
      if (command === "start_provider_stream") {
        return { requestId, controlToken };
      }
      if (command === "cancel_provider_stream") {
        return { requestId, accepted: true };
      }
      throw new Error(`unexpected ${command}`);
    });

    const handle = startFirstChatStream(profile, "hello", callbacks(log), {
      invokeCommand,
      createChannel(handler) {
        deliver = handler;
        return {};
      },
    });
    await Promise.resolve();
    deliver({
      type: "started",
      requestId: `provider-${"c".repeat(32)}`,
      seq: 0,
      maxInFlight: 4,
    });

    await expect(handle.done).resolves.toBe("failed");
    expect(log).toEqual([
      `error:${FIRST_CHAT_STREAM_PROTOCOL_ERROR}`,
      "terminal:failed",
    ]);
    expect(invokeCommand).not.toHaveBeenCalledWith(
      "ack_provider_stream",
      expect.anything(),
    );
    expect(invokeCommand).toHaveBeenCalledWith("cancel_provider_stream", {
      requestId,
      controlToken,
    });
  });

  it("ACKs failure, obtains the terminal snapshot, and never reflects raw errors", async () => {
    const log: string[] = [];
    let deliver: (event: unknown) => void = () => undefined;
    const commands: string[] = [];
    const invokeCommand: CommandInvoker = vi.fn(async (command, args) => {
      commands.push(command);
      if (command === "start_provider_stream") {
        queueMicrotask(() => {
          deliver({ type: "started", requestId, seq: 0, maxInFlight: 4 });
          deliver({
            type: "failed",
            requestId,
            seq: 1,
            error: {
              code: "PROVIDER_FAILED",
              message: "secret-provider-body",
              httpStatus: null,
              retriable: false,
            },
          });
        });
        return { requestId, controlToken };
      }
      if (command === "ack_provider_stream") {
        return ackResponse(args.seq as number);
      }
      if (command === "get_provider_stream_snapshot") {
        return terminalSnapshot("failed", 1);
      }
      throw new Error(`unexpected ${command}`);
    });

    const handle = startFirstChatStream(profile, "hello", callbacks(log), {
      invokeCommand,
      createChannel(handler) {
        deliver = handler;
        return {};
      },
    });
    await expect(handle.done).resolves.toBe("failed");

    expect(log).toEqual([
      `error:${FIRST_CHAT_STREAM_UNKNOWN_ERROR}`,
      "terminal:failed",
    ]);
    expect(JSON.stringify(log)).not.toContain("secret-provider-body");
    expect(commands).toEqual([
      "start_provider_stream",
      "ack_provider_stream",
      "get_provider_stream_snapshot",
      "ack_provider_stream",
    ]);
  });

  it("recovers a lost terminal event from its authenticated snapshot", async () => {
    vi.useFakeTimers();
    try {
      const log: string[] = [];
      let deliver: (event: unknown) => void = () => undefined;
      const commands: string[] = [];
      const invokeCommand: CommandInvoker = vi.fn(async (command, args) => {
        commands.push(
          command === "ack_provider_stream"
            ? `${command}:${String(args.seq)}`
            : command,
        );
        if (command === "start_provider_stream") {
          deliver({ type: "started", requestId, seq: 0, maxInFlight: 4 });
          return { requestId, controlToken };
        }
        if (command === "ack_provider_stream") {
          return ackResponse(args.seq as number);
        }
        if (command === "get_provider_stream_snapshot") {
          return {
            requestId,
            lastSentSeq: 1,
            acknowledgedThrough: 0,
            inFlight: 1,
            cancelRequested: false,
            terminal: { type: "completed", seq: 1, reason: "stop", usage: null },
          };
        }
        throw new Error(`unexpected ${command}`);
      });

      const handle = startFirstChatStream(profile, "hello", callbacks(log), {
        invokeCommand,
        createChannel(handler) {
          deliver = handler;
          return {};
        },
      });
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(FIRST_CHAT_RECOVERY_POLL_MS);

      await expect(handle.done).resolves.toBe("completed");
      expect(log).toEqual(["terminal:completed"]);
      expect(commands).toEqual([
        "start_provider_stream",
        "ack_provider_stream:0",
        "get_provider_stream_snapshot",
        "ack_provider_stream:1",
      ]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("survives a poll that observes terminal state before delayed Channel delivery", async () => {
    vi.useFakeTimers();
    try {
      const log: string[] = [];
      let deliver: (event: unknown) => void = () => undefined;
      let acknowledged = -1;
      const invokeCommand: CommandInvoker = vi.fn(async (command, args) => {
        if (command === "start_provider_stream") {
          deliver({ type: "started", requestId, seq: 0, maxInFlight: 4 });
          return { requestId, controlToken };
        }
        if (command === "ack_provider_stream") {
          acknowledged = args.seq as number;
          return ackResponse(acknowledged);
        }
        if (command === "get_provider_stream_snapshot") {
          return {
            requestId,
            lastSentSeq: 2,
            acknowledgedThrough: acknowledged,
            inFlight: 2 - acknowledged,
            cancelRequested: false,
            terminal: { type: "completed", seq: 2, reason: "stop", usage: null },
          };
        }
        throw new Error(`unexpected ${command}`);
      });

      const handle = startFirstChatStream(profile, "hello", callbacks(log), {
        invokeCommand,
        createChannel(handler) {
          deliver = handler;
          return {};
        },
      });
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(FIRST_CHAT_RECOVERY_POLL_MS);

      deliver({ type: "text_delta", requestId, seq: 1, text: "late" });
      deliver({
        type: "completed",
        requestId,
        seq: 2,
        reason: "stop",
        usage: null,
      });
      await expect(handle.done).resolves.toBe("completed");
      expect(log).toEqual(["delta:text:late", "terminal:completed"]);
      expect(log).not.toContain(`error:${FIRST_CHAT_STREAM_PROTOCOL_ERROR}`);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not publish completed before a terminal snapshot is proven", async () => {
    const log: string[] = [];
    let deliver: (event: unknown) => void = () => undefined;
    const invokeCommand: CommandInvoker = vi.fn(async (command, args) => {
      if (command === "start_provider_stream") {
        queueMicrotask(() => {
          deliver({ type: "started", requestId, seq: 0, maxInFlight: 4 });
          deliver({
            type: "completed",
            requestId,
            seq: 1,
            reason: "stop",
            usage: null,
          });
        });
        return { requestId, controlToken };
      }
      if (command === "ack_provider_stream") {
        return ackResponse(args.seq as number);
      }
      if (command === "get_provider_stream_snapshot") {
        return terminalSnapshot("cancelled", 1);
      }
      if (command === "cancel_provider_stream") {
        return { requestId, accepted: false };
      }
      throw new Error(`unexpected ${command}`);
    });

    const handle = startFirstChatStream(profile, "hello", callbacks(log), {
      invokeCommand,
      createChannel(handler) {
        deliver = handler;
        return {};
      },
    });
    await expect(handle.done).resolves.toBe("failed");

    expect(log).toEqual([
      `error:${FIRST_CHAT_STREAM_PROTOCOL_ERROR}`,
      "terminal:failed",
    ]);
    expect(log).not.toContain("terminal:completed");
  });
});

describe("public stream errors", () => {
  it("uses fixed copy and never reflects unknown native content", () => {
    expect(
      publicStreamErrorMessage({
        code: "SOMETHING_NEW",
        message: "sk-do-not-reflect",
      }),
    ).toBe(FIRST_CHAT_STREAM_UNKNOWN_ERROR);
    expect(publicStreamErrorMessage({ code: "CREDENTIAL_NOT_CONFIGURED" })).toContain(
      "API 키",
    );
  });
});
