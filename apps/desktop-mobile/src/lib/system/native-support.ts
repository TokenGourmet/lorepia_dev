import { invoke } from "@tauri-apps/api/core";

import type { LlmProviderId } from "$lib/providers/catalog";

export const ASSET_STORE_STATUS_COMMAND = "get_asset_store_status" as const;
export const PRODUCT_SAFETY_CONTRACT_COMMAND =
  "get_product_safety_contract" as const;
export const AI_OUTPUT_REPORT_COMMAND = "prepare_ai_output_report" as const;
export const REDACTED_DIAGNOSTICS_COMMAND =
  "export_redacted_diagnostics" as const;

export const NATIVE_SUPPORT_ERROR_MESSAGE =
  "기기 지원 정보를 처리하지 못했습니다.";

const MAX_U64 = 18_446_744_073_709_551_615n;
const MAX_ARTIFACT_BYTES = 32 * 1024;
export const MAX_REPORT_COMMENT_BYTES = 4 * 1024;
export const MAX_REPORT_EXCERPT_BYTES = 16 * 1024;
const MAX_MESSAGE_ID_BYTES = 128;
const CARGO_SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/u;

const ASSET_ERROR_CODES = [
  "ASSET_PATH_UNAVAILABLE",
  "ASSET_SCHEMA_INCOMPATIBLE",
  "ASSET_FILESYSTEM_UNSAFE",
  "ASSET_STORE_UNAVAILABLE",
  "ASSET_INTERNAL",
] as const;

const SAFETY_PROVIDER_KINDS = [
  "OPEN_AI",
  "ANTHROPIC",
  "DEEP_SEEK",
  "OLLAMA_CLOUD",
  "GOOGLE_GEMINI",
  "GOOGLE_VERTEX_AI",
] as const;

const REQUEST_DATA_CATEGORIES = [
  "CURRENT_CONVERSATION_CONTEXT",
  "ACTIVE_CHARACTER_AND_PERSONA_CONTEXT",
  "ACTIVE_PROMPT_AND_LORE_CONTEXT",
  "REQUESTED_MEDIA_WHEN_PROVIDER_SUPPORTS_IT",
] as const;

const CREDENTIAL_POLICIES = [
  "NATIVE_VAULT_ONLY",
  "REQUEST_AUTHORIZATION_HEADER_ONLY",
  "NEVER_IN_DIAGNOSTIC_EXPORT",
] as const;

const DIAGNOSTIC_POLICIES = [
  "LOCAL_USER_INITIATED_EXPORT_ONLY",
  "ALLOWLISTED_METADATA_WITHOUT_USER_CONTENT",
] as const;

const AI_REPORT_CATEGORIES = [
  "SAFETY_CONCERN",
  "HARASSMENT_OR_HATE",
  "SEXUAL_CONTENT",
  "SELF_HARM",
  "ILLEGAL_OR_DANGEROUS",
  "PRIVACY_CONCERN",
  "COPYRIGHT_CONCERN",
  "INCORRECT_OR_LOW_QUALITY",
  "OTHER",
] as const;

const DIAGNOSTIC_CODES = [
  "STORAGE_UNAVAILABLE",
  "STORAGE_RECOVERED_INTERRUPTED_REQUEST",
  "STORAGE_WAL_MAINTENANCE_DEFERRED",
  "PROVIDER_NETWORK_UNAVAILABLE",
  "PROVIDER_RATE_LIMITED",
  "PROVIDER_PROTOCOL_REJECTED",
  "PROVIDER_STREAM_CANCELLED",
  "PROVIDER_STREAM_ACK_TIMEOUT",
  "ASSET_CATALOG_NEEDS_RECONCILIATION",
  "BACKUP_INTERRUPTED",
] as const;

const PLATFORMS = [
  "WINDOWS",
  "MAC_OS",
  "LINUX",
  "ANDROID",
  "IOS",
  "UNKNOWN",
] as const;

const ARCHITECTURES = ["X86_64", "AARCH64", "UNKNOWN"] as const;

