import { invoke } from "@tauri-apps/api/core";

export const IMPORT_M1_COMMAND = "run_import_hardening_m1_probe" as const;
export const IMPORT_POLICY_VERSION = "m1-import-hardening-v1" as const;
export const IMPORT_FIXTURE_CATALOG_SHA256 =
  "484a313423d4e91c792818fb64097d96f8efb7c4a31befe96a1d3f739bfe5eb2" as const;
export const IMPORT_ZIP_VERSION = "8.6.0" as const;
export const IMPORT_PNG_VERSION = "0.18.1" as const;

export const IMPORT_VALID_ARCHIVE_GOLDEN = {
  sourceSha256: "485733d6f60763ef1e2e63b4595debc63500d2b67321ea0e4ffa3084b611dc0f",
  sourceBytes: 665,
  entryCount: 4,
  totalUncompressedBytes: 217,
  scriptEntries: 2,
} as const;

export const IMPORT_VALID_DIRECT_PNG_GOLDEN = {
  sourceSha256: "ff36b8831e688e8fb5a511d916e82621821f67ce1c1c8ee204c395702c5a1a04",
  sourceBytes: 70,
  width: 1,
  height: 1,
} as const;

export const IMPORT_LIMITS = {
  sourceBytes: 2_097_152,
  entryCount: 32,
  entryBytes: 524_288,
  totalUncompressedBytes: 1_048_576,
  compressionRatio: 100,
  pathBytes: 240,
  pathComponentBytes: 64,
  pathDepth: 8,
  streamBufferBytes: 16_384,
  indexBytes: 16_384,
  pngBytes: 524_288,
  pngDecodedBytes: 16_777_216,
  pngChunks: 64,
  pngChunkBytes: 262_144,
  pngWidth: 2_048,
  pngHeight: 2_048,
  pngPixels: 4_194_304,
  ipcResponseBytes: 4_096,
} as const;

export const IMPORT_PROBE_ERROR_CODES = [
  "PATH_UNAVAILABLE",
  "SOURCE_TOO_LARGE",
  "UNSUPPORTED_FORMAT",
  "ARCHIVE_MALFORMED",
  "UNSUPPORTED_COMPRESSION",
  "ENTRY_COUNT_LIMIT",
  "ENTRY_SIZE_LIMIT",
  "TOTAL_SIZE_LIMIT",
  "COMPRESSION_RATIO_LIMIT",
  "UNSAFE_PATH",
  "DUPLICATE_PATH",
  "UNSAFE_ENTRY_TYPE",
  "PNG_MALFORMED",
  "UNSUPPORTED_FILE_TYPE",
  "STAGING_FAILURE",
  "PUBLISH_CONFLICT",
  "PUBLISH_FAILURE",
  "CLEANUP_FAILURE",
  "PROBE_BUSY",
  "INTERNAL_STATE",
] as const;

export type ImportProbeErrorCode = (typeof IMPORT_PROBE_ERROR_CODES)[number];
export type ImportCaseOutcome = "ACCEPTED" | "REJECTED";

export type ImportCaseEvidence = {
  caseId: string;
  outcome: ImportCaseOutcome;
  code: ImportProbeErrorCode | null;
};

export const IMPORT_GOLDEN_CASES = [
  { caseId: "valid-archive", outcome: "ACCEPTED", code: null },
  { caseId: "valid-direct-png", outcome: "ACCEPTED", code: null },
  { caseId: "source-too-large", outcome: "REJECTED", code: "SOURCE_TOO_LARGE" },
  { caseId: "unsupported-source", outcome: "REJECTED", code: "UNSUPPORTED_FORMAT" },
  { caseId: "parent-traversal", outcome: "REJECTED", code: "UNSAFE_PATH" },
  { caseId: "nested-parent-traversal", outcome: "REJECTED", code: "UNSAFE_PATH" },
  { caseId: "absolute-posix-path", outcome: "REJECTED", code: "UNSAFE_PATH" },
  { caseId: "windows-drive-path", outcome: "REJECTED", code: "UNSAFE_PATH" },
  { caseId: "backslash-path", outcome: "REJECTED", code: "UNSAFE_PATH" },
  { caseId: "reserved-device-path", outcome: "REJECTED", code: "UNSAFE_PATH" },
  { caseId: "non-nfc-path", outcome: "REJECTED", code: "UNSAFE_PATH" },
  { caseId: "exact-duplicate-path", outcome: "REJECTED", code: "DUPLICATE_PATH" },
  { caseId: "case-collision", outcome: "REJECTED", code: "DUPLICATE_PATH" },
  { caseId: "prefix-conflict", outcome: "REJECTED", code: "DUPLICATE_PATH" },
  { caseId: "symlink-entry", outcome: "REJECTED", code: "UNSAFE_ENTRY_TYPE" },
  {
    caseId: "unsupported-compression",
    outcome: "REJECTED",
    code: "UNSUPPORTED_COMPRESSION",
  },
  { caseId: "too-many-entries", outcome: "REJECTED", code: "ENTRY_COUNT_LIMIT" },
  { caseId: "oversized-entry", outcome: "REJECTED", code: "ENTRY_SIZE_LIMIT" },
  { caseId: "oversized-total", outcome: "REJECTED", code: "TOTAL_SIZE_LIMIT" },
  {
    caseId: "high-compression-ratio",
    outcome: "REJECTED",
    code: "COMPRESSION_RATIO_LIMIT",
  },
  { caseId: "malformed-archive", outcome: "REJECTED", code: "ARCHIVE_MALFORMED" },
  { caseId: "png-bad-crc", outcome: "REJECTED", code: "PNG_MALFORMED" },
  { caseId: "png-truncated-chunk", outcome: "REJECTED", code: "PNG_MALFORMED" },
  { caseId: "png-trailing-bytes", outcome: "REJECTED", code: "PNG_MALFORMED" },
  { caseId: "png-oversized-dimensions", outcome: "REJECTED", code: "PNG_MALFORMED" },
  {
    caseId: "unsupported-file-type",
    outcome: "REJECTED",
    code: "UNSUPPORTED_FILE_TYPE",
  },
] as const satisfies readonly ImportCaseEvidence[];

