import { invoke } from "@tauri-apps/api/core";

export const LUA_M1_COMMAND = "run_lua_limits_m1_probe" as const;
export const LUA_POLICY_VERSION = "m1-lua-limits-v1" as const;
export const LUA_FIXTURE_CATALOG_SHA256 =
  "9ea567d6901ec39412e73f439ee9ea7d47538baea4d1a92cd409c9f3e9b97db5" as const;
export const LUA_MLUA_VERSION = "0.12.0" as const;
export const LUA_VERSION = "Lua 5.4" as const;

export const LUA_LIMITS = {
  deadlineMs: 50,
  instructionCap: 100_000,
  hookCadence: 1_000,
  memoryCeilingBytes: 8_388_608,
  maxSerializedBytes: 4_096,
} as const;

export const LUA_CASE_CONTRACT = [
  {
    caseId: "allowed-baseline",
    outcome: "ALLOWED",
    codes: ["ALLOWED_RESULT"],
  },
  {
    caseId: "infinite-loop",
    outcome: "INTERRUPTED",
    codes: ["INSTRUCTION_LIMIT", "DEADLINE_LIMIT"],
  },
  {
    caseId: "recovery-after-infinite-loop",
    outcome: "ALLOWED",
    codes: ["ALLOWED_RESULT"],
  },
  {
    caseId: "recursive-pressure",
    outcome: "INTERRUPTED",
    codes: ["STACK_LIMIT", "INSTRUCTION_LIMIT", "DEADLINE_LIMIT"],
  },
  {
    caseId: "recovery-after-recursive-pressure",
    outcome: "ALLOWED",
    codes: ["ALLOWED_RESULT"],
  },
  {
    caseId: "allocator-pressure",
    outcome: "INTERRUPTED",
    codes: ["MEMORY_LIMIT"],
  },
  {
    caseId: "recovery-after-allocator-pressure",
    outcome: "ALLOWED",
    codes: ["ALLOWED_RESULT"],
  },
  {
    caseId: "forbidden-globals-absent",
    outcome: "ABSENT",
    codes: ["FORBIDDEN_GLOBALS_ABSENT"],
  },
  {
    caseId: "recovery-after-forbidden-globals",
    outcome: "ALLOWED",
    codes: ["ALLOWED_RESULT"],
  },
  {
    caseId: "bypass-surfaces-absent",
    outcome: "ABSENT",
    codes: ["BYPASS_SURFACES_ABSENT"],
  },
  {
    caseId: "recovery-after-bypass-surfaces",
    outcome: "ALLOWED",
    codes: ["ALLOWED_RESULT"],
  },
] as const;

export const LUA_PROBE_FAILURE_CODES = [
  "PROBE_BUSY",
  "PROBE_CONTRACT_FAILED",
  "INTERNAL_STATE",
] as const;

export type LuaCaseOutcome = "ALLOWED" | "INTERRUPTED" | "ABSENT";
export type LuaCaseCode =
  | "ALLOWED_RESULT"
  | "INSTRUCTION_LIMIT"
  | "DEADLINE_LIMIT"
  | "STACK_LIMIT"
  | "MEMORY_LIMIT"
  | "FORBIDDEN_GLOBALS_ABSENT"
  | "BYPASS_SURFACES_ABSENT";
export type LuaProbeFailureCode = (typeof LUA_PROBE_FAILURE_CODES)[number];

export type LuaCaseEvidence = {
  caseId: (typeof LUA_CASE_CONTRACT)[number]["caseId"];
  outcome: LuaCaseOutcome;
  code: LuaCaseCode;
  result: 55 | null;
  elapsedMicros: number;
  hookCount: number;
  instructionEstimate: number;
  usedMemoryBytes: number;
  observedPeakMemoryBytes: number;
  memoryCeilingBytes: (typeof LUA_LIMITS)["memoryCeilingBytes"];
};

export type LuaProbeSuccess = {
  protocolVersion: 1;
  policyVersion: typeof LUA_POLICY_VERSION;
  fixtureCatalogSha256: typeof LUA_FIXTURE_CATALOG_SHA256;
  mluaVersion: typeof LUA_MLUA_VERSION;
  luaVersion: typeof LUA_VERSION;
  limits: typeof LUA_LIMITS;
  cases: LuaCaseEvidence[];
  defenses: {
    freshStatePerCase: true;
    textModeOnly: true;
    forbiddenGlobalsAbsent: true;
    bypassSurfacesAbsent: true;
    processLockNonblocking: true;
  };
};