export type AssetStoreErrorCode = (typeof ASSET_ERROR_CODES)[number];
export type SafetyProviderKind = (typeof SAFETY_PROVIDER_KINDS)[number];
export type AiReportCategory = (typeof AI_REPORT_CATEGORIES)[number];

export type AssetStoreLimits = Readonly<{
  maxObjectBytes: string;
  maxTotalBytes: string;
  maxImageWidth: number;
  maxImageHeight: number;
  maxImagePixels: number;
}>;

export type AssetStoreStats = Readonly<{
  objectCount: string;
  activeBytes: string;
  referenceCount: string;
  missingCount: string;
  quarantinedCount: string;
  stagingCount: string;
}>;

export type AssetStoreStatus = Readonly<{
  contractVersion: 1;
  available: boolean;
  supportedSchemaVersion: number;
  errorCode: AssetStoreErrorCode | null;
  limits: AssetStoreLimits;
  stats: AssetStoreStats | null;
}>;

export type ProductSafetyContract = Readonly<{
  contractVersion: 1;
  releaseProfile: "STORE_SAFE";
  requestDestination: "USER_SELECTED_LLM_PROVIDER_ONLY";
  providerDestinations: readonly SafetyProviderKind[];
  requestData: readonly (typeof REQUEST_DATA_CATEGORIES)[number][];
  credentials: readonly (typeof CREDENTIAL_POLICIES)[number][];
  diagnostics: readonly (typeof DIAGNOSTIC_POLICIES)[number][];
  importedJavascript: "DISABLED_BY_SECURITY_POLICY";
  importedLua: "DISABLED_BY_SECURITY_POLICY";
  support: Readonly<{
    privacyPolicyUrlConfigured: boolean;
    supportUrlConfigured: boolean;
    remoteReportSubmissionConfigured: boolean;
  }>;
}>;

export type AiOutputReportInput = Readonly<{
  messageId: string;
  provider: SafetyProviderKind;
  category: AiReportCategory;
  userComment: string | null;
  selectedOutputExcerpt: string | null;
  includeSelectedOutput: boolean;
}>;

export type SafetyArtifact = Readonly<{
  fileName: string;
  mediaType: string;
  byteLength: number;
  json: string;
}>;

export type NativeSupportInvoker = (
  command: string,
  args?: Record<string, unknown>,
) => Promise<unknown>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function exactRecord(
  value: unknown,
  keys: readonly string[],
  error: string,
): Record<string, unknown> {
  if (!isRecord(value)) throw new Error(error);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (
    actual.length !== expected.length ||
    !actual.every((key, index) => key === expected[index])
  ) {
    throw new Error(error);
  }
  return value;
}

function recordWithOptionalKeys(
  value: unknown,
  required: readonly string[],
  optional: readonly string[],
  error: string,
): Record<string, unknown> {
  if (!isRecord(value)) throw new Error(error);
  const allowed = new Set([...required, ...optional]);
  const actual = Object.keys(value);
  if (
    !required.every((key) => Object.hasOwn(value, key)) ||
    actual.some((key) => !allowed.has(key))
  ) {
    throw new Error(error);
  }
  return value;
}

function isSafeNonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function isBoundedText(
  value: unknown,
  maxBytes: number,
  allowEmpty = false,
): value is string {
  return (
    typeof value === "string" &&
    (allowEmpty || value.length > 0) &&
    !value.includes("\0") &&
    utf8ByteLength(value) <= maxBytes
  );
}

function isCanonicalU64(value: unknown): value is string {
  if (typeof value !== "string" || !/^(?:0|[1-9]\d*)$/u.test(value)) {
    return false;
  }
  try {
    return BigInt(value) <= MAX_U64;
  } catch {
    return false;
  }
}

function isOneOf<const T extends readonly string[]>(
  value: unknown,
  values: T,
): value is T[number] {
  return typeof value === "string" && values.includes(value);
}

function parseExactEnumArray<const T extends readonly string[]>(
  value: unknown,
  expected: T,
  error: string,
): readonly T[number][] {
  if (
    !Array.isArray(value) ||
    value.length !== expected.length ||
    !value.every((item, index) => item === expected[index])
  ) {
    throw new Error(error);
  }
  return Object.freeze([...expected]);
}