export type ImportLimits = {
  [Key in keyof typeof IMPORT_LIMITS]: (typeof IMPORT_LIMITS)[Key];
};

export type ValidArchiveEvidence = {
  sourceSha256: (typeof IMPORT_VALID_ARCHIVE_GOLDEN)["sourceSha256"];
  sourceBytes: (typeof IMPORT_VALID_ARCHIVE_GOLDEN)["sourceBytes"];
  entryCount: (typeof IMPORT_VALID_ARCHIVE_GOLDEN)["entryCount"];
  totalUncompressedBytes: (typeof IMPORT_VALID_ARCHIVE_GOLDEN)["totalUncompressedBytes"];
  scriptEntries: (typeof IMPORT_VALID_ARCHIVE_GOLDEN)["scriptEntries"];
  executedEntries: 0;
  quarantine: "inert";
  atomicPublish: true;
  reopenedHashVerified: true;
};

export type ValidDirectPngEvidence = {
  sourceSha256: (typeof IMPORT_VALID_DIRECT_PNG_GOLDEN)["sourceSha256"];
  sourceBytes: (typeof IMPORT_VALID_DIRECT_PNG_GOLDEN)["sourceBytes"];
  width: (typeof IMPORT_VALID_DIRECT_PNG_GOLDEN)["width"];
  height: (typeof IMPORT_VALID_DIRECT_PNG_GOLDEN)["height"];
  atomicPublish: true;
  reopenedHashVerified: true;
};

export type ImportDefenseEvidence = {
  traversalRejected: true;
  collisionRejected: true;
  unsafeEntryTypesRejected: true;
  sizeLimitsEnforced: true;
  compressionRatioEnforced: true;
  malformedArchiveRejected: true;
  strictPngValidated: true;
  unsupportedFilesRejected: true;
  outsideSentinelPreserved: true;
  stagingCleaned: true;
  scriptExecutionDisabled: true;
};

export type ImportProbeSuccess = {
  protocolVersion: 1;
  policyVersion: typeof IMPORT_POLICY_VERSION;
  fixtureCatalogSha256: typeof IMPORT_FIXTURE_CATALOG_SHA256;
  zipVersion: typeof IMPORT_ZIP_VERSION;
  pngVersion: typeof IMPORT_PNG_VERSION;
  limits: ImportLimits;
  cases: ImportCaseEvidence[];
  validArchive: ValidArchiveEvidence;
  validDirectPng: ValidDirectPngEvidence;
  defenses: ImportDefenseEvidence;
  cleanupPending: false;
};

export type ImportProbeFailure = {
  protocolVersion: 1;
  code: ImportProbeErrorCode;
  cleanupPending: boolean;
};

export type NativeInvoke = (command: typeof IMPORT_M1_COMMAND) => Promise<unknown>;

