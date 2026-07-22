import { describe, expect, it, vi } from "vitest";

import {
  LUA_CASE_CONTRACT,
  LUA_FIXTURE_CATALOG_SHA256,
  LUA_LIMITS,
  LUA_M1_COMMAND,
  LUA_MLUA_VERSION,
  LUA_POLICY_VERSION,
  LUA_PROBE_FAILURE_CODES,
  LUA_VERSION,
  LuaProbeCommandError,
  LuaProbeProtocolError,
  parseLuaProbeFailure,
  parseLuaProbeSuccess,
  runLuaLimitsM1Probe,
  type LuaCaseEvidence,
  type LuaProbeSuccess,
} from "./lua-probe";

function validCase(
  expected: (typeof LUA_CASE_CONTRACT)[number],
  index: number,
): LuaCaseEvidence {
  const allowed = expected.outcome === "ALLOWED";
  const hookCount = expected.caseId === "infinite-loop" ? 100 : index;
  const usedMemoryBytes = 32_768 + index * 1_024;
  return {
    caseId: expected.caseId,
    outcome: expected.outcome,
    code: expected.codes[0],
    result: allowed ? 55 : null,
    elapsedMicros: 100 + index,
    hookCount,
    instructionEstimate: hookCount * LUA_LIMITS.hookCadence,
    usedMemoryBytes,
    observedPeakMemoryBytes: usedMemoryBytes + 1_024,
    memoryCeilingBytes: LUA_LIMITS.memoryCeilingBytes,
  };
}

function validSuccess(): LuaProbeSuccess {
  return {
    protocolVersion: 1,
    policyVersion: LUA_POLICY_VERSION,
    fixtureCatalogSha256: LUA_FIXTURE_CATALOG_SHA256,
    mluaVersion: LUA_MLUA_VERSION,
    luaVersion: LUA_VERSION,
    limits: { ...LUA_LIMITS },
    cases: LUA_CASE_CONTRACT.map(validCase),
    defenses: {
      freshStatePerCase: true,
      textModeOnly: true,
      forbiddenGlobalsAbsent: true,
      bypassSurfacesAbsent: true,
      processLockNonblocking: true,
    },
  };
}

function expectProtocolFailure(value: unknown): void {
  expect(() => parseLuaProbeSuccess(value)).toThrow(LuaProbeProtocolError);
}