function parseAssetLimits(value: unknown): AssetStoreLimits {
  const record = exactRecord(
    value,
    [
      "maxObjectBytes",
      "maxTotalBytes",
      "maxImageWidth",
      "maxImageHeight",
      "maxImagePixels",
    ],
    "INVALID_ASSET_STORE_STATUS",
  );
  if (
    !isCanonicalU64(record.maxObjectBytes) ||
    record.maxObjectBytes === "0" ||
    !isCanonicalU64(record.maxTotalBytes) ||
    BigInt(record.maxTotalBytes) < BigInt(record.maxObjectBytes) ||
    !isSafeNonNegativeInteger(record.maxImageWidth) ||
    record.maxImageWidth < 1 ||
    !isSafeNonNegativeInteger(record.maxImageHeight) ||
    record.maxImageHeight < 1 ||
    !isSafeNonNegativeInteger(record.maxImagePixels) ||
    record.maxImagePixels < 1
  ) {
    throw new Error("INVALID_ASSET_STORE_STATUS");
  }
  return Object.freeze({
    maxObjectBytes: record.maxObjectBytes,
    maxTotalBytes: record.maxTotalBytes,
    maxImageWidth: record.maxImageWidth,
    maxImageHeight: record.maxImageHeight,
    maxImagePixels: record.maxImagePixels,
  });
}

function parseAssetStats(value: unknown): AssetStoreStats {
  const record = exactRecord(
    value,
    [
      "objectCount",
      "activeBytes",
      "referenceCount",
      "missingCount",
      "quarantinedCount",
      "stagingCount",
    ],
    "INVALID_ASSET_STORE_STATUS",
  );
  if (
    !isCanonicalU64(record.objectCount) ||
    !isCanonicalU64(record.activeBytes) ||
    !isCanonicalU64(record.referenceCount) ||
    !isCanonicalU64(record.missingCount) ||
    !isCanonicalU64(record.quarantinedCount) ||
    !isCanonicalU64(record.stagingCount)
  ) {
    throw new Error("INVALID_ASSET_STORE_STATUS");
  }
  return Object.freeze({
    objectCount: record.objectCount,
    activeBytes: record.activeBytes,
    referenceCount: record.referenceCount,
    missingCount: record.missingCount,
    quarantinedCount: record.quarantinedCount,
    stagingCount: record.stagingCount,
  });
}

export function parseAssetStoreStatus(value: unknown): AssetStoreStatus {
  const record = exactRecord(
    value,
    [
      "contractVersion",
      "available",
      "supportedSchemaVersion",
      "errorCode",
      "limits",
      "stats",
    ],
    "INVALID_ASSET_STORE_STATUS",
  );
  if (
    record.contractVersion !== 1 ||
    typeof record.available !== "boolean" ||
    !isSafeNonNegativeInteger(record.supportedSchemaVersion) ||
    record.supportedSchemaVersion < 1 ||
    !(
      record.errorCode === null ||
      isOneOf(record.errorCode, ASSET_ERROR_CODES)
    ) ||
    record.available !== (record.errorCode === null) ||
    record.available !== (record.stats !== null)
  ) {
    throw new Error("INVALID_ASSET_STORE_STATUS");
  }
  return Object.freeze({
    contractVersion: 1,
    available: record.available,
    supportedSchemaVersion: record.supportedSchemaVersion,
    errorCode: record.errorCode,
    limits: parseAssetLimits(record.limits),
    stats: record.stats === null ? null : parseAssetStats(record.stats),
  });
}

