import {
  ENGINE_VERSION,
  POLICY_VERSION,
  PROBE_CASE_IDS,
  PROTOCOL_VERSION,
  WASM_MAXIMUM_PAGES,
  WASM_PAGE_BYTES,
  WEDGE_START_ACK_WATCHDOG_MS,
  WORKER_BOOT_WATCHDOG_MS,
  WORKER_EXECUTION_WATCHDOG_MS,
  isCaseCode,
  isCaseOutcome,
  isProbeCaseId,
  outcomeForCode,
  receiptMatchesExpected,
  serializedMessageIsBounded,
  type CaseCode,
  type CaseReceipt,
  type ProbeCaseId,
  type ProbeSuiteReceipt,
  type WorkerReady,
  type WorkerRequest,
  type WorkerResult,
  type WorkerWedgeStarted,
} from "./runner-contract";

export type WorkerFactory = () => Worker;

export interface ExecuteOptions {
  signal?: AbortSignal;
  invocationId?: string;
}

export class RunnerBusyError extends Error {
  constructor() {
    super("RUNNER_BUSY");
    this.name = "RunnerBusyError";
  }
}

function createWorker(): Worker {
  return new Worker(new URL("./script-runner.worker.ts", import.meta.url), {
    type: "module",
    name: "lorepia-script-runner",
  });
}

function newInvocationId(): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function hostReceipt(
  caseId: ProbeCaseId,
  invocationId: string,
  code: CaseCode,
  started: number,
  hostHeartbeatTicks: number,
): CaseReceipt {
  return {
    protocolVersion: PROTOCOL_VERSION,
    policyVersion: POLICY_VERSION,
    engineVersion: ENGINE_VERSION,
    invocationId,
    caseId,
    outcome: outcomeForCode(code),
    code,
    elapsedMs: performance.now() - started,
    hostHeartbeatTicks,
    outputBytes: 0,
    outputSha256: null,
    wasmMemoryBytes: 0,
  };
}

function isExactReady(value: unknown): value is WorkerReady {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    Object.keys(record).sort().join(",") === "protocolVersion,type" &&
    record.type === "READY" &&
    record.protocolVersion === PROTOCOL_VERSION
  );
}

function isExactWedgeStarted(
  value: unknown,
  invocationId: string,
): value is WorkerWedgeStarted {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    !serializedMessageIsBounded(value)
  ) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    Object.keys(record).sort().join(",") ===
      "caseId,invocationId,protocolVersion,type" &&
    record.type === "WEDGE_STARTED" &&
    record.protocolVersion === PROTOCOL_VERSION &&
    record.invocationId === invocationId &&
    record.caseId === "host-watchdog-termination"
  );
}

function parseWorkerResult(
  value: unknown,
  caseId: ProbeCaseId,
  invocationId: string,
  hostHeartbeatTicks: number,
): CaseReceipt | null {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    !serializedMessageIsBounded(value)
  ) {
    return null;
  }
  const envelope = value as Partial<WorkerResult>;
  if (envelope.type !== "RESULT") {
    return null;
  }
  const receipt = envelope.receipt as Record<string, unknown> | undefined;
  if (!receipt || Array.isArray(receipt)) {
    return null;
  }
  const keys = Object.keys(receipt).sort().join(",");
  if (
    keys !==
    "caseId,code,elapsedMs,engineVersion,invocationId,outcome,outputBytes,outputSha256,policyVersion,protocolVersion,wasmMemoryBytes"
  ) {
    return null;
  }
  if (
    receipt.protocolVersion !== PROTOCOL_VERSION ||
    receipt.policyVersion !== POLICY_VERSION ||
    receipt.engineVersion !== ENGINE_VERSION ||
    receipt.invocationId !== invocationId ||
    receipt.caseId !== caseId ||
    !isProbeCaseId(receipt.caseId) ||
    !isCaseOutcome(receipt.outcome) ||
    !isCaseCode(receipt.code) ||
    receipt.outcome !== outcomeForCode(receipt.code) ||
    typeof receipt.elapsedMs !== "number" ||
    !Number.isFinite(receipt.elapsedMs) ||
    receipt.elapsedMs < 0 ||
    typeof receipt.outputBytes !== "number" ||
    !Number.isSafeInteger(receipt.outputBytes) ||
    receipt.outputBytes < 0 ||
    !(
      receipt.outputSha256 === null ||
      (typeof receipt.outputSha256 === "string" &&
        /^[0-9a-f]{64}$/.test(receipt.outputSha256))
    ) ||
    typeof receipt.wasmMemoryBytes !== "number" ||
    !Number.isSafeInteger(receipt.wasmMemoryBytes) ||
    receipt.wasmMemoryBytes < 0 ||
    receipt.wasmMemoryBytes > WASM_MAXIMUM_PAGES * WASM_PAGE_BYTES
  ) {
    return null;
  }

  return {
    ...(receipt as unknown as Omit<CaseReceipt, "hostHeartbeatTicks">),
    hostHeartbeatTicks,
  };
}

export class ScriptRunnerController {
  #busy = false;
  readonly #workerFactory: WorkerFactory;

  constructor(workerFactory: WorkerFactory = createWorker) {
    this.#workerFactory = workerFactory;
  }

  async executeCase(
    caseId: ProbeCaseId,
    options: ExecuteOptions = {},
  ): Promise<CaseReceipt> {
    if (this.#busy) {
      throw new RunnerBusyError();
    }
    this.#busy = true;
    try {
      return await this.#executeCaseUnlocked(caseId, options);
    } finally {
      this.#busy = false;
    }
  }

