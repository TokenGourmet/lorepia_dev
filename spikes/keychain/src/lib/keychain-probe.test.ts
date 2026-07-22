import { describe, expect, it, vi } from "vitest";

import {
  KEYCHAIN_BACKENDS,
  KEYCHAIN_M1_COMMAND,
  KEYCHAIN_PROBE_ERROR_CODES,
  KeychainProbeCommandError,
  KeychainProbeProtocolError,
  parseKeychainProbeFailure,
  parseKeychainProbeSuccess,
  runKeychainM1Probe,
  type NativeInvoke,
} from "./keychain-probe";

function validSuccess(): Record<string, unknown> {
  return {
    runId: "0123456789abcdef0123456789abcdef",
    backend: "macos-keychain",
    referenceFingerprint: "0123456789abcdef",
    lifecycle: {
      absentBeforeCreate: true,
      created: true,
      initialReadMatched: true,
      updated: true,
      updatedReadMatched: true,
      deleted: true,
      absentAfterDelete: true,
    },
    staleCleanupRecovered: false,
    cleanupPending: false,
  };
}

describe("keychain probe success contract", () => {
  it.each(KEYCHAIN_BACKENDS)("accepts the bounded %s evidence", (backend) => {
    const value = { ...validSuccess(), backend };
    expect(parseKeychainProbeSuccess(value)).toEqual(value);
  });

  it("accepts stale-cleanup recovery only as a boolean", () => {
    expect(
      parseKeychainProbeSuccess({ ...validSuccess(), staleCleanupRecovered: true }),
    ).toMatchObject({ staleCleanupRecovered: true });
    expect(() =>
      parseKeychainProbeSuccess({ ...validSuccess(), staleCleanupRecovered: 1 }),
    ).toThrow(KeychainProbeProtocolError);
  });

  it.each([
    ["unknown top-level field", { ...validSuccess(), note: "extra" }],
    [
      "unknown lifecycle field",
      {
        ...validSuccess(),
        lifecycle: { ...(validSuccess().lifecycle as object), readAgain: true },
      },
    ],
    ["missing field", (() => { const value = validSuccess(); delete value.backend; return value; })()],
    ["uppercase run ID", { ...validSuccess(), runId: "0123456789ABCDEF0123456789ABCDEF" }],
    ["short run ID", { ...validSuccess(), runId: "0123456789abcdef" }],
    ["non-hex run ID", { ...validSuccess(), runId: "z123456789abcdef0123456789abcdef" }],
    ["unknown backend", { ...validSuccess(), backend: "memory" }],
    ["uppercase fingerprint", { ...validSuccess(), referenceFingerprint: "0123456789ABCDEF" }],
    ["long fingerprint", { ...validSuccess(), referenceFingerprint: "0123456789abcdef00" }],
    [
      "failed lifecycle stage",
      {
        ...validSuccess(),
        lifecycle: { ...(validSuccess().lifecycle as object), updatedReadMatched: false },
      },
    ],
    ["pending cleanup on success", { ...validSuccess(), cleanupPending: true }],
  ])("rejects %s", (_caseName, value) => {
    expect(() => parseKeychainProbeSuccess(value)).toThrow(KeychainProbeProtocolError);
  });

  it.each([
    ["secret", "value"],
    ["password", "value"],
    ["apiToken", "value"],
    ["rawAccount", "value"],
    ["account", "value"],
    ["reference", "value"],
    ["accountReference", "value"],
  ])("rejects secret-like field %s", (key, fieldValue) => {
    expect(() =>
      parseKeychainProbeSuccess({ ...validSuccess(), [key]: fieldValue }),
    ).toThrow(KeychainProbeProtocolError);
    expect(() =>
      parseKeychainProbeSuccess({
        ...validSuccess(),
        lifecycle: { ...(validSuccess().lifecycle as object), [key]: fieldValue },
      }),
    ).toThrow(KeychainProbeProtocolError);
  });
});

describe("keychain probe failure contract", () => {
  it.each(KEYCHAIN_PROBE_ERROR_CODES)("accepts bounded error %s", (code) => {
    expect(parseKeychainProbeFailure({ code, cleanupPending: false })).toEqual({
      code,
      cleanupPending: false,
    });
  });

  it.each([
    { code: "OTHER", cleanupPending: false },
    { code: "STORE_FAILURE" },
    { code: "STORE_FAILURE", cleanupPending: "false" },
    { code: "STORE_FAILURE", cleanupPending: false, message: "raw detail" },
    { code: "STORE_FAILURE", cleanupPending: false, password: "leak" },
  ])("rejects malformed or expanded failure %#", (value) => {
    expect(() => parseKeychainProbeFailure(value)).toThrow(
      KeychainProbeProtocolError,
    );
  });
});

describe("keychain probe invocation", () => {
  it("invokes exactly the no-argument M-1 command and parses its result", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => validSuccess());

    await expect(runKeychainM1Probe(invokeNative)).resolves.toEqual(validSuccess());
    expect(invokeNative).toHaveBeenCalledTimes(1);
    expect(invokeNative.mock.calls[0]).toEqual([KEYCHAIN_M1_COMMAND]);
  });

  it("turns only a strict bounded native failure into a typed error", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => {
      throw { code: "STORE_LOCKED", cleanupPending: false };
    });

    const failure = await runKeychainM1Probe(invokeNative).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(KeychainProbeCommandError);
    expect((failure as KeychainProbeCommandError).failure).toEqual({
      code: "STORE_LOCKED",
      cleanupPending: false,
    });
  });

  it("rejects an unbounded thrown error without copying its fields", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => {
      throw { code: "STORE_FAILURE", cleanupPending: false, secret: "must-not-cross" };
    });

    const failure = await runKeychainM1Probe(invokeNative).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(KeychainProbeProtocolError);
    expect(failure).not.toHaveProperty("secret");
    expect(String(failure)).not.toContain("must-not-cross");
  });
});
