import { describe, expect, it } from "vitest";

import { allFixtureSources, FIXTURE_INPUT_JSON } from "./fixtures";
import {
  ENGINE_DEADLINE_MS,
  ENGINE_WARMUP_DEADLINE_MS,
  INPUT_MAX_BYTES,
  MAX_WORKER_MESSAGE_BYTES,
  OUTPUT_MAX_BYTES,
  PROBE_CASE_IDS,
  SOURCE_MAX_BYTES,
  WORKER_BOOT_WATCHDOG_MS,
  assertBoundedCandidate,
  expectedCode,
  isProbeCaseId,
  serializedMessageIsBounded,
  utf8ByteLength,
} from "./runner-contract";

describe("script runner boundary contract", () => {
  it("pins the complete hostile-and-recovery suite order", () => {
    expect(PROBE_CASE_IDS).toEqual([
      "allowed-baseline",
      "infinite-loop",
      "recovery-after-infinite-loop",
      "recursive-pressure",
      "recovery-after-recursive-pressure",
      "allocator-pressure",
      "recovery-after-allocator-pressure",
      "forbidden-globals-absent",
      "recovery-after-forbidden-globals",
      "oversized-output",
      "recovery-after-oversized-output",
      "raw-error-redacted",
      "recovery-after-raw-error",
      "host-watchdog-termination",
      "recovery-after-host-watchdog",
    ]);
  });

  it("accepts only own fixed case identifiers", () => {
    expect(isProbeCaseId("allowed-baseline")).toBe(true);
    expect(isProbeCaseId("toString")).toBe(false);
    expect(isProbeCaseId("constructor")).toBe(false);
  });

  it("rejects source and input above the byte caps before engine creation", () => {
    expect(() =>
      assertBoundedCandidate("a".repeat(SOURCE_MAX_BYTES), "{}"),
    ).not.toThrow();
    expect(() =>
      assertBoundedCandidate("a".repeat(SOURCE_MAX_BYTES + 1), "{}"),
    ).toThrow("SOURCE_LIMIT");
    expect(() =>
      assertBoundedCandidate("", "a".repeat(INPUT_MAX_BYTES)),
    ).not.toThrow();
    expect(() =>
      assertBoundedCandidate("", "한".repeat(INPUT_MAX_BYTES / 3 + 1)),
    ).toThrow("INPUT_LIMIT");
  });

  it("keeps every embedded fixture and its fixed input below preflight caps", () => {
    for (const [, source] of allFixtureSources()) {
      expect(utf8ByteLength(source)).toBeLessThanOrEqual(SOURCE_MAX_BYTES);
    }
    expect(utf8ByteLength(FIXTURE_INPUT_JSON)).toBeLessThanOrEqual(
      INPUT_MAX_BYTES,
    );
  });

  it("keeps Worker messages below the direct boundary budget", () => {
    const bounded = { value: "x".repeat(MAX_WORKER_MESSAGE_BYTES - 20) };
    const oversized = { value: "x".repeat(MAX_WORKER_MESSAGE_BYTES) };

    expect(serializedMessageIsBounded(bounded)).toBe(true);
    expect(serializedMessageIsBounded(oversized)).toBe(false);
  });

  it("warms trusted engine code before starting the guest deadline", () => {
    expect(ENGINE_WARMUP_DEADLINE_MS).toBeGreaterThan(ENGINE_DEADLINE_MS);
    expect(ENGINE_WARMUP_DEADLINE_MS).toBeLessThan(
      WORKER_BOOT_WATCHDOG_MS,
    );
  });

  it("pins the output limit below the Worker message limit only after hashing", () => {
    expect(OUTPUT_MAX_BYTES).toBeGreaterThan(MAX_WORKER_MESSAGE_BYTES);
  });

  it("has an expected stable code for every case", () => {
    expect(PROBE_CASE_IDS.map(expectedCode)).toEqual([
      "ALLOWED_RESULT",
      "ENGINE_INTERRUPTED",
      "ALLOWED_RESULT",
      "STACK_LIMIT",
      "ALLOWED_RESULT",
      "MEMORY_LIMIT",
      "ALLOWED_RESULT",
      "FORBIDDEN_GLOBALS_ABSENT",
      "ALLOWED_RESULT",
      "OUTPUT_LIMIT",
      "ALLOWED_RESULT",
      "SCRIPT_ERROR",
      "ALLOWED_RESULT",
      "HOST_TERMINATED",
      "ALLOWED_RESULT",
    ]);
  });
});