  async runProbeSuite(): Promise<ProbeSuiteReceipt> {
    const cases: Array<CaseReceipt & { passed: boolean }> = [];
    for (const caseId of PROBE_CASE_IDS) {
      const receipt = await this.executeCase(caseId);
      cases.push({ ...receipt, passed: receiptMatchesExpected(receipt) });
    }
    return {
      protocolVersion: PROTOCOL_VERSION,
      policyVersion: POLICY_VERSION,
      engineVersion: ENGINE_VERSION,
      passed: cases.filter((entry) => entry.passed).length,
      total: cases.length,
      cases,
      defenses: {
        freshWorkerPerInvocation: true,
        fixedMaximumWasmMemory: true,
        engineInterrupt: true,
        hostTerminateFallback: true,
        nativeCommandSurfaceEmpty: true,
        sourceNeverCrossesTauriIpc: true,
      },
    };
  }

  #executeCaseUnlocked(
    caseId: ProbeCaseId,
    options: ExecuteOptions,
  ): Promise<CaseReceipt> {
    const invocationId = options.invocationId ?? newInvocationId();
    if (!/^[0-9a-f]{32}$/.test(invocationId)) {
      throw new Error("INVALID_INVOCATION_ID");
    }
    const started = performance.now();
    const worker = this.#workerFactory();

    return new Promise((resolve) => {
      let settled = false;
      let ready = false;
      let hostHeartbeatTicks = 0;
      let executionTimer: ReturnType<typeof setTimeout> | null = null;
      let wedgeStartTimer: ReturnType<typeof setTimeout> | null = null;
      let heartbeatTimer: ReturnType<typeof setInterval> | null = null;

      const finish = (receipt: CaseReceipt): void => {
        if (settled) return;
        settled = true;
        clearTimeout(bootTimer);
        if (executionTimer) clearTimeout(executionTimer);
        if (wedgeStartTimer) clearTimeout(wedgeStartTimer);
        if (heartbeatTimer) clearInterval(heartbeatTimer);
        options.signal?.removeEventListener("abort", onAbort);
        worker.terminate();
        resolve(receipt);
      };

      const onAbort = (): void => {
        finish(
          hostReceipt(
            caseId,
            invocationId,
            "CANCELLED",
            started,
            hostHeartbeatTicks,
          ),
        );
      };

      const bootTimer = setTimeout(() => {
        finish(
          hostReceipt(
            caseId,
            invocationId,
            "BOOT_TIMEOUT",
            started,
            hostHeartbeatTicks,
          ),
        );
      }, WORKER_BOOT_WATCHDOG_MS);

      options.signal?.addEventListener("abort", onAbort, { once: true });
      if (options.signal?.aborted) {
        onAbort();
        return;
      }

      worker.addEventListener("error", () => {
        finish(
          hostReceipt(
            caseId,
            invocationId,
            "WORKER_FAILURE",
            started,
            hostHeartbeatTicks,
          ),
        );
      });

      worker.addEventListener("message", (event: MessageEvent<unknown>) => {
        if (!ready) {
          if (!isExactReady(event.data)) {
            finish(
              hostReceipt(
                caseId,
                invocationId,
                "CONTRACT_FAILURE",
                started,
                hostHeartbeatTicks,
              ),
            );
            return;
          }
          ready = true;
          clearTimeout(bootTimer);
          heartbeatTimer = setInterval(() => {
            hostHeartbeatTicks += 1;
          }, 10);
          if (caseId === "host-watchdog-termination") {
            wedgeStartTimer = setTimeout(() => {
              finish(
                hostReceipt(
                  caseId,
                  invocationId,
                  "CONTRACT_FAILURE",
                  started,
                  hostHeartbeatTicks,
                ),
              );
            }, WEDGE_START_ACK_WATCHDOG_MS);
          } else {
            executionTimer = setTimeout(() => {
              finish(
                hostReceipt(
                  caseId,
                  invocationId,
                  "HOST_TERMINATED",
                  started,
                  hostHeartbeatTicks,
                ),
              );
            }, WORKER_EXECUTION_WATCHDOG_MS);
          }
          const request: WorkerRequest = {
            type: "RUN",
            protocolVersion: PROTOCOL_VERSION,
            invocationId,
            caseId,
          };
          if (!serializedMessageIsBounded(request)) {
            finish(
              hostReceipt(
                caseId,
                invocationId,
                "CONTRACT_FAILURE",
                started,
                hostHeartbeatTicks,
              ),
            );
            return;
          }
          worker.postMessage(request);
          return;
        }

        if (
          caseId === "host-watchdog-termination" &&
          wedgeStartTimer !== null
        ) {
          if (!isExactWedgeStarted(event.data, invocationId)) {
            finish(
              hostReceipt(
                caseId,
                invocationId,
                "CONTRACT_FAILURE",
                started,
                hostHeartbeatTicks,
              ),
            );
            return;
          }
          clearTimeout(wedgeStartTimer);
          wedgeStartTimer = null;
          executionTimer = setTimeout(() => {
            finish(
              hostReceipt(
                caseId,
                invocationId,
                "HOST_TERMINATED",
                started,
                hostHeartbeatTicks,
              ),
            );
          }, WORKER_EXECUTION_WATCHDOG_MS);
          return;
        }

        const receipt = parseWorkerResult(
          event.data,
          caseId,
          invocationId,
          hostHeartbeatTicks,
        );
        finish(
          receipt ??
            hostReceipt(
              caseId,
              invocationId,
              "CONTRACT_FAILURE",
              started,
              hostHeartbeatTicks,
            ),
        );
      });
    });
  }
}

export const scriptRunner = new ScriptRunnerController();
