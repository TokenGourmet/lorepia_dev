import { invoke } from "@tauri-apps/api/core";

export const SQLITE_M1_COMMAND = "run_sqlite_m1_probe" as const;

export const SQLITE_FIXTURE_SHA256 =
  "b5e8b2f2fdcf40d33dbb5eca555c982700e3cc1559dfe3adc878d85e2380b674" as const;

export const SQLITE_GOLDEN_RESULTS = [
  { queryId: "q-fts-euneunhan", resultIds: [1] },
  { queryId: "q-fts-doseogwan", resultIds: [1] },
  { queryId: "q-fts-jeonggeojang", resultIds: [2] },
  { queryId: "q-like-byeol", resultIds: [2] },
  { queryId: "q-like-bit", resultIds: [1, 2, 4] },
  { queryId: "q-like-escaped-wildcards", resultIds: [5] },
  { queryId: "q-fts-injection-literal", resultIds: [] },
] as const;

export const SQLITE_PROBE_ERROR_CODES = [
  "PATH_UNAVAILABLE",
  "OPEN_FAILURE",
  "MIGRATION_FAILURE",
  "PERSISTENCE_FAILURE",
  "CONCURRENCY_FAILURE",
  "FTS_UNAVAILABLE",
  "FTS_GOLDEN_MISMATCH",
  "CLEANUP_FAILURE",
  "PROBE_BUSY",
  "INTERNAL_STATE",
] as const;

export type SQLiteProbeErrorCode = (typeof SQLITE_PROBE_ERROR_CODES)[number];

export type SQLitePersistenceEvidence = {
  markerReopened: true;
  fixtureRowsReopened: true;
};

export type SQLiteConcurrencyEvidence = {
  journalMode: "wal";
  busyTimeoutMs: 250;
  readerWriterConcurrent: true;
  snapshotIsolated: true;
  busyObserved: true;
  retrySucceeded: true;
};

export type SQLiteGoldenResult = {
  queryId: string;
  resultIds: number[];
};

export type SQLiteSearchEvidence = {
  tokenizer: "trigram";
  shortQueryPolicy: "escaped-like-bounded";
  shortQueryLimit: 64;
  golden: SQLiteGoldenResult[];
  mutationSync: true;
  injectionSafe: true;
};

export type SQLiteProbeSuccess = {
  protocolVersion: 1;
  schemaVersion: 2;
  appliedMigrations: [1, 2];
  migrationsIdempotent: true;
  futureSchemaRejected: true;
  persistence: SQLitePersistenceEvidence;
  concurrency: SQLiteConcurrencyEvidence;
  search: SQLiteSearchEvidence;
  sqliteVersion: string;
  compileOptions: string[];
  fts5Enabled: true;
  fixtureSha256: string;
  cleanupPending: false;
};

export type SQLiteProbeFailure = {
  code: SQLiteProbeErrorCode;
  cleanupPending: boolean;
};

export type NativeInvoke = (command: typeof SQLITE_M1_COMMAND) => Promise<unknown>;

const SUCCESS_KEYS = [
  "appliedMigrations",
  "cleanupPending",
  "compileOptions",
  "concurrency",
  "fixtureSha256",
  "fts5Enabled",
  "futureSchemaRejected",
  "migrationsIdempotent",
  "persistence",
  "protocolVersion",
  "schemaVersion",
  "search",
  "sqliteVersion",
] as const;
const PERSISTENCE_KEYS = ["fixtureRowsReopened", "markerReopened"] as const;
const CONCURRENCY_KEYS = [
  "busyObserved",
  "busyTimeoutMs",
  "journalMode",
  "readerWriterConcurrent",
  "retrySucceeded",
  "snapshotIsolated",
] as const;
const SEARCH_KEYS = [
  "golden",
  "injectionSafe",
  "mutationSync",
  "shortQueryLimit",
  "shortQueryPolicy",
  "tokenizer",
] as const;
const GOLDEN_KEYS = ["queryId", "resultIds"] as const;
const FAILURE_KEYS = ["cleanupPending", "code"] as const;

