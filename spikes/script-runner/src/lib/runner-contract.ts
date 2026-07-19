export const PROTOCOL_VERSION = 1 as const;
export const POLICY_VERSION = "m1-script-runner-v1" as const;
export const ENGINE_VERSION = "quickjs-emscripten-0.32.0" as const;

export const SOURCE_MAX_BYTES = 64 * 1024;
export const INPUT_MAX_BYTES = 16 * 1024;
export const OUTPUT_MAX_BYTES = 16 * 1024;
export const QUICKJS_MEMORY_LIMIT_BYTES = 8 * 1024 * 1024;
export const QUICKJS_STACK_LIMIT_BYTES = 256 * 1024;
export const WASM_INITIAL_PAGES = 256;
export const WASM_MAXIMUM_PAGES = 512;
export const WASM_PAGE_BYTES = 64 * 1024;
export const ENGINE_DEADLINE_MS = 50;
export const ENGINE_WARMUP_DEADLINE_MS = 500;
export const WORKER_EXECUTION_WATCHDOG_MS = 500;
export const WEDGE_START_ACK_WATCHDOG_MS = 250;
export const WORKER_BOOT_WATCHDOG_MS = 20_000;
export const MAX_WORKER_MESSAGE_BYTES = 4_096;

export const CASE_DEFINITIONS = {
  "allowed-baseline": "allowed",
  "infinite-loop": "infinite-loop",
  "recovery-after-infinite-loop": "allowed",
  "recursive-pressure": "recursive-pressure",
  "recovery-after-recursive-pressure": "allowed",
  "allocator-pressure": "allocator-pressure",
  "recovery-after-allocator-pressure": "allowed",
  "forbidden-globals-absent": "forbidden-globals",
  "recovery-after-forbidden-globals": "allowed",
  "oversized-output": "oversized-output",
  "recovery-after-oversized-output": "allowed",
  "raw-error-redacted": "script-error",
  "recovery-after-raw-error": "allowed",
  "host-watchdog-termination": "host-watchdog",
  "recovery-after-host-watchdog": "allowed",
} as const;

export type ProbeCaseId = keyof typeof CASE_DEFINITIONS;
export type FixtureId = (typeof CASE_DEFINITIONS)[ProbeCaseId];

export const PROBE_CASE_IDS = Object.freeze(
  Object.keys(CASE_DEFINITIONS) as ProbeCaseId[],
);

export type CaseCode =
  | "ALLOWED_RESULT"
  | "ENGINE_INTERRUPTED"
  | "STACK_LIMIT"
  | "MEMORY_LIMIT"
  | "FORBIDDEN_GLOBALS_ABSENT"
  | "OUTPUT_LIMIT"
  | "SCRIPT_ERROR"
  | "HOST_TERMINATED"
  | "CANCELLED"
  | "BOOT_TIMEOUT"
  | "WORKER_FAILURE"
  | "CONTRACT_FAILURE";

export type CaseOutcome =
  | "ALLOWED"
  | "INTERRUPTED"
  | "ABSENT"
  | "REJECTED"
  | "TERMINATED"
  | "ERROR";

const CASE_CODES: ReadonlySet<string> = new Set<CaseCode>([
  "ALLOWED_RESULT",
  "ENGINE_INTERRUPTED",
  "STACK_LIMIT",
  "MEMORY_LIMIT",
  "FORBIDDEN_GLOBALS_ABSENT",
  "OUTPUT_LIMIT",
  "SCRIPT_ERROR",
  "HOST_TERMINATED",
  "CANCELLED",
  "BOOT_TIMEOUT",
  "WORKER_FAILURE",
  "CONTRACT_FAILURE",
]);

const CASE_OUTCOMES: ReadonlySet<string> = new Set<CaseOutcome>([
  "ALLOWED",
  "INTERRUPTED",
  "ABSENT",
  "REJECTED",
  "TERMINATED",
  "ERROR",
]);

export function isCaseCode(value: unknown): value is CaseCode {
  return typeof value === "string" && CASE_CODES.has(value);
}

export function isCaseOutcome(value: unknown): value is CaseOutcome {
  return typeof value === "string" && CASE_OUTCOMES.has(value);
}

