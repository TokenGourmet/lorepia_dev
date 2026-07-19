import RELEASE_SYNC from "@jitl/quickjs-wasmfile-release-sync";
import {
  newQuickJSWASMModuleFromVariant,
  newVariant,
} from "quickjs-emscripten-core";

import {
  ENGINE_DEADLINE_MS,
  ENGINE_WARMUP_DEADLINE_MS,
  INPUT_MAX_BYTES,
  OUTPUT_MAX_BYTES,
  QUICKJS_MEMORY_LIMIT_BYTES,
  QUICKJS_STACK_LIMIT_BYTES,
  SOURCE_MAX_BYTES,
  WASM_INITIAL_PAGES,
  WASM_MAXIMUM_PAGES,
  assertBoundedCandidate,
  utf8ByteLength,
  type CaseCode,
  type FixtureId,
} from "./runner-contract";
import {
  EXPECTED_ALLOWED_OUTPUT,
  EXPECTED_FORBIDDEN_GLOBALS_OUTPUT,
  FIXTURE_INPUT_JSON,
  fixtureSource,
} from "./fixtures";

export interface EngineExecution {
  code: CaseCode;
  elapsedMs: number;
  outputBytes: number;
  outputSha256: string | null;
  wasmMemoryBytes: number;
}

type QuickJsModule = Awaited<
  ReturnType<typeof newQuickJSWASMModuleFromVariant>
>;

export interface PreparedEngine {
  quickJs: QuickJsModule;
  wasmMemory: WebAssembly.Memory;
}

type WrappedResult =
  | { kind: "OK"; outputJson: string; outputBytes: number }
  | { kind: "OUTPUT_LIMIT"; outputBytes: number }
  | { kind: "NON_JSON_OUTPUT" };

function monotonicNow(): number {
  return globalThis.performance?.now() ?? Date.now();
}

function buildWrappedSource(source: string, inputJson: string): string {
  return `(() => {
    "use strict";
    const safeEval = globalThis.eval;
    const safeParse = globalThis.JSON.parse;
    const safeStringify = globalThis.JSON.stringify;
    const safeReflectApply = globalThis.Reflect.apply;
    const safeCharCodeAt = globalThis.String.prototype.charCodeAt;
    const source = ${JSON.stringify(source)};
    const inputJson = ${JSON.stringify(inputJson)};
    const outputLimit = ${OUTPUT_MAX_BYTES};
    const utf8Bytes = (value) => {
      let bytes = 0;
      for (let index = 0; index < value.length; index += 1) {
        const code = safeReflectApply(safeCharCodeAt, value, [index]);
        if (code <= 0x7f) bytes += 1;
        else if (code <= 0x7ff) bytes += 2;
        else if (code >= 0xd800 && code <= 0xdbff) {
          const next = index + 1 < value.length
            ? safeReflectApply(safeCharCodeAt, value, [index + 1])
            : 0;
          if (next >= 0xdc00 && next <= 0xdfff) {
            bytes += 4;
            index += 1;
          } else bytes += 3;
        } else bytes += 3;
        if (bytes > outputLimit) return bytes;
      }
      return bytes;
    };
    const hook = (0, safeEval)("(" + source + "\\n)");
    if (typeof hook !== "function") {
      return safeStringify({ kind: "NON_JSON_OUTPUT" });
    }
    const output = hook(safeParse(inputJson));
    const outputJson = safeStringify(output);
    if (typeof outputJson !== "string") {
      return safeStringify({ kind: "NON_JSON_OUTPUT" });
    }
    const outputBytes = utf8Bytes(outputJson);
    if (outputBytes > outputLimit) {
      return safeStringify({ kind: "OUTPUT_LIMIT", outputBytes });
    }
    return safeStringify({ kind: "OK", outputJson, outputBytes });
  })()`;
}

async function sha256Hex(value: string): Promise<string> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) {
    throw new Error("SHA256_UNAVAILABLE");
  }
  const digest = await subtle.digest("SHA-256", new TextEncoder().encode(value));
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

function classifyThrown(error: unknown): CaseCode {
  const labels: string[] = [];
  const collect = (value: unknown): void => {
    if (typeof value === "string") {
      labels.push(value);
      return;
    }
    if (typeof value !== "object" || value === null) return;
    const record = value as Record<string, unknown>;
    if (typeof record.name === "string") labels.push(record.name);
    if (typeof record.message === "string") labels.push(record.message);
  };
  collect(error);
  if (typeof error === "object" && error !== null && "cause" in error) {
    collect((error as { cause?: unknown }).cause);
  }
  const message = labels.join(" ").toLowerCase();
  if (message.includes("interrupted")) {
    return "ENGINE_INTERRUPTED";
  }
  if (
    message.includes("out of memory") ||
    message.includes("allocation failed") ||
    message.includes("memory.grow") ||
    message.includes("maximum memory")
  ) {
    return "MEMORY_LIMIT";
  }
  if (
    message.includes("stack overflow") ||
    message.includes("maximum call stack")
  ) {
    return "STACK_LIMIT";
  }
  return "SCRIPT_ERROR";
}