const SQLITE_VERSION_PATTERN = /^[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}$/;
const COMPILE_OPTION_PATTERN = /^[A-Z][A-Z0-9_]*(?:=[A-Za-z0-9_.+:\/-]+)?$/;
const QUERY_ID_PATTERN = /^[a-z0-9][a-z0-9-]{0,63}$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const MAX_COMPILE_OPTIONS = 128;
const MAX_COMPILE_OPTION_LENGTH = 128;
const MAX_RESULT_IDS = 64;
const MAX_RESULT_ID = 2_147_483_647;

export class SQLiteProbeProtocolError extends Error {
  constructor() {
    super("SQLite probe returned an invalid bounded response");
    this.name = "SQLiteProbeProtocolError";
  }
}

export class SQLiteProbeCommandError extends Error {
  readonly failure: SQLiteProbeFailure;

  constructor(failure: SQLiteProbeFailure) {
    super("SQLite probe command failed");
    this.name = "SQLiteProbeCommandError";
    this.failure = failure;
  }
}

function failProtocol(): never {
  throw new SQLiteProbeProtocolError();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function requireExactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
): void {
  const actual = Object.keys(value).sort();
  if (actual.length !== expected.length) failProtocol();
  for (let index = 0; index < expected.length; index += 1) {
    if (actual[index] !== expected[index]) failProtocol();
  }
}

function parsePersistence(value: unknown): SQLitePersistenceEvidence {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, PERSISTENCE_KEYS);
  if (value.markerReopened !== true || value.fixtureRowsReopened !== true) {
    failProtocol();
  }
  return { markerReopened: true, fixtureRowsReopened: true };
}

function parseConcurrency(value: unknown): SQLiteConcurrencyEvidence {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, CONCURRENCY_KEYS);
  if (
    value.journalMode !== "wal" ||
    value.busyTimeoutMs !== 250 ||
    value.readerWriterConcurrent !== true ||
    value.snapshotIsolated !== true ||
    value.busyObserved !== true ||
    value.retrySucceeded !== true
  ) {
    failProtocol();
  }
  return {
    journalMode: "wal",
    busyTimeoutMs: 250,
    readerWriterConcurrent: true,
    snapshotIsolated: true,
    busyObserved: true,
    retrySucceeded: true,
  };
}

function parseResultIds(value: unknown): number[] {
  if (!Array.isArray(value) || value.length > MAX_RESULT_IDS) failProtocol();
  let previous = -1;
  return value.map((entry) => {
    if (
      !Number.isSafeInteger(entry) ||
      (entry as number) < 0 ||
      (entry as number) > MAX_RESULT_ID ||
      (entry as number) <= previous
    ) {
      failProtocol();
    }
    previous = entry as number;
    return entry as number;
  });
}

function parseGolden(value: unknown): SQLiteGoldenResult[] {
  if (!Array.isArray(value) || value.length !== SQLITE_GOLDEN_RESULTS.length) {
    failProtocol();
  }
  const queryIds = new Set<string>();
  return value.map((entry, index) => {
    if (!isRecord(entry)) failProtocol();
    requireExactKeys(entry, GOLDEN_KEYS);
    if (typeof entry.queryId !== "string" || !QUERY_ID_PATTERN.test(entry.queryId)) {
      failProtocol();
    }
    if (queryIds.has(entry.queryId)) failProtocol();
    queryIds.add(entry.queryId);
    const resultIds = parseResultIds(entry.resultIds);
    const expected = SQLITE_GOLDEN_RESULTS[index];
    if (
      expected === undefined ||
      entry.queryId !== expected.queryId ||
      resultIds.length !== expected.resultIds.length ||
      resultIds.some((resultId, resultIndex) => resultId !== expected.resultIds[resultIndex])
    ) {
      failProtocol();
    }
    return {
      queryId: entry.queryId,
      resultIds,
    };
  });
}