export function parseProductSafetyContract(
  value: unknown,
): ProductSafetyContract {
  const record = exactRecord(
    value,
    [
      "contractVersion",
      "releaseProfile",
      "requestDestination",
      "providerDestinations",
      "requestData",
      "credentials",
      "diagnostics",
      "importedJavascript",
      "importedLua",
      "support",
    ],
    "INVALID_PRODUCT_SAFETY_CONTRACT",
  );
  const support = exactRecord(
    record.support,
    [
      "privacyPolicyUrlConfigured",
      "supportUrlConfigured",
      "remoteReportSubmissionConfigured",
    ],
    "INVALID_PRODUCT_SAFETY_CONTRACT",
  );
  if (
    record.contractVersion !== 1 ||
    record.releaseProfile !== "STORE_SAFE" ||
    record.requestDestination !== "USER_SELECTED_LLM_PROVIDER_ONLY" ||
    record.importedJavascript !== "DISABLED_BY_SECURITY_POLICY" ||
    record.importedLua !== "DISABLED_BY_SECURITY_POLICY" ||
    typeof support.privacyPolicyUrlConfigured !== "boolean" ||
    typeof support.supportUrlConfigured !== "boolean" ||
    typeof support.remoteReportSubmissionConfigured !== "boolean"
  ) {
    throw new Error("INVALID_PRODUCT_SAFETY_CONTRACT");
  }
  return Object.freeze({
    contractVersion: 1,
    releaseProfile: "STORE_SAFE",
    requestDestination: "USER_SELECTED_LLM_PROVIDER_ONLY",
    providerDestinations: parseExactEnumArray(
      record.providerDestinations,
      SAFETY_PROVIDER_KINDS,
      "INVALID_PRODUCT_SAFETY_CONTRACT",
    ),
    requestData: parseExactEnumArray(
      record.requestData,
      REQUEST_DATA_CATEGORIES,
      "INVALID_PRODUCT_SAFETY_CONTRACT",
    ),
    credentials: parseExactEnumArray(
      record.credentials,
      CREDENTIAL_POLICIES,
      "INVALID_PRODUCT_SAFETY_CONTRACT",
    ),
    diagnostics: parseExactEnumArray(
      record.diagnostics,
      DIAGNOSTIC_POLICIES,
      "INVALID_PRODUCT_SAFETY_CONTRACT",
    ),
    importedJavascript: "DISABLED_BY_SECURITY_POLICY",
    importedLua: "DISABLED_BY_SECURITY_POLICY",
    support: Object.freeze({
      privacyPolicyUrlConfigured: support.privacyPolicyUrlConfigured,
      supportUrlConfigured: support.supportUrlConfigured,
      remoteReportSubmissionConfigured:
        support.remoteReportSubmissionConfigured,
    }),
  });
}

function parseArtifactEnvelope(
  value: unknown,
  fileName: string,
  mediaType: string,
): SafetyArtifact {
  const record = exactRecord(
    value,
    ["fileName", "mediaType", "byteLength", "json"],
    "INVALID_SAFETY_ARTIFACT",
  );
  if (
    record.fileName !== fileName ||
    record.mediaType !== mediaType ||
    !isSafeNonNegativeInteger(record.byteLength) ||
    record.byteLength < 1 ||
    record.byteLength > MAX_ARTIFACT_BYTES ||
    typeof record.json !== "string" ||
    utf8ByteLength(record.json) !== record.byteLength
  ) {
    throw new Error("INVALID_SAFETY_ARTIFACT");
  }
  try {
    JSON.parse(record.json);
  } catch {
    throw new Error("INVALID_SAFETY_ARTIFACT");
  }
  return Object.freeze({
    fileName,
    mediaType,
    byteLength: record.byteLength,
    json: record.json,
  });
}

function normalizedOptionalText(
  value: unknown,
  maxBytes: number,
  error: string,
): string | null {
  if (value === null) return null;
  if (!isBoundedText(value, maxBytes, true)) throw new Error(error);
  const normalized = value.trim();
  return normalized.length === 0 ? null : normalized;
}