describe("strict Lua limits success receipt", () => {
  it("accepts the exact bounded receipt", () => {
    const value = validSuccess();
    expect(new TextEncoder().encode(JSON.stringify(value)).byteLength).toBeLessThanOrEqual(
      LUA_LIMITS.maxSerializedBytes,
    );
    expect(parseLuaProbeSuccess(value)).toEqual(value);
  });

  it.each([
    ["protocol", { protocolVersion: 2 }],
    ["policy", { policyVersion: "m1-lua-limits-v2" }],
    ["catalog hash", { fixtureCatalogSha256: "f".repeat(64) }],
    ["mlua", { mluaVersion: "0.12.1" }],
    ["Lua", { luaVersion: "Lua 5.5" }],
  ])("rejects a mismatched %s", (_label, mutation) => {
    expectProtocolFailure({ ...validSuccess(), ...mutation });
  });

  it("rejects missing and unknown top-level fields", () => {
    const missing = { ...validSuccess() } as Record<string, unknown>;
    delete missing.defenses;
    expectProtocolFailure(missing);
    expectProtocolFailure({ ...validSuccess(), rawError: "native detail" });
  });

  it.each(Object.entries(LUA_LIMITS))("pins the exact %s limit", (key, value) => {
    expectProtocolFailure({
      ...validSuccess(),
      limits: { ...LUA_LIMITS, [key]: value + 1 },
    });
  });

  it("rejects missing, reordered, duplicate, and expanded cases", () => {
    const cases = validSuccess().cases;
    expectProtocolFailure({ ...validSuccess(), cases: cases.slice(0, -1) });
    expectProtocolFailure({
      ...validSuccess(),
      cases: [cases[1], cases[0], ...cases.slice(2)],
    });
    expectProtocolFailure({
      ...validSuccess(),
      cases: [cases[0], cases[0], ...cases.slice(2)],
    });
    expectProtocolFailure({
      ...validSuccess(),
      cases: cases.map((entry, index) =>
        index === 0 ? { ...entry, fixtureSource: "return 55" } : entry,
      ),
    });
  });

  it("allows only the documented nondeterministic bounded stop codes", () => {
    const infinite = validSuccess();
    infinite.cases[1] = {
      ...infinite.cases[1],
      code: "DEADLINE_LIMIT",
      elapsedMicros: 50_000,
    };
    expect(parseLuaProbeSuccess(infinite)).toEqual(infinite);

    const recursive = validSuccess();
    recursive.cases[3] = {
      ...recursive.cases[3],
      code: "DEADLINE_LIMIT",
      elapsedMicros: 50_000,
    };
    expect(parseLuaProbeSuccess(recursive)).toEqual(recursive);

    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 5 ? { ...entry, code: "INSTRUCTION_LIMIT" } : entry,
      ),
    });
  });

  it("binds stop codes to their mandatory measurements", () => {
    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 1
          ? { ...entry, hookCount: 0, instructionEstimate: 0 }
          : entry,
      ),
    });

    const deadline = validSuccess();
    deadline.cases[1] = {
      ...deadline.cases[1],
      code: "DEADLINE_LIMIT",
      elapsedMicros: 49_999,
    };
    expectProtocolFailure(deadline);

    const deadlineWithoutHook = validSuccess();
    deadlineWithoutHook.cases[1] = {
      ...deadlineWithoutHook.cases[1],
      code: "DEADLINE_LIMIT",
      elapsedMicros: 50_000,
      hookCount: 0,
      instructionEstimate: 0,
    };
    expectProtocolFailure(deadlineWithoutHook);

    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 1 ? { ...entry, elapsedMicros: 50_000 } : entry,
      ),
    });

    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 0 ? { ...entry, elapsedMicros: 50_001 } : entry,
      ),
    });
    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 7 ? { ...entry, elapsedMicros: 50_001 } : entry,
      ),
    });
    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 0
          ? { ...entry, hookCount: 100, instructionEstimate: 100_000 }
          : entry,
      ),
    });
  });

  it.each([
    ["negative elapsed", { elapsedMicros: -1 }],
    ["fractional elapsed", { elapsedMicros: 1.5 }],
    ["too many hooks", { hookCount: 101, instructionEstimate: 101_000 }],
    ["wrong instruction estimate", { instructionEstimate: 999 }],
    ["used memory over ceiling", { usedMemoryBytes: LUA_LIMITS.memoryCeilingBytes + 1 }],
    ["peak memory over ceiling", { observedPeakMemoryBytes: LUA_LIMITS.memoryCeilingBytes + 1 }],
    ["peak below used", { usedMemoryBytes: 10, observedPeakMemoryBytes: 9 }],
    ["wrong case ceiling", { memoryCeilingBytes: LUA_LIMITS.memoryCeilingBytes - 1 }],
  ])("rejects invalid numeric evidence: %s", (_label, mutation) => {
    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 0 ? { ...entry, ...mutation } : entry,
      ),
    });
  });

  it("requires result 55 only for allowed cases", () => {
    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 0 ? { ...entry, result: null } : entry,
      ),
    });
    expectProtocolFailure({
      ...validSuccess(),
      cases: validSuccess().cases.map((entry, index) =>
        index === 1 ? { ...entry, result: 55 } : entry,
      ),
    });
  });

  it.each(Object.keys(validSuccess().defenses))(
    "requires defense %s to be literal true",
    (key) => {
      expectProtocolFailure({
        ...validSuccess(),
        defenses: { ...validSuccess().defenses, [key]: false },
      });
    },
  );

  it("rejects extra defense and limit fields", () => {
    expectProtocolFailure({
      ...validSuccess(),
      defenses: { ...validSuccess().defenses, nativeMessage: true },
    });
    expectProtocolFailure({
      ...validSuccess(),
      limits: { ...LUA_LIMITS, hiddenBudget: 1 },
    });
  });

  it("rejects oversized and non-JSON-serializable receipts", () => {
    expectProtocolFailure({ ...validSuccess(), padding: "한".repeat(1_500) });
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    expectProtocolFailure(cyclic);
    expectProtocolFailure({ value: 1n });
  });
});

describe("bounded Lua limits failures", () => {
  it.each(LUA_PROBE_FAILURE_CODES)("accepts exact failure code %s", (code) => {
    expect(parseLuaProbeFailure({ protocolVersion: 1, code })).toEqual({
      protocolVersion: 1,
      code,
    });
  });

  it("rejects unknown, missing, expanded, and oversized failures", () => {
    expect(() => parseLuaProbeFailure({ protocolVersion: 1, code: "RAW_ERROR" })).toThrow(
      LuaProbeProtocolError,
    );
    expect(() => parseLuaProbeFailure({ code: "INTERNAL_STATE" })).toThrow(
      LuaProbeProtocolError,
    );
    expect(() =>
      parseLuaProbeFailure({
        protocolVersion: 1,
        code: "INTERNAL_STATE",
        detail: "raw Lua error",
      }),
    ).toThrow(LuaProbeProtocolError);
    expect(() =>
      parseLuaProbeFailure({
        protocolVersion: 1,
        code: "INTERNAL_STATE",
        padding: "x".repeat(LUA_LIMITS.maxSerializedBytes),
      }),
    ).toThrow(LuaProbeProtocolError);
  });
});

describe("no-argument Lua limits invocation", () => {
  it("uses the exact command without arguments and parses success", async () => {
    const nativeInvoke = vi.fn(async () => validSuccess());
    await expect(runLuaLimitsM1Probe(nativeInvoke)).resolves.toEqual(validSuccess());
    expect(nativeInvoke).toHaveBeenCalledTimes(1);
    expect(nativeInvoke).toHaveBeenCalledWith(LUA_M1_COMMAND);
  });

  it("maps only bounded failures and hides unknown rejection details", async () => {
    const boundedInvoke = vi.fn(async () =>
      Promise.reject({ protocolVersion: 1, code: "PROBE_BUSY" }),
    );
    await expect(runLuaLimitsM1Probe(boundedInvoke)).rejects.toEqual(
      new LuaProbeCommandError({ protocolVersion: 1, code: "PROBE_BUSY" }),
    );

    const unknownInvoke = vi.fn(async () => Promise.reject(new Error("raw native detail")));
    await expect(runLuaLimitsM1Probe(unknownInvoke)).rejects.toBeInstanceOf(
      LuaProbeProtocolError,
    );
  });
});
