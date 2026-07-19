import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ENGINE_VERSION,
  POLICY_VERSION,
  PROTOCOL_VERSION,
  WASM_INITIAL_PAGES,
  WASM_PAGE_BYTES,
  WEDGE_START_ACK_WATCHDOG_MS,
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
  onPost: (request: WorkerRequest) => void = () => {};

  addEventListener(type: string, listener: EventListenerOrEventListenerObject): void {
    const callback =
      typeof listener === "function"
        ? (listener as Listener)
        : ((event: Event) => listener.handleEvent(event));
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), callback]);
  }

  postMessage(value: unknown): void {
    this.onPost(value as WorkerRequest);
  }

  terminate(): void {
    this.terminated = true;
  }

  ready(): void {
    this.emit("message", {
      data: { type: "READY", protocolVersion: PROTOCOL_VERSION },
    } as MessageEvent<unknown>);
  }

  result(
    caseId: ProbeCaseId,
    invocationId: string,
    overrides: Record<string, unknown> = {},
  ): void {
    this.emit("message", {
      data: {
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
          ...overrides,
        },
      },
    } as MessageEvent<unknown>);
  }

  wedgeStarted(request: WorkerRequest): void {
    this.emit("message", {
      data: {
        type: "WEDGE_STARTED",
        protocolVersion: PROTOCOL_VERSION,
        invocationId: request.invocationId,
        caseId: "host-watchdog-termination",
      },
    } as MessageEvent<unknown>);
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