export function validateAiOutputReportInput(
  value: AiOutputReportInput,
): AiOutputReportInput {
  const record = exactRecord(
    value,
    [
      "messageId",
      "provider",
      "category",
      "userComment",
      "selectedOutputExcerpt",
      "includeSelectedOutput",
    ],
    "INVALID_AI_OUTPUT_REPORT_INPUT",
  );
  if (
    !isBoundedText(record.messageId, MAX_MESSAGE_ID_BYTES) ||
    !/^[A-Za-z0-9_-]+$/u.test(record.messageId) ||
    !isOneOf(record.provider, SAFETY_PROVIDER_KINDS) ||
    !isOneOf(record.category, AI_REPORT_CATEGORIES) ||
    typeof record.includeSelectedOutput !== "boolean"
  ) {
    throw new Error("INVALID_AI_OUTPUT_REPORT_INPUT");
  }
  const userComment = normalizedOptionalText(
    record.userComment,
    MAX_REPORT_COMMENT_BYTES,
    "INVALID_AI_OUTPUT_REPORT_INPUT",
  );
  const selectedOutputExcerpt = normalizedOptionalText(
    record.selectedOutputExcerpt,
    MAX_REPORT_EXCERPT_BYTES,
    "INVALID_AI_OUTPUT_REPORT_INPUT",
  );
  if (record.includeSelectedOutput !== (selectedOutputExcerpt !== null)) {
    throw new Error("INVALID_AI_OUTPUT_REPORT_INPUT");
  }
  return Object.freeze({
    messageId: record.messageId,
    provider: record.provider,
    category: record.category,
    userComment,
    selectedOutputExcerpt,
    includeSelectedOutput: record.includeSelectedOutput,
  });
}

function parseAiOutputReportJson(
  artifact: SafetyArtifact,
  input: AiOutputReportInput,
): void {
  const parsed = JSON.parse(artifact.json) as unknown;
  const record = recordWithOptionalKeys(
    parsed,
    [
      "contractVersion",
      "reportId",
      "createdAtMs",
      "messageId",
      "provider",
      "category",
      "containsUserSelectedContent",
      "readyForUserReview",
      "submitted",
      "networkRequestCreated",
    ],
    ["userComment", "selectedOutputExcerpt"],
    "INVALID_AI_OUTPUT_REPORT_ARTIFACT",
  );
  if (
    record.contractVersion !== 1 ||
    typeof record.reportId !== "string" ||
    !/^[a-f0-9]{32}$/u.test(record.reportId) ||
    !isSafeNonNegativeInteger(record.createdAtMs) ||
    record.messageId !== input.messageId ||
    record.provider !== input.provider ||
    record.category !== input.category ||
    record.userComment !==
      (input.userComment === null ? undefined : input.userComment) ||
    record.selectedOutputExcerpt !==
      (input.selectedOutputExcerpt === null
        ? undefined
        : input.selectedOutputExcerpt) ||
    record.containsUserSelectedContent !== input.includeSelectedOutput ||
    record.readyForUserReview !== true ||
    record.submitted !== false ||
    record.networkRequestCreated !== false
  ) {
    throw new Error("INVALID_AI_OUTPUT_REPORT_ARTIFACT");
  }
}

export function parseAiOutputReportArtifact(
  value: unknown,
  input: AiOutputReportInput,
): SafetyArtifact {
  const validatedInput = validateAiOutputReportInput(input);
  const artifact = parseArtifactEnvelope(
    value,
    "lorepia-ai-output-report.json",
    "application/vnd.lorepia.ai-output-report+json",
  );
  parseAiOutputReportJson(artifact, validatedInput);
  return artifact;
}

function parseRedactedDiagnosticJson(artifact: SafetyArtifact): void {
  const parsed = JSON.parse(artifact.json) as unknown;
  const record = exactRecord(
    parsed,
    [
      "contractVersion",
      "generatedAtMs",
      "appVersion",
      "platform",
      "architecture",
      "storage",
      "recentCodes",
      "privacy",
    ],
    "INVALID_REDACTED_DIAGNOSTICS",
  );
  const storage = exactRecord(
    record.storage,
    ["available", "schemaVersion", "recoveredInterruptedRequests"],
    "INVALID_REDACTED_DIAGNOSTICS",
  );
  const privacy = exactRecord(
    record.privacy,
    [
      "containsApiCredentials",
      "containsPromptOrLoreContent",
      "containsChatOrPersonaContent",
      "containsFileSystemPaths",
      "containsRawProviderErrors",
    ],
    "INVALID_REDACTED_DIAGNOSTICS",
  );
  if (
    record.contractVersion !== 1 ||
    !isSafeNonNegativeInteger(record.generatedAtMs) ||
    typeof record.appVersion !== "string" ||
    !CARGO_SEMVER.test(record.appVersion) ||
    !isOneOf(record.platform, PLATFORMS) ||
    !isOneOf(record.architecture, ARCHITECTURES) ||
    typeof storage.available !== "boolean" ||
    !(
      storage.schemaVersion === null ||
      isSafeNonNegativeInteger(storage.schemaVersion)
    ) ||
    storage.available !== (storage.schemaVersion !== null) ||
    !isSafeNonNegativeInteger(storage.recoveredInterruptedRequests) ||
    !Array.isArray(record.recentCodes) ||
    record.recentCodes.length > 64 ||
    !record.recentCodes.every((code) => isOneOf(code, DIAGNOSTIC_CODES)) ||
    new Set(record.recentCodes).size !== record.recentCodes.length ||
    privacy.containsApiCredentials !== false ||
    privacy.containsPromptOrLoreContent !== false ||
    privacy.containsChatOrPersonaContent !== false ||
    privacy.containsFileSystemPaths !== false ||
    privacy.containsRawProviderErrors !== false
  ) {
    throw new Error("INVALID_REDACTED_DIAGNOSTICS");
  }
}