export type LuaProbeFailure = {
  protocolVersion: 1;
  code: LuaProbeFailureCode;
};

export type NativeInvoke = (command: typeof LUA_M1_COMMAND) => Promise<unknown>;

const SUCCESS_KEYS = [
  "cases",
  "defenses",
  "fixtureCatalogSha256",
  "limits",
  "luaVersion",
  "mluaVersion",
  "policyVersion",
  "protocolVersion",
] as const;
const LIMIT_KEYS = [
  "deadlineMs",
  "hookCadence",
  "instructionCap",
  "maxSerializedBytes",
  "memoryCeilingBytes",
] as const;
const CASE_KEYS = [
  "caseId",
  "code",
  "elapsedMicros",
  "hookCount",
  "instructionEstimate",
  "memoryCeilingBytes",
  "observedPeakMemoryBytes",
  "outcome",
  "result",
  "usedMemoryBytes",
] as const;
const DEFENSE_KEYS = [
  "bypassSurfacesAbsent",
  "forbiddenGlobalsAbsent",
  "freshStatePerCase",
  "processLockNonblocking",
  "textModeOnly",
] as const;
const FAILURE_KEYS = ["code", "protocolVersion"] as const;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;

export class LuaProbeProtocolError extends Error {
  constructor() {
    super("Lua limits probe returned an invalid bounded response");
    this.name = "LuaProbeProtocolError";
  }
}

export class LuaProbeCommandError extends Error {
  readonly failure: LuaProbeFailure;

  constructor(failure: LuaProbeFailure) {
    super("Lua limits probe command failed");
    this.name = "LuaProbeCommandError";
    this.failure = failure;
  }
}

function failProtocol(): never {
  throw new LuaProbeProtocolError();
}

function record(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    failProtocol();
  }
  return value as Record<string, unknown>;
}

function exactKeys(value: Record<string, unknown>, keys: readonly string[]): void {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    failProtocol();
  }
}

function boundedInteger(value: unknown, maximum = Number.MAX_SAFE_INTEGER): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0 || (value as number) > maximum) {
    failProtocol();
  }
  return value as number;
}

function enforceSerializedResponseLimit(value: unknown): void {
  let serialized: string | undefined;
  try {
    serialized = JSON.stringify(value);
  } catch {
    failProtocol();
  }
  if (
    serialized === undefined ||
    new TextEncoder().encode(serialized).byteLength > LUA_LIMITS.maxSerializedBytes
  ) {
    failProtocol();
  }
}

function parseLimits(value: unknown): typeof LUA_LIMITS {
  const candidate = record(value);
  exactKeys(candidate, LIMIT_KEYS);
  for (const [key, expected] of Object.entries(LUA_LIMITS)) {
    if (candidate[key] !== expected) failProtocol();
  }
  return candidate as typeof LUA_LIMITS;
}

function parseCase(
  value: unknown,
  expected: (typeof LUA_CASE_CONTRACT)[number],
): LuaCaseEvidence {
  const candidate = record(value);
  exactKeys(candidate, CASE_KEYS);
  if (
    candidate.caseId !== expected.caseId ||
    candidate.outcome !== expected.outcome ||
    !expected.codes.some((code) => code === candidate.code)
  ) {
    failProtocol();
  }

  const expectsResult = expected.outcome === "ALLOWED";
  if (candidate.result !== (expectsResult ? 55 : null)) failProtocol();

  const elapsedMicros = boundedInteger(candidate.elapsedMicros);
  const maximumHooks = Math.ceil(LUA_LIMITS.instructionCap / LUA_LIMITS.hookCadence);
  const hookCount = boundedInteger(candidate.hookCount, maximumHooks);
  const instructionEstimate = boundedInteger(
    candidate.instructionEstimate,
    LUA_LIMITS.instructionCap,
  );
  if (instructionEstimate !== hookCount * LUA_LIMITS.hookCadence) failProtocol();

  const usedMemoryBytes = boundedInteger(
    candidate.usedMemoryBytes,
    LUA_LIMITS.memoryCeilingBytes,
  );
  const observedPeakMemoryBytes = boundedInteger(
    candidate.observedPeakMemoryBytes,
    LUA_LIMITS.memoryCeilingBytes,
  );
  if (
    observedPeakMemoryBytes < usedMemoryBytes ||
    candidate.memoryCeilingBytes !== LUA_LIMITS.memoryCeilingBytes
  ) {
    failProtocol();
  }

  if (
    candidate.code === "INSTRUCTION_LIMIT" &&
    (hookCount !== maximumHooks ||
      instructionEstimate !== LUA_LIMITS.instructionCap ||
      elapsedMicros >= LUA_LIMITS.deadlineMs * 1_000)
  ) {
    failProtocol();
  }
  if (
    candidate.code === "DEADLINE_LIMIT" &&
    (hookCount < 1 || elapsedMicros < LUA_LIMITS.deadlineMs * 1_000)
  ) {
    failProtocol();
  }
  if (
    (expected.outcome === "ALLOWED" || expected.outcome === "ABSENT") &&
    (elapsedMicros > LUA_LIMITS.deadlineMs * 1_000 ||
      instructionEstimate >= LUA_LIMITS.instructionCap)
  ) {
    failProtocol();
  }
  if (
    (candidate.code === "STACK_LIMIT" || candidate.code === "MEMORY_LIMIT") &&
    instructionEstimate >= LUA_LIMITS.instructionCap
  ) {
    failProtocol();
  }

  return {
    caseId: expected.caseId,
    outcome: expected.outcome,
    code: candidate.code as LuaCaseCode,
    result: candidate.result as 55 | null,
    elapsedMicros,
    hookCount,
    instructionEstimate,
    usedMemoryBytes,
    observedPeakMemoryBytes,
    memoryCeilingBytes: LUA_LIMITS.memoryCeilingBytes,
  };
}