const SUCCESS_KEYS = [
  "cases",
  "cleanupPending",
  "defenses",
  "fixtureCatalogSha256",
  "limits",
  "pngVersion",
  "policyVersion",
  "protocolVersion",
  "validArchive",
  "validDirectPng",
  "zipVersion",
] as const;
const LIMIT_KEYS = Object.keys(IMPORT_LIMITS).sort();
const CASE_KEYS = ["caseId", "code", "outcome"] as const;
const VALID_ARCHIVE_KEYS = [
  "atomicPublish",
  "entryCount",
  "executedEntries",
  "quarantine",
  "reopenedHashVerified",
  "scriptEntries",
  "sourceBytes",
  "sourceSha256",
  "totalUncompressedBytes",
] as const;
const VALID_DIRECT_PNG_KEYS = [
  "atomicPublish",
  "height",
  "reopenedHashVerified",
  "sourceBytes",
  "sourceSha256",
  "width",
] as const;
const DEFENSE_KEYS = [
  "collisionRejected",
  "compressionRatioEnforced",
  "malformedArchiveRejected",
  "outsideSentinelPreserved",
  "scriptExecutionDisabled",
  "sizeLimitsEnforced",
  "stagingCleaned",
  "strictPngValidated",
  "traversalRejected",
  "unsafeEntryTypesRejected",
  "unsupportedFilesRejected",
] as const;
const FAILURE_KEYS = ["cleanupPending", "code", "protocolVersion"] as const;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;

export class ImportProbeProtocolError extends Error {
  constructor() {
    super("import-hardening probe returned an invalid bounded response");
    this.name = "ImportProbeProtocolError";
  }
}

export class ImportProbeCommandError extends Error {
  readonly failure: ImportProbeFailure;

  constructor(failure: ImportProbeFailure) {
    super("import-hardening probe command failed");
    this.name = "ImportProbeCommandError";
    this.failure = failure;
  }
}

function failProtocol(): never {
  throw new ImportProbeProtocolError();
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
    new TextEncoder().encode(serialized).byteLength > IMPORT_LIMITS.ipcResponseBytes
  ) {
    failProtocol();
  }
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
  const sortedExpected = [...expected].sort();
  if (
    actual.length !== sortedExpected.length ||
    actual.some((key, index) => key !== sortedExpected[index])
  ) {
    failProtocol();
  }
}

function isErrorCode(value: unknown): value is ImportProbeErrorCode {
  return (
    typeof value === "string" &&
    (IMPORT_PROBE_ERROR_CODES as readonly string[]).includes(value)
  );
}

function parseSha256(value: unknown): string {
  if (typeof value !== "string" || !SHA256_PATTERN.test(value)) failProtocol();
  return value;
}

function parsePositiveInteger(value: unknown, maximum: number): number {
  if (!Number.isSafeInteger(value) || (value as number) <= 0 || (value as number) > maximum) {
    failProtocol();
  }
  return value as number;
}

function parseLimits(value: unknown): ImportLimits {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, LIMIT_KEYS);
  for (const [key, expected] of Object.entries(IMPORT_LIMITS)) {
    if (value[key] !== expected) failProtocol();
  }
  return { ...IMPORT_LIMITS };
}

function parseCases(value: unknown): ImportCaseEvidence[] {
  if (!Array.isArray(value) || value.length !== IMPORT_GOLDEN_CASES.length) {
    failProtocol();
  }
  const seen = new Set<string>();
  return value.map((entry, index) => {
    if (!isRecord(entry)) failProtocol();
    requireExactKeys(entry, CASE_KEYS);
    if (typeof entry.caseId !== "string" || seen.has(entry.caseId)) failProtocol();
    seen.add(entry.caseId);
    if (entry.outcome !== "ACCEPTED" && entry.outcome !== "REJECTED") failProtocol();
    if (entry.code !== null && !isErrorCode(entry.code)) failProtocol();
    const expected = IMPORT_GOLDEN_CASES[index];
    if (
      expected === undefined ||
      entry.caseId !== expected.caseId ||
      entry.outcome !== expected.outcome ||
      entry.code !== expected.code
    ) {
      failProtocol();
    }
    return {
      caseId: entry.caseId,
      outcome: entry.outcome,
      code: entry.code,
    };
  });
}

function parseValidArchive(value: unknown): ValidArchiveEvidence {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, VALID_ARCHIVE_KEYS);
  const sourceSha256 = parseSha256(value.sourceSha256);
  const sourceBytes = parsePositiveInteger(value.sourceBytes, IMPORT_LIMITS.sourceBytes);
  const totalUncompressedBytes = parsePositiveInteger(
    value.totalUncompressedBytes,
    IMPORT_LIMITS.totalUncompressedBytes,
  );
  if (
    sourceSha256 !== IMPORT_VALID_ARCHIVE_GOLDEN.sourceSha256 ||
    sourceBytes !== IMPORT_VALID_ARCHIVE_GOLDEN.sourceBytes ||
    value.entryCount !== IMPORT_VALID_ARCHIVE_GOLDEN.entryCount ||
    totalUncompressedBytes !== IMPORT_VALID_ARCHIVE_GOLDEN.totalUncompressedBytes ||
    value.scriptEntries !== IMPORT_VALID_ARCHIVE_GOLDEN.scriptEntries ||
    value.executedEntries !== 0 ||
    value.quarantine !== "inert" ||
    value.atomicPublish !== true ||
    value.reopenedHashVerified !== true
  ) {
    failProtocol();
  }
  return {
    sourceSha256: IMPORT_VALID_ARCHIVE_GOLDEN.sourceSha256,
    sourceBytes: IMPORT_VALID_ARCHIVE_GOLDEN.sourceBytes,
    entryCount: IMPORT_VALID_ARCHIVE_GOLDEN.entryCount,
    totalUncompressedBytes: IMPORT_VALID_ARCHIVE_GOLDEN.totalUncompressedBytes,
    scriptEntries: IMPORT_VALID_ARCHIVE_GOLDEN.scriptEntries,
    executedEntries: 0,
    quarantine: "inert",
    atomicPublish: true,
    reopenedHashVerified: true,
  };
}