export function parseRedactedDiagnosticsArtifact(
  value: unknown,
): SafetyArtifact {
  const artifact = parseArtifactEnvelope(
    value,
    "lorepia-diagnostics.json",
    "application/vnd.lorepia.diagnostics+json",
  );
  parseRedactedDiagnosticJson(artifact);
  return artifact;
}

export async function requestAssetStoreStatus(
  invokeCommand: NativeSupportInvoker = invoke,
): Promise<AssetStoreStatus> {
  return parseAssetStoreStatus(
    await invokeCommand(ASSET_STORE_STATUS_COMMAND),
  );
}

export async function requestProductSafetyContract(
  invokeCommand: NativeSupportInvoker = invoke,
): Promise<ProductSafetyContract> {
  return parseProductSafetyContract(
    await invokeCommand(PRODUCT_SAFETY_CONTRACT_COMMAND),
  );
}

export async function requestAiOutputReport(
  input: AiOutputReportInput,
  invokeCommand: NativeSupportInvoker = invoke,
): Promise<SafetyArtifact> {
  const validatedInput = validateAiOutputReportInput(input);
  return parseAiOutputReportArtifact(
    await invokeCommand(AI_OUTPUT_REPORT_COMMAND, { input: validatedInput }),
    validatedInput,
  );
}

export async function requestRedactedDiagnostics(
  invokeCommand: NativeSupportInvoker = invoke,
): Promise<SafetyArtifact> {
  return parseRedactedDiagnosticsArtifact(
    await invokeCommand(REDACTED_DIAGNOSTICS_COMMAND),
  );
}

export function toSafetyProviderKind(
  providerId: LlmProviderId,
): SafetyProviderKind {
  switch (providerId) {
    case "openai":
      return "OPEN_AI";
    case "anthropic":
      return "ANTHROPIC";
    case "deepseek":
      return "DEEP_SEEK";
    case "ollama-cloud":
      return "OLLAMA_CLOUD";
    case "google-gemini":
      return "GOOGLE_GEMINI";
    case "google-vertex-ai":
      return "GOOGLE_VERTEX_AI";
  }
}

export function publicNativeSupportError(error?: unknown): string {
  if (isRecord(error) && typeof error.code === "string") {
    switch (error.code) {
      case "SAFETY_INPUT_INVALID":
      case "AI_REPORT_CONTENT_CONSENT_REQUIRED":
      case "AI_REPORT_CONTENT_CONSENT_MISMATCH":
        return "포함할 내용과 동의 상태를 확인해 주세요.";
      case "SAFETY_INPUT_TOO_LARGE":
        return "입력 내용이 허용 길이를 초과했습니다.";
      case "DIAGNOSTIC_BUNDLE_TOO_LARGE":
        return "진단 정보가 허용 크기를 초과했습니다.";
      case "SYSTEM_CLOCK_INVALID":
        return "기기 시각을 확인한 뒤 다시 시도해 주세요.";
    }
  }
  return NATIVE_SUPPORT_ERROR_MESSAGE;
}
