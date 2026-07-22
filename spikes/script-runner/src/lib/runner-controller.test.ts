import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ENGINE_VERSION,
  MAX_WORKER_MESSAGE_BYTES,
  POLICY_VERSION,
  PROTOCOL_VERSION,
  WASM_INITIAL_PAGES,
  WASM_MAXIMUM_PAGES,
  WASM_PAGE_BYTES,
  WEDGE_START_ACK_WATCHDOG_MS,
  WORKER_BOOT_WATCHDOG_MS,
  WORKER_EXECUTION_WATCHDOG_MS,
  type ProbeCaseId,
  type WorkerRequest,
} from "./runner-contract";
import {
  RunnerBusyError,
  ScriptRunnerController,
} from "./runner-controller";

type Listener = (event: MessageEvent<unknown> | Event) => void;

class FakeWorker {
  readonly listeners = new Map<string, Listener[]>();
  terminated = false;
  terminationCount = 0;
  postCount = 0;
  onPost: (request: WorkerRequest) => void = () => {};

  addEventListener(type: string, listener: EventListenerOrEventListenerObject): void {
    const callback =
      typeof listener === "function"
        ? (listener as Listener)
        : ((event: Event) => listener.handleEvent(event));
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), callback]);
  }

  postMessage(value: unknown): void {
    this.postCount += 1;
    this.onPost(value as WorkerRequest);
  }

  terminate(): void {
    this.terminated = true;
    this.terminationCount += 1;
  }

  message(data: unknown): void {
    this.emit("message", { data } as MessageEvent<unknown>);
  }

  ready(overrides: Record<string, unknown> = {}): void {
    this.message({
      type: "READY",
      protocolVersion: PROTOCOL_VERSION,
      ...overrides,
    });
  }

  result(
    caseId: ProbeCaseId,
    invocationId: string,
    receiptOverrides: Record<string, unknown> = {},
    envelopeOverrides: Record<string, unknown> = {},
  ): void {
    this.message({
      type: "RESULT",
      receipt: {
        protocolVersion: PROTOCOL_VERSION,
        policyVersion: POLICY_VERSION,
        engineVersion: ENGINE_VERSION,
        invocationId,
        caseId,
        outcome: "ALLOWED",
        code: "ALLOWED_RESULT",
        elapsedMs: 4,
        outputBytes: 28,
        outputSha256:
          "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        wasmMemoryBytes: WASM_INITIAL_PAGES * WASM_PAGE_BYTES,
        ...receiptOverrides,
      },
      ...envelopeOverrides,
    });
  }

  wedgeStarted(
    request: WorkerRequest,
    overrides: Record<string, unknown> = {},
  ): void {
    this.message({
      type: "WEDGE_STARTED",
      protocolVersion: PROTOCOL_VERSION,
      invocationId: request.invocationId,
      caseId: "host-watchdog-termination",
      ...overrides,
    });
  }

  error(): void {
    this.emit("error", new Event("error"));
  }

  messageError(): void {
    this.emit("messageerror", new Event("messageerror"));
  }

  emit(type: string, event: MessageEvent<unknown> | Event): void {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

function setupWorker(
  configure: (worker: FakeWorker) => void,
): { controller: ScriptRunnerController; worker: FakeWorker } {
  const worker = new FakeWorker();
  configure(worker);
  const controller = new ScriptRunnerController(() => worker as unknown as Worker);
  return { controller, worker };
}

function setupWorkerSequence(
  configurations: Array<(worker: FakeWorker) => void>,
): { controller: ScriptRunnerController; workers: FakeWorker[] } {
  const workers: FakeWorker[] = [];
  const controller = new ScriptRunnerController(() => {
    const configure = configurations[workers.length];
    if (!configure) throw new Error("UNEXPECTED_WORKER_CREATION");
    const worker = new FakeWorker();
    workers.push(worker);
    configure(worker);
    return worker as unknown as Worker;
  });
  return { controller, workers };
}

function validResult(
  caseId: ProbeCaseId,
  invocationId: string,
): Record<string, unknown> {
  return {
    type: "RESULT",
    receipt: {
      protocolVersion: PROTOCOL_VERSION,
      policyVersion: POLICY_VERSION,
      engineVersion: ENGINE_VERSION,
      invocationId,
      caseId,
      outcome: "ALLOWED",
      code: "ALLOWED_RESULT",
      elapsedMs: 4,
      outputBytes: 28,
      outputSha256:
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      wasmMemoryBytes: WASM_INITIAL_PAGES * WASM_PAGE_BYTES,
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("script runner host controller", () => {
  it("binds a bounded result to the exact invocation and terminates the Worker", async () => {
    const invocationId = "01".repeat(16);
    const { controller, worker } = setupWorker((candidate) => {
      queueMicrotask(() => candidate.ready());
      candidate.onPost = (request) => {
        queueMicrotask(() =>
          candidate.result(request.caseId, request.invocationId),
        );
      };
    });

    await expect(
      controller.executeCase("allowed-baseline", { invocationId }),
    ).resolves.toMatchObject({
      invocationId,
      caseId: "allowed-baseline",
      code: "ALLOWED_RESULT",
    });
    expect(worker.terminated).toBe(true);
  });

  it.each([
    ["null", null],
    ["array", []],
    ["wrong type", { type: "RESULT", protocolVersion: PROTOCOL_VERSION }],
    ["wrong protocol", { type: "READY", protocolVersion: 2 }],
    [
      "extra key",
      { type: "READY", protocolVersion: PROTOCOL_VERSION, extra: true },
    ],
    [
      "oversized extra key",
      {
        type: "READY",
        protocolVersion: PROTOCOL_VERSION,
        padding: "x".repeat(MAX_WORKER_MESSAGE_BYTES),
      },
    ],
  ])(
    "rejects a malformed or forged READY message: %s",
    async (_label, payload) => {
      const invocationId = "10".repeat(16);
      const { controller, worker } = setupWorker((candidate) => {
        queueMicrotask(() => candidate.message(payload));
      });

      await expect(
        controller.executeCase("allowed-baseline", { invocationId }),
      ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
      expect(worker.terminated).toBe(true);
      expect(worker.postCount).toBe(0);
    },
  );

  it.each([
    ["missing receipt", { type: "RESULT" }],
    ["array receipt", { type: "RESULT", receipt: [] }],
    ["wrong envelope type", { type: "READY", receipt: {} }],
  ])("rejects a malformed RESULT envelope: %s", async (_label, payload) => {
    const invocationId = "11".repeat(16);
    const { controller, worker } = setupWorker((candidate) => {
      queueMicrotask(() => candidate.ready());
      candidate.onPost = () => queueMicrotask(() => candidate.message(payload));
    });

    await expect(
      controller.executeCase("allowed-baseline", { invocationId }),
    ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
    expect(worker.terminated).toBe(true);
  });

  it("rejects extra keys on both the RESULT envelope and receipt", async () => {
    const envelopeInvocationId = "12".repeat(16);
    const receiptInvocationId = "13".repeat(16);
    const { controller, workers } = setupWorkerSequence([
      (candidate) => {
        queueMicrotask(() => candidate.ready());
        candidate.onPost = (request) => {
          queueMicrotask(() =>
            candidate.result(request.caseId, request.invocationId, {}, {
              extra: true,
            }),
          );
        };
      },
      (candidate) => {
        queueMicrotask(() => candidate.ready());
        candidate.onPost = (request) => {
          queueMicrotask(() =>
            candidate.result(request.caseId, request.invocationId, {
              extra: true,
            }),
          );
        };
      },
    ]);

    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: envelopeInvocationId,
      }),
    ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: receiptInvocationId,
      }),
    ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
    expect(workers).toHaveLength(2);
    expect(workers.every((worker) => worker.terminated)).toBe(true);
  });

  it("rejects forged RESULT metadata and out-of-range numeric fields", async () => {
    const mutations: Array<Record<string, unknown>> = [
      { protocolVersion: 2 },
      { policyVersion: "forged-policy" },
      { engineVersion: "forged-engine" },
      { caseId: "infinite-loop" },
      { outcome: "FORGED" },
      { code: "FORGED" },
      { elapsedMs: Number.NaN },
      { elapsedMs: -1 },
      { outputBytes: Number.MAX_SAFE_INTEGER + 1 },
      { outputSha256: "not-a-sha256" },
      { wasmMemoryBytes: (WASM_MAXIMUM_PAGES + 1) * WASM_PAGE_BYTES },
    ];
    const configurations = mutations.map(
      (mutation) => (candidate: FakeWorker): void => {
        queueMicrotask(() => candidate.ready());
        candidate.onPost = (request) => {
          queueMicrotask(() =>
            candidate.result(request.caseId, request.invocationId, mutation),
          );
        };
      },
    );
    const { controller } = setupWorkerSequence(configurations);

    for (let index = 0; index < mutations.length; index += 1) {
      const receipt = await controller.executeCase("allowed-baseline", {
        invocationId: (20 + index).toString(16).padStart(2, "0").repeat(16),
      });
      expect(receipt.code, JSON.stringify(mutations[index])).toBe(
        "CONTRACT_FAILURE",
      );
    }
  });

  it("rejects an oversized RESULT before accepting its otherwise valid receipt", async () => {
    const invocationId = "30".repeat(16);
    const { controller, worker } = setupWorker((candidate) => {
      queueMicrotask(() => candidate.ready());
      candidate.onPost = (request) => {
        queueMicrotask(() =>
          candidate.message({
            ...validResult(request.caseId, request.invocationId),
            padding: "x".repeat(MAX_WORKER_MESSAGE_BYTES),
          }),
        );
      };
    });

    await expect(
      controller.executeCase("allowed-baseline", { invocationId }),
    ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
    expect(worker.terminated).toBe(true);
  });

  it("rejects a stale or cross-owned result", async () => {
    const invocationId = "02".repeat(16);
    const { controller } = setupWorker((candidate) => {
      queueMicrotask(() => candidate.ready());
      candidate.onPost = (request) => {
        queueMicrotask(() => candidate.result(request.caseId, "ff".repeat(16)));
      };
    });

    await expect(
      controller.executeCase("allowed-baseline", { invocationId }),
    ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
  });

  it("rejects a forged code and outcome pairing", async () => {
    const invocationId = "07".repeat(16);
    const { controller } = setupWorker((candidate) => {
      queueMicrotask(() => candidate.ready());
      candidate.onPost = (request) => {
        queueMicrotask(() =>
          candidate.result(request.caseId, request.invocationId, {
            code: "MEMORY_LIMIT",
            outcome: "ALLOWED",
          }),
        );
      };
    });

    await expect(
      controller.executeCase("allowed-baseline", { invocationId }),
    ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
  });

  it("externally terminates a wedged Worker while the host heartbeat advances", async () => {
    vi.useFakeTimers();
    const { controller, worker } = setupWorker((candidate) => {
      queueMicrotask(() => candidate.ready());
      candidate.onPost = (request) => {
        queueMicrotask(() => candidate.wedgeStarted(request));
      };
    });
    const pending = controller.executeCase("host-watchdog-termination", {
      invocationId: "03".repeat(16),
    });
    await vi.advanceTimersByTimeAsync(WORKER_EXECUTION_WATCHDOG_MS + 10);

    await expect(pending).resolves.toMatchObject({
      code: "HOST_TERMINATED",
    });
    const receipt = await pending;
    expect(receipt.hostHeartbeatTicks).toBeGreaterThan(0);
    expect(worker.terminated).toBe(true);
  });

  it("does not report termination when the Worker never confirms the wedge", async () => {
    vi.useFakeTimers();
    const { controller, worker } = setupWorker((candidate) => {
      queueMicrotask(() => candidate.ready());
    });
    const pending = controller.executeCase("host-watchdog-termination", {
      invocationId: "08".repeat(16),
    });
    await vi.advanceTimersByTimeAsync(WEDGE_START_ACK_WATCHDOG_MS + 10);

    await expect(pending).resolves.toMatchObject({
      code: "CONTRACT_FAILURE",
    });
    expect(worker.terminated).toBe(true);
  });

  it("rejects malformed, forged, extra-key, and oversized WEDGE_STARTED messages", async () => {
    const mutations: Array<(request: WorkerRequest) => unknown> = [
      () => null,
      () => ({ type: "WEDGE_STARTED" }),
      (request) => ({
        type: "WEDGE_STARTED",
        protocolVersion: 2,
        invocationId: request.invocationId,
        caseId: "host-watchdog-termination",
      }),
      (request) => ({
        type: "WEDGE_STARTED",
        protocolVersion: PROTOCOL_VERSION,
        invocationId: "ff".repeat(16),
        caseId: request.caseId,
      }),
      (request) => ({
        type: "WEDGE_STARTED",
        protocolVersion: PROTOCOL_VERSION,
        invocationId: request.invocationId,
        caseId: "allowed-baseline",
      }),
      (request) => ({
        type: "WEDGE_STARTED",
        protocolVersion: PROTOCOL_VERSION,
        invocationId: request.invocationId,
        caseId: request.caseId,
        extra: true,
      }),
      (request) => ({
        type: "WEDGE_STARTED",
        protocolVersion: PROTOCOL_VERSION,
        invocationId: request.invocationId,
        caseId: request.caseId,
        padding: "x".repeat(MAX_WORKER_MESSAGE_BYTES),
      }),
    ];
    const { controller, workers } = setupWorkerSequence(
      mutations.map(
        (messageFor) => (candidate: FakeWorker): void => {
          queueMicrotask(() => candidate.ready());
          candidate.onPost = (request) => {
            queueMicrotask(() => candidate.message(messageFor(request)));
          };
        },
      ),
    );

    for (let index = 0; index < mutations.length; index += 1) {
      const receipt = await controller.executeCase(
        "host-watchdog-termination",
        { invocationId: (40 + index).toString(16).repeat(16) },
      );
      expect(receipt.code, `wedge mutation ${index}`).toBe("CONTRACT_FAILURE");
    }
    expect(workers.every((worker) => worker.terminated)).toBe(true);
  });

  it("ignores replayed or duplicate messages after the invocation settles", async () => {
    const invocationId = "50".repeat(16);
    const { controller, worker } = setupWorker((candidate) => {
      queueMicrotask(() => candidate.ready());
      candidate.onPost = (request) => {
        queueMicrotask(() =>
          candidate.result(request.caseId, request.invocationId),
        );
      };
    });

    const receipt = await controller.executeCase("allowed-baseline", {
      invocationId,
    });
    worker.result("allowed-baseline", invocationId);
    worker.message({
      type: "RESULT",
      receipt: { extra: "replay-after-settlement" },
    });

    expect(receipt.code).toBe("ALLOWED_RESULT");
    expect(worker.terminationCount).toBe(1);
  });

  it("cancels by terminating the current Worker", async () => {
    const abortController = new AbortController();
    const { controller, worker } = setupWorker(() => {});
    const pending = controller.executeCase("infinite-loop", {
      invocationId: "04".repeat(16),
      signal: abortController.signal,
    });
    abortController.abort();

    await expect(pending).resolves.toMatchObject({ code: "CANCELLED" });
    expect(worker.terminated).toBe(true);
  });

  it("cancels an already-aborted invocation before READY or RUN", async () => {
    const abortController = new AbortController();
    abortController.abort();
    const { controller, worker } = setupWorker(() => {});

    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: "51".repeat(16),
        signal: abortController.signal,
      }),
    ).resolves.toMatchObject({ code: "CANCELLED" });
    expect(worker.terminated).toBe(true);
    expect(worker.postCount).toBe(0);
  });

  it("fails closed on Worker error and message deserialization error", async () => {
    const { controller, workers } = setupWorkerSequence([
      (candidate) => queueMicrotask(() => candidate.error()),
      (candidate) => queueMicrotask(() => candidate.messageError()),
      (candidate) => {
        queueMicrotask(() => candidate.ready());
        candidate.onPost = (request) => {
          queueMicrotask(() =>
            candidate.result(request.caseId, request.invocationId),
          );
        };
      },
    ]);

    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: "52".repeat(16),
      }),
    ).resolves.toMatchObject({ code: "WORKER_FAILURE" });
    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: "53".repeat(16),
      }),
    ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: "57".repeat(16),
      }),
    ).resolves.toMatchObject({ code: "ALLOWED_RESULT" });
    expect(workers).toHaveLength(3);
    expect(workers.every((worker) => worker.terminated)).toBe(true);
  });

  it("terminates a Worker at the boot deadline and recovers fresh", async () => {
    vi.useFakeTimers();
    const { controller, workers } = setupWorkerSequence([
      () => {},
      (candidate) => {
        queueMicrotask(() => candidate.ready());
        candidate.onPost = (request) => {
          queueMicrotask(() =>
            candidate.result(request.caseId, request.invocationId),
          );
        };
      },
    ]);
    const pending = controller.executeCase("allowed-baseline", {
      invocationId: "54".repeat(16),
    });
    await vi.advanceTimersByTimeAsync(WORKER_BOOT_WATCHDOG_MS + 1);

    await expect(pending).resolves.toMatchObject({ code: "BOOT_TIMEOUT" });
    expect(workers[0]?.terminated).toBe(true);
    expect(workers[0]?.postCount).toBe(0);
    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: "58".repeat(16),
      }),
    ).resolves.toMatchObject({ code: "ALLOWED_RESULT" });
    expect(workers).toHaveLength(2);
    expect(workers[1]?.terminated).toBe(true);
  });

  it("recovers with a fresh Worker after a contract failure", async () => {
    const { controller, workers } = setupWorkerSequence([
      (candidate) => {
        queueMicrotask(() =>
          candidate.ready({ extra: "malformed-first-worker" }),
        );
      },
      (candidate) => {
        queueMicrotask(() => candidate.ready());
        candidate.onPost = (request) => {
          queueMicrotask(() =>
            candidate.result(request.caseId, request.invocationId),
          );
        };
      },
    ]);

    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: "55".repeat(16),
      }),
    ).resolves.toMatchObject({ code: "CONTRACT_FAILURE" });
    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: "56".repeat(16),
      }),
    ).resolves.toMatchObject({ code: "ALLOWED_RESULT" });
    expect(workers).toHaveLength(2);
    expect(workers.every((worker) => worker.terminated)).toBe(true);
  });

  it("admits only one memory-bounded Worker at a time", async () => {
    const abortController = new AbortController();
    const { controller } = setupWorker(() => {});
    const first = controller.executeCase("infinite-loop", {
      invocationId: "05".repeat(16),
      signal: abortController.signal,
    });

    await expect(
      controller.executeCase("allowed-baseline", {
        invocationId: "06".repeat(16),
      }),
    ).rejects.toBeInstanceOf(RunnerBusyError);
    abortController.abort();
    await first;
  });
});
