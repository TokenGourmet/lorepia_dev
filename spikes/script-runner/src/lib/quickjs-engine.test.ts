import { createHash } from "node:crypto";
import { describe, expect, it, vi } from "vitest";

import {
  OUTPUT_MAX_BYTES,
  WASM_MAXIMUM_PAGES,
  WASM_PAGE_BYTES,
} from "./runner-contract";
import {
  EXPECTED_ALLOWED_OUTPUT,
  EXPECTED_FORBIDDEN_GLOBALS_OUTPUT,
} from "./fixtures";
import {
  executeBoundedSource,
  executeFixture,
  type PreparedEngine,
} from "./quickjs-engine";

const sha256 = (value: string): string =>
  createHash("sha256").update(value).digest("hex");

function throwingEngine(
  error: unknown,
  beforeThrow?: (options: { shouldInterrupt?: () => boolean }) => void,
): PreparedEngine {
  const wasmMemory = new WebAssembly.Memory({ initial: 1, maximum: 1 });
  return {
    wasmMemory,
    quickJs: {
      evalCode: (
        _source: string,
        options: { shouldInterrupt?: () => boolean },
      ) => {
        beforeThrow?.(options);
        throw error;
      },
    } as unknown as PreparedEngine["quickJs"],
  };
}

describe.sequential("QuickJS-WASM execution boundary", () => {
  it("preserves the legitimate bounded input-to-JSON behavior", async () => {
    const result = await executeFixture("allowed");

    expect(result.code).toBe("ALLOWED_RESULT");
    expect(result.outputBytes).toBe(
      new TextEncoder().encode(EXPECTED_ALLOWED_OUTPUT).byteLength,
    );
    expect(result.outputSha256).toBe(sha256(EXPECTED_ALLOWED_OUTPUT));
    expect(result.wasmMemoryBytes).toBeLessThanOrEqual(
      WASM_MAXIMUM_PAGES * WASM_PAGE_BYTES,
    );
  });

  it("interrupts a synchronous infinite loop inside the engine", async () => {
    const result = await executeFixture("infinite-loop");

    expect(result.code).toBe("ENGINE_INTERRUPTED");
    expect(result.outputBytes).toBe(0);
    expect(result.outputSha256).toBeNull();
  });

  it("contains recursive stack pressure", async () => {
    const result = await executeFixture("recursive-pressure");

    expect(result.code).toBe("STACK_LIMIT");
  });

  it("contains allocator pressure inside the QuickJS memory ceiling", async () => {
    const result = await executeFixture("allocator-pressure");

    expect(result.code).toBe("MEMORY_LIMIT");
    expect(result.wasmMemoryBytes).toBeLessThanOrEqual(
      WASM_MAXIMUM_PAGES * WASM_PAGE_BYTES,
    );
  });

  it("does not expose browser, Tauri, Node, or module-loader globals", async () => {
    const result = await executeFixture("forbidden-globals");

    expect(result.code).toBe("FORBIDDEN_GLOBALS_ABSENT");
    expect(result.outputSha256).toBe(
      sha256(EXPECTED_FORBIDDEN_GLOBALS_OUTPUT),
    );
  });

  it("rejects output before it can cross the Worker boundary", async () => {
    const result = await executeFixture("oversized-output");

    expect(result.code).toBe("OUTPUT_LIMIT");
    expect(result.outputBytes).toBeGreaterThan(OUTPUT_MAX_BYTES);
    expect(result.outputSha256).toBeNull();
  });

  it("redacts raw engine and script errors", async () => {
    const result = await executeFixture("script-error");

    expect(result.code).toBe("SCRIPT_ERROR");
    expect(JSON.stringify(result)).not.toContain(
      "LOREPIA_RAW_SECRET_MUST_NOT_CROSS_BOUNDARY",
    );
  });

  it("normalizes browser host stack and memory-limit error wording", async () => {
    const stack = await executeBoundedSource(
      "(_input) => null",
      "{}",
      "allowed",
      throwingEngine(
        new RangeError("Maximum call stack size exceeded"),
      ),
    );
    const memory = await executeBoundedSource(
      "(_input) => null",
      "{}",
      "allowed",
      throwingEngine(
        new WebAssembly.RuntimeError(
          "WebAssembly.Memory.grow(): Maximum memory size exceeded",
        ),
      ),
    );

    expect(stack.code).toBe("STACK_LIMIT");
    expect(memory.code).toBe("MEMORY_LIMIT");
  });

  it("trusts the host interrupt signal instead of the thrown value shape", async () => {
    const now = vi
      .spyOn(Date, "now")
      .mockReturnValueOnce(1_000)
      .mockReturnValue(1_100);
    try {
      const result = await executeBoundedSource(
        "(_input) => null",
        "{}",
        "allowed",
        throwingEngine(null, (options) => {
          expect(options.shouldInterrupt?.()).toBe(true);
        }),
      );

      expect(result.code).toBe("ENGINE_INTERRUPTED");
    } finally {
      now.mockRestore();
    }
  });

  it("recovers through a fresh engine after every hostile class", async () => {
    for (const hostile of [
      "infinite-loop",
      "recursive-pressure",
      "allocator-pressure",
      "oversized-output",
      "script-error",
    ] as const) {
      await executeFixture(hostile);
      await expect(executeFixture("allowed")).resolves.toMatchObject({
        code: "ALLOWED_RESULT",
      });
    }
  });
});
