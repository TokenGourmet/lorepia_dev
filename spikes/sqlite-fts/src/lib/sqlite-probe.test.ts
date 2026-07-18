import { describe, expect, it, vi } from "vitest";

import {
  SQLITE_FIXTURE_SHA256,
  SQLITE_GOLDEN_RESULTS,
  SQLITE_M1_COMMAND,
  SQLITE_PROBE_ERROR_CODES,
  SQLiteProbeCommandError,
  SQLiteProbeProtocolError,
  parseSQLiteProbeFailure,
  parseSQLiteProbeSuccess,
  runSQLiteM1Probe,
  type NativeInvoke,
} from "./sqlite-probe";

function validSuccess(): Record<string, unknown> {
  return {
    protocolVersion: 1,
    schemaVersion: 2,
    appliedMigrations: [1, 2],
    migrationsIdempotent: true,
    futureSchemaRejected: true,
    persistence: {
      markerReopened: true,
      fixtureRowsReopened: true,
    },
    concurrency: {
      journalMode: "wal",
      busyTimeoutMs: 250,
      readerWriterConcurrent: true,
      snapshotIsolated: true,
      busyObserved: true,
      retrySucceeded: true,
    },
    search: {
      tokenizer: "trigram",
      shortQueryPolicy: "escaped-like-bounded",
      shortQueryLimit: 64,
      golden: SQLITE_GOLDEN_RESULTS.map((entry) => ({
        queryId: entry.queryId,
        resultIds: [...entry.resultIds],
      })),
      mutationSync: true,
      injectionSafe: true,
    },
    sqliteVersion: "3.50.2",
    compileOptions: ["ENABLE_FTS5", "MAX_VARIABLE_NUMBER=250000", "THREADSAFE=1"],
    fts5Enabled: true,
    fixtureSha256: SQLITE_FIXTURE_SHA256,
    cleanupPending: false,
  };
}

function withPersistence(
  change: Record<string, unknown>,
): Record<string, unknown> {
  const value = validSuccess();
  value.persistence = {
    ...(value.persistence as Record<string, unknown>),
    ...change,
  };
  return value;
}

function withConcurrency(
  change: Record<string, unknown>,
): Record<string, unknown> {
  const value = validSuccess();
  value.concurrency = {
    ...(value.concurrency as Record<string, unknown>),
    ...change,
  };
  return value;
}

function withSearch(change: Record<string, unknown>): Record<string, unknown> {
  const value = validSuccess();
  value.search = {
    ...(value.search as Record<string, unknown>),
    ...change,
  };
  return value;
}

