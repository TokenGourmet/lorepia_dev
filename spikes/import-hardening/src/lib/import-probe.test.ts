import { describe, expect, it, vi } from "vitest";

import {
  IMPORT_FIXTURE_CATALOG_SHA256,
  IMPORT_GOLDEN_CASES,
  IMPORT_LIMITS,
  IMPORT_M1_COMMAND,
  IMPORT_PNG_VERSION,
  IMPORT_POLICY_VERSION,
  IMPORT_PROBE_ERROR_CODES,
  IMPORT_VALID_ARCHIVE_GOLDEN,
  IMPORT_VALID_DIRECT_PNG_GOLDEN,
  IMPORT_ZIP_VERSION,
  ImportProbeCommandError,
  ImportProbeProtocolError,
  parseImportProbeFailure,
  parseImportProbeSuccess,
  runImportHardeningM1Probe,
  type ImportProbeSuccess,
} from "./import-probe";

function validSuccess(): ImportProbeSuccess {
  return {
    protocolVersion: 1,
    policyVersion: IMPORT_POLICY_VERSION,
    fixtureCatalogSha256: IMPORT_FIXTURE_CATALOG_SHA256,
    zipVersion: IMPORT_ZIP_VERSION,
    pngVersion: IMPORT_PNG_VERSION,
    limits: { ...IMPORT_LIMITS },
    cases: IMPORT_GOLDEN_CASES.map((entry) => ({ ...entry })),
    validArchive: {
      ...IMPORT_VALID_ARCHIVE_GOLDEN,
      executedEntries: 0,
      quarantine: "inert",
      atomicPublish: true,
      reopenedHashVerified: true,
    },
    validDirectPng: {
      ...IMPORT_VALID_DIRECT_PNG_GOLDEN,
      atomicPublish: true,
      reopenedHashVerified: true,
    },
    defenses: {
      traversalRejected: true,
      collisionRejected: true,
      unsafeEntryTypesRejected: true,
      sizeLimitsEnforced: true,
      compressionRatioEnforced: true,
      malformedArchiveRejected: true,
      strictPngValidated: true,
      unsupportedFilesRejected: true,
      outsideSentinelPreserved: true,
      stagingCleaned: true,
      scriptExecutionDisabled: true,
    },
    cleanupPending: false,
  };
}

function expectProtocolFailure(value: unknown): void {
  expect(() => parseImportProbeSuccess(value)).toThrow(ImportProbeProtocolError);
}