function parseSearch(value: unknown): SQLiteSearchEvidence {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, SEARCH_KEYS);
  if (
    value.tokenizer !== "trigram" ||
    value.shortQueryPolicy !== "escaped-like-bounded" ||
    value.shortQueryLimit !== 64 ||
    value.mutationSync !== true ||
    value.injectionSafe !== true
  ) {
    failProtocol();
  }
  return {
    tokenizer: "trigram",
    shortQueryPolicy: "escaped-like-bounded",
    shortQueryLimit: 64,
    golden: parseGolden(value.golden),
    mutationSync: true,
    injectionSafe: true,
  };
}

function parseCompileOptions(value: unknown): string[] {
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.length > MAX_COMPILE_OPTIONS
  ) {
    failProtocol();
  }
  let previous: string | null = null;
  const options = value.map((entry) => {
    if (
      typeof entry !== "string" ||
      entry.length > MAX_COMPILE_OPTION_LENGTH ||
      !COMPILE_OPTION_PATTERN.test(entry) ||
      (previous !== null && previous >= entry)
    ) {
      failProtocol();
    }
    previous = entry;
    return entry;
  });
  if (!options.includes("ENABLE_FTS5")) failProtocol();
  return options;
}

export function parseSQLiteProbeSuccess(value: unknown): SQLiteProbeSuccess {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, SUCCESS_KEYS);
  if (
    value.protocolVersion !== 1 ||
    value.schemaVersion !== 2 ||
    !Array.isArray(value.appliedMigrations) ||
    value.appliedMigrations.length !== 2 ||
    value.appliedMigrations[0] !== 1 ||
    value.appliedMigrations[1] !== 2 ||
    value.migrationsIdempotent !== true ||
    value.futureSchemaRejected !== true ||
    value.fts5Enabled !== true ||
    typeof value.sqliteVersion !== "string" ||
    !SQLITE_VERSION_PATTERN.test(value.sqliteVersion) ||
    typeof value.fixtureSha256 !== "string" ||
    !SHA256_PATTERN.test(value.fixtureSha256) ||
    value.fixtureSha256 !== SQLITE_FIXTURE_SHA256 ||
    value.cleanupPending !== false
  ) {
    failProtocol();
  }
  return {
    protocolVersion: 1,
    schemaVersion: 2,
    appliedMigrations: [1, 2],
    migrationsIdempotent: true,
    futureSchemaRejected: true,
    persistence: parsePersistence(value.persistence),
    concurrency: parseConcurrency(value.concurrency),
    search: parseSearch(value.search),
    sqliteVersion: value.sqliteVersion,
    compileOptions: parseCompileOptions(value.compileOptions),
    fts5Enabled: true,
    fixtureSha256: value.fixtureSha256,
    cleanupPending: false,
  };
}

export function parseSQLiteProbeFailure(value: unknown): SQLiteProbeFailure {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, FAILURE_KEYS);
  if (
    typeof value.code !== "string" ||
    !(SQLITE_PROBE_ERROR_CODES as readonly string[]).includes(value.code) ||
    typeof value.cleanupPending !== "boolean"
  ) {
    failProtocol();
  }
  return {
    code: value.code as SQLiteProbeErrorCode,
    cleanupPending: value.cleanupPending,
  };
}

function invokeSQLiteM1Command(): Promise<unknown> {
  return invoke<unknown>("run_sqlite_m1_probe");
}

export async function runSQLiteM1Probe(
  invokeNative: NativeInvoke = invokeSQLiteM1Command,
): Promise<SQLiteProbeSuccess> {
  let rawResult: unknown;
  try {
    rawResult = await invokeNative(SQLITE_M1_COMMAND);
  } catch (rawFailure) {
    throw new SQLiteProbeCommandError(parseSQLiteProbeFailure(rawFailure));
  }
  return parseSQLiteProbeSuccess(rawResult);
}
