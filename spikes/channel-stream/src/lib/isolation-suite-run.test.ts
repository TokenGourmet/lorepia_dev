import { describe, expect, it } from "vitest";

import {
  createIsolationSuiteRunGate,
  isValidSuiteRunId,
} from "./isolation-suite-run";

describe("isolation suite run gate", () => {
  it("starts the audit once after every unique result arrives", () => {
    const gate = createIsolationSuiteRunGate(["one", "two"]);
    gate.start("run-1");

    expect(gate.accept("run-1", "one")).toBe("accepted");
    expect(gate.accept("run-1", "one")).toBe("ignored");
    expect(gate.accept("run-1", "two")).toBe("complete");
    expect(gate.auditStarted).toBe(true);
    expect(gate.receivedCount).toBe(2);
    expect(gate.accept("run-1", "two")).toBe("ignored");
  });

  it("ignores missing, stale, and unknown run results", () => {
    const gate = createIsolationSuiteRunGate(["one"]);
    gate.start("run-current");

    expect(gate.accept("run-old", "one")).toBe("ignored");
    expect(gate.accept("run-current", "unknown")).toBe("ignored");
    expect(gate.receivedCount).toBe(0);
  });

  it("does not let a stale async completion finish a newer run", () => {
    const gate = createIsolationSuiteRunGate(["one"]);
    gate.start("run-old");
    expect(gate.accept("run-old", "one")).toBe("complete");
    expect(gate.finish("run-old")).toBe(true);

    gate.start("run-new");
    expect(gate.finish("run-old")).toBe(false);
    expect(gate.isActive("run-new")).toBe(true);
  });

  it("invalidates outstanding results on reload or unmount", () => {
    const gate = createIsolationSuiteRunGate(["one", "two"]);
    gate.start("run-1");
    expect(gate.accept("run-1", "one")).toBe("accepted");

    gate.invalidate();
    expect(gate.activeRunId).toBeNull();
    expect(gate.receivedCount).toBe(0);
    expect(gate.accept("run-1", "two")).toBe("ignored");
  });

  it("requires bounded host run IDs and unique expected IDs", () => {
    expect(isValidSuiteRunId("suite-0123456789abcdef")).toBe(true);
    expect(isValidSuiteRunId("contains space")).toBe(false);
    expect(() => createIsolationSuiteRunGate(["same", "same"])).toThrow(
      "non-empty unique list",
    );
  });
});