describe("SQLite probe success contract", () => {
  it("accepts the exact bounded M-1 receipt", () => {
    const value = validSuccess();
    expect(parseSQLiteProbeSuccess(value)).toEqual(value);
  });

  it.each([
    ["protocol version", { protocolVersion: 2 }],
    ["schema version", { schemaVersion: 1 }],
    ["migration list", { appliedMigrations: [2, 1] }],
    ["missing migration", { appliedMigrations: [1] }],
    ["idempotence", { migrationsIdempotent: false }],
    ["future schema rejection", { futureSchemaRejected: false }],
    ["FTS5 flag", { fts5Enabled: false }],
    ["pending cleanup", { cleanupPending: true }],
  ])("rejects an invalid exact %s value", (_caseName, change) => {
    expect(() => parseSQLiteProbeSuccess({ ...validSuccess(), ...change })).toThrow(
      SQLiteProbeProtocolError,
    );
  });

  it.each([
    ["marker reopen", { markerReopened: false }],
    ["fixture reopen", { fixtureRowsReopened: false }],
    ["unknown persistence field", { copied: true }],
  ])("rejects invalid persistence evidence: %s", (_caseName, change) => {
    expect(() => parseSQLiteProbeSuccess(withPersistence(change))).toThrow(
      SQLiteProbeProtocolError,
    );
  });

  it.each([
    ["journal mode", { journalMode: "delete" }],
    ["busy timeout", { busyTimeoutMs: 251 }],
    ["reader/writer overlap", { readerWriterConcurrent: false }],
    ["snapshot isolation", { snapshotIsolated: false }],
    ["observed busy", { busyObserved: false }],
    ["retry", { retrySucceeded: false }],
    ["unknown concurrency field", { writerCount: 1 }],
  ])("rejects invalid concurrency evidence: %s", (_caseName, change) => {
    expect(() => parseSQLiteProbeSuccess(withConcurrency(change))).toThrow(
      SQLiteProbeProtocolError,
    );
  });

  it.each([
    ["tokenizer", { tokenizer: "unicode61" }],
    ["short-query policy", { shortQueryPolicy: "like" }],
    ["short-query limit", { shortQueryLimit: 65 }],
    ["mutation synchronization", { mutationSync: false }],
    ["injection safety", { injectionSafe: false }],
    ["empty golden set", { golden: [] }],
    [
      "partial golden set",
      {
        golden: SQLITE_GOLDEN_RESULTS.slice(0, -1).map((entry) => ({
          queryId: entry.queryId,
          resultIds: [...entry.resultIds],
        })),
      },
    ],
    [
      "wrong golden result",
      {
        golden: SQLITE_GOLDEN_RESULTS.map((entry, index) => ({
          queryId: entry.queryId,
          resultIds: index === 0 ? [2] : [...entry.resultIds],
        })),
      },
    ],
    ["unknown search field", { query: "달빛" }],
  ])("rejects invalid search evidence: %s", (_caseName, change) => {
    expect(() => parseSQLiteProbeSuccess(withSearch(change))).toThrow(
      SQLiteProbeProtocolError,
    );
  });

  it.each([
    ["unsorted result IDs", [{ queryId: "q-fts-euneunhan", resultIds: [4, 1] }]],
    ["duplicate result IDs", [{ queryId: "q-fts-euneunhan", resultIds: [1, 1] }]],
    ["negative result ID", [{ queryId: "q-fts-euneunhan", resultIds: [-1] }]],
    ["fractional result ID", [{ queryId: "q-fts-euneunhan", resultIds: [1.5] }]],
    ["oversized result ID", [{ queryId: "q-fts-euneunhan", resultIds: [2_147_483_648] }]],
    ["malformed query ID", [{ queryId: "Q one", resultIds: [1] }]],
    [
      "duplicate query ID",
      [
        { queryId: "q-one", resultIds: [1] },
        { queryId: "q-one", resultIds: [2] },
      ],
    ],
    ["unknown golden field", [{ queryId: "q-fts-euneunhan", resultIds: [1], term: "은은한" }]],
  ])("rejects malformed golden evidence: %s", (_caseName, golden) => {
    const padded = [
      ...golden,
      ...SQLITE_GOLDEN_RESULTS.slice(golden.length).map((entry) => ({
        queryId: entry.queryId,
        resultIds: [...entry.resultIds],
      })),
    ];
    expect(() => parseSQLiteProbeSuccess(withSearch({ golden: padded }))).toThrow(
      SQLiteProbeProtocolError,
    );
  });

  it("bounds each golden result list to 64 IDs", () => {
    const rejected = Array.from({ length: 65 }, (_, index) => index);
    const golden = SQLITE_GOLDEN_RESULTS.map((entry, index) => ({
      queryId: entry.queryId,
      resultIds: index === 0 ? rejected : [...entry.resultIds],
    }));
    expect(() =>
      parseSQLiteProbeSuccess(withSearch({ golden })),
    ).toThrow(SQLiteProbeProtocolError);
  });

  it.each([
    "3.50",
    "3.50.2.1",
    "v3.50.2",
    "3.050.2-beta",
    "1234.1.1",
    "3 50 2",
  ])("rejects malformed SQLite version %s", (sqliteVersion) => {
    expect(() =>
      parseSQLiteProbeSuccess({ ...validSuccess(), sqliteVersion }),
    ).toThrow(SQLiteProbeProtocolError);
  });

  it.each([
    ["empty list", []],
    ["unsorted list", ["THREADSAFE=1", "ENABLE_FTS5"]],
    ["duplicate option", ["ENABLE_FTS5", "ENABLE_FTS5"]],
    ["missing FTS5 option", ["THREADSAFE=1"]],
    ["lowercase name", ["enable_fts5"]],
    ["space", ["ENABLE FTS5"]],
    ["empty value", ["COMPILER="]],
  ])("rejects invalid compile options: %s", (_caseName, compileOptions) => {
    expect(() =>
      parseSQLiteProbeSuccess({ ...validSuccess(), compileOptions }),
    ).toThrow(SQLiteProbeProtocolError);
  });

  it("accepts a bounded negative numeric compile-option value", () => {
    const value = {
      ...validSuccess(),
      compileOptions: ["DEFAULT_CACHE_SIZE=-2000", "ENABLE_FTS5", "THREADSAFE=1"],
    };
    expect(parseSQLiteProbeSuccess(value)).toMatchObject({
      compileOptions: value.compileOptions,
    });
  });

  it("bounds compile-option count and length", () => {
    const tooMany = Array.from(
      { length: 129 },
      (_, index) => `OPTION_${index.toString().padStart(3, "0")}`,
    );
    const tooLong = `OPTION_${"A".repeat(122)}`;
    expect(tooLong).toHaveLength(129);
    expect(() =>
      parseSQLiteProbeSuccess({ ...validSuccess(), compileOptions: tooMany }),
    ).toThrow(SQLiteProbeProtocolError);
    expect(() =>
      parseSQLiteProbeSuccess({ ...validSuccess(), compileOptions: [tooLong] }),
    ).toThrow(SQLiteProbeProtocolError);
  });

  it.each([
    "A".repeat(64),
    "0".repeat(63),
    `${"0".repeat(63)}g`,
    `${"0".repeat(64)}00`,
  ])("rejects malformed fixture SHA-256 %s", (fixtureSha256) => {
    expect(() =>
      parseSQLiteProbeSuccess({ ...validSuccess(), fixtureSha256 }),
    ).toThrow(SQLiteProbeProtocolError);
  });

  it("rejects a different well-formed fixture SHA-256", () => {
    expect(() =>
      parseSQLiteProbeSuccess({ ...validSuccess(), fixtureSha256: "0".repeat(64) }),
    ).toThrow(SQLiteProbeProtocolError);
  });

  it.each([
    ["unknown top-level field", { ...validSuccess(), note: "extra" }],
    [
      "missing top-level field",
      (() => {
        const value = validSuccess();
        delete value.search;
        return value;
      })(),
    ],
    ["array response", []],
    ["null response", null],
  ])("rejects a non-exact response: %s", (_caseName, value) => {
    expect(() => parseSQLiteProbeSuccess(value)).toThrow(SQLiteProbeProtocolError);
  });
});