export function outcomeForCode(code: CaseCode): CaseOutcome {
  switch (code) {
    case "ALLOWED_RESULT":
      return "ALLOWED";
    case "ENGINE_INTERRUPTED":
    case "STACK_LIMIT":
    case "MEMORY_LIMIT":
      return "INTERRUPTED";
    case "FORBIDDEN_GLOBALS_ABSENT":
      return "ABSENT";
    case "OUTPUT_LIMIT":
    case "SCRIPT_ERROR":
      return "REJECTED";
    case "HOST_TERMINATED":
    case "CANCELLED":
      return "TERMINATED";
    case "BOOT_TIMEOUT":
    case "WORKER_FAILURE":
    case "CONTRACT_FAILURE":
      return "ERROR";
  }
}

export interface CaseReceipt {
  protocolVersion: typeof PROTOCOL_VERSION;
  policyVersion: typeof POLICY_VERSION;
  engineVersion: typeof ENGINE_VERSION;
  invocationId: string;
  caseId: ProbeCaseId;
  outcome: CaseOutcome;
  code: CaseCode;
  elapsedMs: number;
  hostHeartbeatTicks: number;
  outputBytes: number;
  outputSha256: string | null;
  wasmMemoryBytes: number;
}

export interface ProbeSuiteReceipt {
  protocolVersion: typeof PROTOCOL_VERSION;
  policyVersion: typeof POLICY_VERSION;
  engineVersion: typeof ENGINE_VERSION;
  passed: number;
  total: number;
  cases: Array<CaseReceipt & { passed: boolean }>;
  defenses: {
    freshWorkerPerInvocation: true;
    fixedMaximumWasmMemory: true;
    engineInterrupt: true;
    hostTerminateFallback: true;
    nativeCommandSurfaceEmpty: true;
    sourceNeverCrossesTauriIpc: true;
  };
}

export type WorkerRequest = {
  type: "RUN";
  protocolVersion: typeof PROTOCOL_VERSION;
  invocationId: string;
  caseId: ProbeCaseId;
};

export type WorkerReady = {
  type: "READY";
  protocolVersion: typeof PROTOCOL_VERSION;
};

export type WorkerResult = {
  type: "RESULT";
  receipt: Omit<CaseReceipt, "hostHeartbeatTicks">;
};

export type WorkerWedgeStarted = {
  type: "WEDGE_STARTED";
  protocolVersion: typeof PROTOCOL_VERSION;
  invocationId: string;
  caseId: "host-watchdog-termination";
};

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

export function assertBoundedCandidate(source: string, inputJson: string): void {
  if (utf8ByteLength(source) > SOURCE_MAX_BYTES) {
    throw new Error("SOURCE_LIMIT");
  }
  if (utf8ByteLength(inputJson) > INPUT_MAX_BYTES) {
    throw new Error("INPUT_LIMIT");
  }
}

export function isProbeCaseId(value: unknown): value is ProbeCaseId {
  return typeof value === "string" && Object.hasOwn(CASE_DEFINITIONS, value);
}

export function expectedCode(caseId: ProbeCaseId): CaseCode {
  if (caseId.startsWith("recovery-") || caseId === "allowed-baseline") {
    return "ALLOWED_RESULT";
  }

  switch (caseId) {
    case "infinite-loop":
      return "ENGINE_INTERRUPTED";
    case "recursive-pressure":
      return "STACK_LIMIT";
    case "allocator-pressure":
      return "MEMORY_LIMIT";
    case "forbidden-globals-absent":
      return "FORBIDDEN_GLOBALS_ABSENT";
    case "oversized-output":
      return "OUTPUT_LIMIT";
    case "raw-error-redacted":
      return "SCRIPT_ERROR";
    case "host-watchdog-termination":
      return "HOST_TERMINATED";
  }
  throw new Error(`unmapped probe case: ${caseId}`);
}

export function receiptMatchesExpected(receipt: CaseReceipt): boolean {
  if (
    receipt.code !== expectedCode(receipt.caseId) ||
    receipt.outcome !== outcomeForCode(receipt.code)
  ) {
    return false;
  }
  if (
    receipt.caseId === "host-watchdog-termination" &&
    receipt.hostHeartbeatTicks < 1
  ) {
    return false;
  }
  return true;
}

export function serializedMessageIsBounded(value: unknown): boolean {
  try {
    return utf8ByteLength(JSON.stringify(value)) <= MAX_WORKER_MESSAGE_BYTES;
  } catch {
    return false;
  }
}