describe("strict import-hardening success receipt", () => {
  it("accepts the exact bounded receipt", () => {
    const value = validSuccess();
    expect(new TextEncoder().encode(JSON.stringify(value)).byteLength).toBeLessThanOrEqual(
      IMPORT_LIMITS.ipcResponseBytes,
    );
    expect(parseImportProbeSuccess(value)).toEqual(value);
  });

  it.each([
    ["protocol", { protocolVersion: 2 }],
    ["policy", { policyVersion: "m1-import-hardening-v2" }],
    ["catalog hash", { fixtureCatalogSha256: "0".repeat(64) }],
    ["zip version", { zipVersion: "8.6.1" }],
    ["png version", { pngVersion: "0.18.2" }],
    ["cleanup", { cleanupPending: true }],
  ])("rejects a mismatched %s", (_label, mutation) => {
    expectProtocolFailure({ ...validSuccess(), ...mutation });
  });

  it("rejects missing and unknown top-level fields", () => {
    const missing = { ...validSuccess() } as Record<string, unknown>;
    delete missing.defenses;
    expectProtocolFailure(missing);
    expectProtocolFailure({ ...validSuccess(), nativePath: "/private/raw" });
  });

  it.each(Object.entries(IMPORT_LIMITS))(
    "pins the exact %s limit",
    (key, value) => {
      expectProtocolFailure({
        ...validSuccess(),
        limits: { ...IMPORT_LIMITS, [key]: value + 1 },
      });
    },
  );

  it("rejects extra limit fields", () => {
    expectProtocolFailure({
      ...validSuccess(),
      limits: { ...IMPORT_LIMITS, hiddenBudget: 1 },
    });
  });

  it("rejects missing, duplicate, reordered, or expanded cases", () => {
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
        index === 0 ? { ...entry, nativeMessage: "raw" } : entry,
      ),
    });
  });

  it("rejects catalog expectation and error-code mutations", () => {
    const cases = validSuccess().cases;
    expectProtocolFailure({
      ...validSuccess(),
      cases: cases.map((entry, index) =>
        index === 0 ? { ...entry, outcome: "REJECTED", code: "UNSAFE_PATH" } : entry,
      ),
    });
    expectProtocolFailure({
      ...validSuccess(),
      cases: cases.map((entry, index) =>
        index === 2 ? { ...entry, code: "UNKNOWN_CODE" } : entry,
      ),
    });
  });

  it.each([
    ["archive SHA length", { sourceSha256: "1".repeat(63) }],
    ["archive SHA alphabet", { sourceSha256: "G".repeat(64) }],
    ["archive alternate SHA", { sourceSha256: "0".repeat(64) }],
    ["archive source zero", { sourceBytes: 0 }],
    ["archive source fraction", { sourceBytes: 1.5 }],
    ["archive source golden mismatch", { sourceBytes: IMPORT_VALID_ARCHIVE_GOLDEN.sourceBytes + 1 }],
    ["archive source limit", { sourceBytes: IMPORT_LIMITS.sourceBytes + 1 }],
    ["archive entry count", { entryCount: 5 }],
    ["archive total zero", { totalUncompressedBytes: 0 }],
    [
      "archive total golden mismatch",
      { totalUncompressedBytes: IMPORT_VALID_ARCHIVE_GOLDEN.totalUncompressedBytes + 1 },
    ],
    ["archive total limit", { totalUncompressedBytes: IMPORT_LIMITS.totalUncompressedBytes + 1 }],
    ["archive scripts", { scriptEntries: 1 }],
    ["executed script", { executedEntries: 1 }],
    ["quarantine", { quarantine: "active" }],
    ["atomic publish", { atomicPublish: false }],
    ["reopen hash", { reopenedHashVerified: false }],
  ])("rejects invalid validArchive proof: %s", (_label, mutation) => {
    expectProtocolFailure({
      ...validSuccess(),
      validArchive: { ...validSuccess().validArchive, ...mutation },
    });
  });

  it("rejects extra archive proof fields", () => {
    expectProtocolFailure({
      ...validSuccess(),
      validArchive: { ...validSuccess().validArchive, path: "/raw" },
    });
  });

  it.each([
    ["PNG SHA", { sourceSha256: "z".repeat(64) }],
    ["PNG alternate SHA", { sourceSha256: "0".repeat(64) }],
    ["PNG source zero", { sourceBytes: 0 }],
    ["PNG source golden mismatch", { sourceBytes: IMPORT_VALID_DIRECT_PNG_GOLDEN.sourceBytes + 1 }],
    ["PNG source limit", { sourceBytes: IMPORT_LIMITS.pngBytes + 1 }],
    ["PNG width", { width: 2 }],
    ["PNG height", { height: 2 }],
    ["PNG atomic publish", { atomicPublish: false }],
    ["PNG reopen hash", { reopenedHashVerified: false }],
  ])("rejects invalid validDirectPng proof: %s", (_label, mutation) => {
    expectProtocolFailure({
      ...validSuccess(),
      validDirectPng: { ...validSuccess().validDirectPng, ...mutation },
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

  it("rejects extra defense fields", () => {
    expectProtocolFailure({
      ...validSuccess(),
      defenses: { ...validSuccess().defenses, nativeDetail: true },
    });
  });

  it("rejects a serialized response larger than 4096 UTF-8 bytes", () => {
    const oversized = {
      ...validSuccess(),
      validArchive: {
        ...validSuccess().validArchive,
        sourceSha256: "한".repeat(1_500),
      },
    };
    expect(new TextEncoder().encode(JSON.stringify(oversized)).byteLength).toBeGreaterThan(
      IMPORT_LIMITS.ipcResponseBytes,
    );
    expectProtocolFailure(oversized);
  });

  it("rejects non-JSON-serializable input", () => {
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    expectProtocolFailure(cyclic);
    expectProtocolFailure({ value: 1n });
  });
});

describe("bounded import-hardening failures", () => {
  it.each(IMPORT_PROBE_ERROR_CODES)("accepts exact failure code %s", (code) => {
    expect(parseImportProbeFailure({ protocolVersion: 1, code, cleanupPending: false })).toEqual({
      protocolVersion: 1,
      code,
      cleanupPending: false,
    });
  });

  it("accepts cleanupPending only as a boolean", () => {
    expect(parseImportProbeFailure({
      protocolVersion: 1,
      code: "CLEANUP_FAILURE",
      cleanupPending: true,
    })).toEqual({ protocolVersion: 1, code: "CLEANUP_FAILURE", cleanupPending: true });
    expect(() =>
      parseImportProbeFailure({
        protocolVersion: 1,
        code: "CLEANUP_FAILURE",
        cleanupPending: "true",
      }),
    ).toThrow(ImportProbeProtocolError);
  });

  it("rejects unknown, missing, expanded, and oversized failures", () => {
    expect(() =>
      parseImportProbeFailure({ protocolVersion: 1, code: "RAW_ERROR", cleanupPending: false }),
    ).toThrow(ImportProbeProtocolError);
    expect(() =>
      parseImportProbeFailure({ code: "INTERNAL_STATE", cleanupPending: false }),
    ).toThrow(ImportProbeProtocolError);
    expect(() =>
      parseImportProbeFailure({
        protocolVersion: 1,
        code: "INTERNAL_STATE",
        cleanupPending: false,
        message: "x".repeat(4_096),
      }),
    ).toThrow(ImportProbeProtocolError);
  });
});

describe("native invocation boundary", () => {
  it("invokes exactly the no-argument M-1 command and parses its result", async () => {
    const invokeNative = vi.fn(async () => validSuccess());
    await expect(runImportHardeningM1Probe(invokeNative)).resolves.toEqual(validSuccess());
    expect(invokeNative).toHaveBeenCalledTimes(1);
    expect(invokeNative).toHaveBeenCalledWith(IMPORT_M1_COMMAND);
  });

  it("turns only an exact bounded rejection into a command error", async () => {
    const failure = { protocolVersion: 1, code: "UNSAFE_PATH", cleanupPending: false } as const;
    const invokeNative = vi.fn(async () => Promise.reject(failure));
    const rejection = runImportHardeningM1Probe(invokeNative);
    await expect(rejection).rejects.toBeInstanceOf(ImportProbeCommandError);
    await rejection.catch((error: unknown) => {
      expect((error as ImportProbeCommandError).failure).toEqual(failure);
    });
  });

  it("rejects an unbounded native error without copying its fields", async () => {
    const invokeNative = vi.fn(async () => Promise.reject(new Error("native path and details")));
    const rejection = runImportHardeningM1Probe(invokeNative);
    await expect(rejection).rejects.toBeInstanceOf(ImportProbeProtocolError);
    await rejection.catch((error: unknown) => {
      expect((error as Error).message).not.toContain("native path and details");
    });
  });
});