describe("SQLite probe failure contract", () => {
  it.each(SQLITE_PROBE_ERROR_CODES)("accepts bounded error %s", (code) => {
    expect(parseSQLiteProbeFailure({ code, cleanupPending: false })).toEqual({
      code,
      cleanupPending: false,
    });
    expect(parseSQLiteProbeFailure({ code, cleanupPending: true })).toEqual({
      code,
      cleanupPending: true,
    });
  });

  it.each([
    { code: "OTHER", cleanupPending: false },
    { code: "OPEN_FAILURE" },
    { code: "OPEN_FAILURE", cleanupPending: "false" },
    { code: "OPEN_FAILURE", cleanupPending: false, message: "raw detail" },
  ])("rejects malformed or expanded failure %#", (value) => {
    expect(() => parseSQLiteProbeFailure(value)).toThrow(SQLiteProbeProtocolError);
  });
});

describe("SQLite probe invocation", () => {
  it("invokes exactly the no-argument M-1 command and parses its result", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => validSuccess());

    await expect(runSQLiteM1Probe(invokeNative)).resolves.toEqual(validSuccess());
    expect(invokeNative).toHaveBeenCalledTimes(1);
    expect(invokeNative.mock.calls[0]).toEqual([SQLITE_M1_COMMAND]);
  });

  it("turns only a strict bounded native failure into a typed error", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => {
      throw { code: "CONCURRENCY_FAILURE", cleanupPending: false };
    });

    const failure = await runSQLiteM1Probe(invokeNative).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(SQLiteProbeCommandError);
    expect((failure as SQLiteProbeCommandError).failure).toEqual({
      code: "CONCURRENCY_FAILURE",
      cleanupPending: false,
    });
  });

  it("rejects an expanded thrown error without copying its fields", async () => {
    const invokeNative = vi.fn<NativeInvoke>(async () => {
      throw {
        code: "OPEN_FAILURE",
        cleanupPending: false,
        message: "native path must not cross",
      };
    });

    const failure = await runSQLiteM1Probe(invokeNative).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(SQLiteProbeProtocolError);
    expect(failure).not.toHaveProperty("message", "native path must not cross");
    expect(String(failure)).not.toContain("native path must not cross");
  });
});