function parseValidDirectPng(value: unknown): ValidDirectPngEvidence {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, VALID_DIRECT_PNG_KEYS);
  const sourceSha256 = parseSha256(value.sourceSha256);
  const sourceBytes = parsePositiveInteger(
    value.sourceBytes,
    Math.min(IMPORT_LIMITS.sourceBytes, IMPORT_LIMITS.pngBytes),
  );
  if (
    sourceSha256 !== IMPORT_VALID_DIRECT_PNG_GOLDEN.sourceSha256 ||
    sourceBytes !== IMPORT_VALID_DIRECT_PNG_GOLDEN.sourceBytes ||
    value.width !== IMPORT_VALID_DIRECT_PNG_GOLDEN.width ||
    value.height !== IMPORT_VALID_DIRECT_PNG_GOLDEN.height ||
    value.atomicPublish !== true ||
    value.reopenedHashVerified !== true
  ) {
    failProtocol();
  }
  return {
    sourceSha256: IMPORT_VALID_DIRECT_PNG_GOLDEN.sourceSha256,
    sourceBytes: IMPORT_VALID_DIRECT_PNG_GOLDEN.sourceBytes,
    width: IMPORT_VALID_DIRECT_PNG_GOLDEN.width,
    height: IMPORT_VALID_DIRECT_PNG_GOLDEN.height,
    atomicPublish: true,
    reopenedHashVerified: true,
  };
}

function parseDefenses(value: unknown): ImportDefenseEvidence {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, DEFENSE_KEYS);
  for (const key of DEFENSE_KEYS) {
    if (value[key] !== true) failProtocol();
  }
  return {
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
  };
}

export function parseImportProbeSuccess(value: unknown): ImportProbeSuccess {
  enforceSerializedResponseLimit(value);
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, SUCCESS_KEYS);
  if (
    value.protocolVersion !== 1 ||
    value.policyVersion !== IMPORT_POLICY_VERSION ||
    value.fixtureCatalogSha256 !== IMPORT_FIXTURE_CATALOG_SHA256 ||
    value.zipVersion !== IMPORT_ZIP_VERSION ||
    value.pngVersion !== IMPORT_PNG_VERSION ||
    value.cleanupPending !== false
  ) {
    failProtocol();
  }
  return {
    protocolVersion: 1,
    policyVersion: IMPORT_POLICY_VERSION,
    fixtureCatalogSha256: IMPORT_FIXTURE_CATALOG_SHA256,
    zipVersion: IMPORT_ZIP_VERSION,
    pngVersion: IMPORT_PNG_VERSION,
    limits: parseLimits(value.limits),
    cases: parseCases(value.cases),
    validArchive: parseValidArchive(value.validArchive),
    validDirectPng: parseValidDirectPng(value.validDirectPng),
    defenses: parseDefenses(value.defenses),
    cleanupPending: false,
  };
}

export function parseImportProbeFailure(value: unknown): ImportProbeFailure {
  enforceSerializedResponseLimit(value);
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, FAILURE_KEYS);
  if (
    value.protocolVersion !== 1 ||
    !isErrorCode(value.code) ||
    typeof value.cleanupPending !== "boolean"
  ) {
    failProtocol();
  }
  return {
    protocolVersion: 1,
    code: value.code,
    cleanupPending: value.cleanupPending,
  };
}

function invokeImportHardeningM1Command(): Promise<unknown> {
  return invoke<unknown>("run_import_hardening_m1_probe");
}

export async function runImportHardeningM1Probe(
  invokeNative: NativeInvoke = invokeImportHardeningM1Command,
): Promise<ImportProbeSuccess> {
  let rawResult: unknown;
  try {
    rawResult = await invokeNative(IMPORT_M1_COMMAND);
  } catch (rawFailure) {
    throw new ImportProbeCommandError(parseImportProbeFailure(rawFailure));
  }
  return parseImportProbeSuccess(rawResult);
}
