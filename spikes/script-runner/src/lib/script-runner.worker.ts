/// <reference lib="webworker" />

import {
  CASE_DEFINITIONS,
  ENGINE_VERSION,
  MAX_WORKER_MESSAGE_BYTES,
  POLICY_VERSION,
  PROTOCOL_VERSION,
  isProbeCaseId,
  outcomeForCode,
  serializedMessageIsBounded,
  type WorkerReady,
  type WorkerRequest,
  type WorkerResult,
  type WorkerWedgeStarted,
} from "./runner-contract";
import {
  executeFixture,
  prepareEngine,
  type PreparedEngine,
} from "./quickjs-engine";

const workerScope = self as unknown as DedicatedWorkerGlobalScope;

function requestIsExact(value: unknown): value is WorkerRequest {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record).sort();
  return (
    keys.join(",") === "caseId,invocationId,protocolVersion,type" &&
    record.type === "RUN" &&
    record.protocolVersion === PROTOCOL_VERSION &&
    typeof record.invocationId === "string" &&
    /^[0-9a-f]{32}$/.test(record.invocationId) &&
    isProbeCaseId(record.caseId)
  );
}

function postBoundedMessage(
  message: WorkerResult | WorkerWedgeStarted,
): void {
  if (!serializedMessageIsBounded(message)) {
    throw new Error(`worker receipt exceeds ${MAX_WORKER_MESSAGE_BYTES} bytes`);
  }
  workerScope.postMessage(message);
}

async function executeRequest(
  request: WorkerRequest,
  engine: PreparedEngine,
): Promise<void> {
  const fixtureId = CASE_DEFINITIONS[request.caseId];
  if (fixtureId === "host-watchdog") {
    postBoundedMessage({
      type: "WEDGE_STARTED",
      protocolVersion: PROTOCOL_VERSION,
      invocationId: request.invocationId,
      caseId: "host-watchdog-termination",
    });
    // This deliberately wedges only the disposable worker. The trusted host
    // must remain responsive and terminate this worker from outside.
    while (true) {
      // Intentionally empty.
    }
  }

  const execution = await executeFixture(fixtureId, engine);
  postBoundedMessage({
    type: "RESULT",
    receipt: {
      protocolVersion: PROTOCOL_VERSION,
      policyVersion: POLICY_VERSION,
      engineVersion: ENGINE_VERSION,
      invocationId: request.invocationId,
      caseId: request.caseId,
      outcome: outcomeForCode(execution.code),
      code: execution.code,
      elapsedMs: execution.elapsedMs,
      outputBytes: execution.outputBytes,
      outputSha256: execution.outputSha256,
      wasmMemoryBytes: execution.wasmMemoryBytes,
    },
  });
  workerScope.close();
}

const engine = await prepareEngine();

workerScope.addEventListener("message", (event: MessageEvent<unknown>) => {
  if (!requestIsExact(event.data) || !serializedMessageIsBounded(event.data)) {
    throw new Error("invalid worker request");
  }
  void executeRequest(event.data, engine);
});

const ready: WorkerReady = {
  type: "READY",
  protocolVersion: PROTOCOL_VERSION,
};
workerScope.postMessage(ready);