function parseDefenses(value: unknown): LuaProbeSuccess["defenses"] {
  const candidate = record(value);
  exactKeys(candidate, DEFENSE_KEYS);
  for (const key of DEFENSE_KEYS) {
    if (candidate[key] !== true) failProtocol();
  }
  return candidate as LuaProbeSuccess["defenses"];
}

export function parseLuaProbeSuccess(value: unknown): LuaProbeSuccess {
  enforceSerializedResponseLimit(value);
  const candidate = record(value);
  exactKeys(candidate, SUCCESS_KEYS);
  if (
    candidate.protocolVersion !== 1 ||
    candidate.policyVersion !== LUA_POLICY_VERSION ||
    candidate.fixtureCatalogSha256 !== LUA_FIXTURE_CATALOG_SHA256 ||
    !SHA256_PATTERN.test(candidate.fixtureCatalogSha256 as string) ||
    candidate.mluaVersion !== LUA_MLUA_VERSION ||
    candidate.luaVersion !== LUA_VERSION ||
    !Array.isArray(candidate.cases) ||
    candidate.cases.length !== LUA_CASE_CONTRACT.length
  ) {
    failProtocol();
  }

  return {
    protocolVersion: 1,
    policyVersion: LUA_POLICY_VERSION,
    fixtureCatalogSha256: LUA_FIXTURE_CATALOG_SHA256,
    mluaVersion: LUA_MLUA_VERSION,
    luaVersion: LUA_VERSION,
    limits: parseLimits(candidate.limits),
    cases: candidate.cases.map((entry, index) =>
      parseCase(entry, LUA_CASE_CONTRACT[index]),
    ),
    defenses: parseDefenses(candidate.defenses),
  };
}

export function parseLuaProbeFailure(value: unknown): LuaProbeFailure {
  enforceSerializedResponseLimit(value);
  const candidate = record(value);
  exactKeys(candidate, FAILURE_KEYS);
  if (
    candidate.protocolVersion !== 1 ||
    !LUA_PROBE_FAILURE_CODES.includes(candidate.code as LuaProbeFailureCode)
  ) {
    failProtocol();
  }
  return { protocolVersion: 1, code: candidate.code as LuaProbeFailureCode };
}

function invokeLuaLimitsM1Command(): Promise<unknown> {
  return invoke<unknown>("run_lua_limits_m1_probe");
}

export async function runLuaLimitsM1Probe(
  nativeInvoke: NativeInvoke = invokeLuaLimitsM1Command,
): Promise<LuaProbeSuccess> {
  let rawResult: unknown;
  try {
    rawResult = await nativeInvoke(LUA_M1_COMMAND);
  } catch (rawFailure) {
    let failure: LuaProbeFailure;
    try {
      failure = parseLuaProbeFailure(rawFailure);
    } catch {
      throw new LuaProbeProtocolError();
    }
    throw new LuaProbeCommandError(failure);
  }
  return parseLuaProbeSuccess(rawResult);
}