function expectedSuccessCode(
  fixtureId: Exclude<FixtureId, "host-watchdog">,
  outputJson: string,
): CaseCode {
  if (fixtureId === "forbidden-globals") {
    return outputJson === EXPECTED_FORBIDDEN_GLOBALS_OUTPUT
      ? "FORBIDDEN_GLOBALS_ABSENT"
      : "CONTRACT_FAILURE";
  }
  if (fixtureId === "allowed") {
    return outputJson === EXPECTED_ALLOWED_OUTPUT
      ? "ALLOWED_RESULT"
      : "CONTRACT_FAILURE";
  }
  return "CONTRACT_FAILURE";
}

export async function executeBoundedSource(
  source: string,
  inputJson: string,
  fixtureId: Exclude<FixtureId, "host-watchdog">,
  preparedEngine?: PreparedEngine,
): Promise<EngineExecution> {
  assertBoundedCandidate(source, inputJson);
  const started = monotonicNow();
  let wasmMemory: WebAssembly.Memory | null = null;
  let interruptTriggered = false;

  try {
    const engine = preparedEngine ?? (await prepareEngine());
    wasmMemory = engine.wasmMemory;

    const deadline = Date.now() + ENGINE_DEADLINE_MS;
    const rawResult = engine.quickJs.evalCode(buildWrappedSource(source, inputJson), {
      memoryLimitBytes: QUICKJS_MEMORY_LIMIT_BYTES,
      maxStackSizeBytes: QUICKJS_STACK_LIMIT_BYTES,
      shouldInterrupt: () => {
        if (Date.now() < deadline) return false;
        interruptTriggered = true;
        return true;
      },
    });

    if (typeof rawResult !== "string") {
      throw new Error("INVALID_ENGINE_RESULT");
    }
    const wrapped = JSON.parse(rawResult) as WrappedResult;
    if (wrapped.kind === "OUTPUT_LIMIT") {
      return {
        code: "OUTPUT_LIMIT",
        elapsedMs: monotonicNow() - started,
        outputBytes: wrapped.outputBytes,
        outputSha256: null,
        wasmMemoryBytes: wasmMemory.buffer.byteLength,
      };
    }
    if (wrapped.kind !== "OK") {
      return {
        code: "SCRIPT_ERROR",
        elapsedMs: monotonicNow() - started,
        outputBytes: 0,
        outputSha256: null,
        wasmMemoryBytes: wasmMemory.buffer.byteLength,
      };
    }
    if (
      wrapped.outputBytes !== utf8ByteLength(wrapped.outputJson) ||
      wrapped.outputBytes > OUTPUT_MAX_BYTES
    ) {
      throw new Error("OUTPUT_CONTRACT_MISMATCH");
    }
    return {
      code: expectedSuccessCode(fixtureId, wrapped.outputJson),
      elapsedMs: monotonicNow() - started,
      outputBytes: wrapped.outputBytes,
      outputSha256: await sha256Hex(wrapped.outputJson),
      wasmMemoryBytes: wasmMemory.buffer.byteLength,
    };
  } catch (error) {
    return {
      code: interruptTriggered ? "ENGINE_INTERRUPTED" : classifyThrown(error),
      elapsedMs: monotonicNow() - started,
      outputBytes: 0,
      outputSha256: null,
      wasmMemoryBytes: wasmMemory?.buffer.byteLength ?? 0,
    };
  }
}

export async function prepareEngine(): Promise<PreparedEngine> {
  const wasmMemory = new WebAssembly.Memory({
    initial: WASM_INITIAL_PAGES,
    maximum: WASM_MAXIMUM_PAGES,
  });
  const variant = newVariant(RELEASE_SYNC, { wasmMemory });
  const quickJs = await newQuickJSWASMModuleFromVariant(variant);
  if (quickJs.getWasmMemory() !== wasmMemory) {
    throw new Error("WASM_MEMORY_NOT_OWNED");
  }
  const warmupDeadline = Date.now() + ENGINE_WARMUP_DEADLINE_MS;
  const warmupResult = quickJs.evalCode('"LOREPIA_ENGINE_READY"', {
    memoryLimitBytes: QUICKJS_MEMORY_LIMIT_BYTES,
    maxStackSizeBytes: QUICKJS_STACK_LIMIT_BYTES,
    shouldInterrupt: () => Date.now() >= warmupDeadline,
  });
  if (warmupResult !== "LOREPIA_ENGINE_READY") {
    throw new Error("ENGINE_WARMUP_FAILED");
  }
  return { quickJs, wasmMemory };
}

export async function executeFixture(
  fixtureId: Exclude<FixtureId, "host-watchdog">,
  preparedEngine?: PreparedEngine,
): Promise<EngineExecution> {
  const source = fixtureSource(fixtureId);
  if (
    utf8ByteLength(source) > SOURCE_MAX_BYTES ||
    utf8ByteLength(FIXTURE_INPUT_JSON) > INPUT_MAX_BYTES
  ) {
    throw new Error("FIXTURE_CONTRACT_FAILED");
  }
  return executeBoundedSource(
    source,
    FIXTURE_INPUT_JSON,
    fixtureId,
    preparedEngine,
  );
}
