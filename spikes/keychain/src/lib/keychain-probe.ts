import { invoke } from "@tauri-apps/api/core";

export const KEYCHAIN_M1_COMMAND = "run_keychain_m1_probe" as const;

export const KEYCHAIN_BACKENDS = [
  "macos-keychain",
  "ios-protected-data",
  "windows-credential-manager",
  "linux-secret-service",
  "android-keystore-encrypted-preferences",
] as const;

export const KEYCHAIN_PROBE_ERROR_CODES = [
  "PROBE_BUSY",
  "STORE_UNAVAILABLE",
  "STORE_LOCKED",
  "STORE_FAILURE",
  "CLEANUP_FAILED",
  "COLLISION",
  "RANDOM_FAILURE",
  "INTERNAL_STATE",
] as const;

export type KeychainBackend = (typeof KEYCHAIN_BACKENDS)[number];
export type KeychainProbeErrorCode = (typeof KEYCHAIN_PROBE_ERROR_CODES)[number];

export type KeychainLifecycleEvidence = {
  absentBeforeCreate: true;
  created: true;
  initialReadMatched: true;
  updated: true;
  updatedReadMatched: true;
  deleted: true;
  absentAfterDelete: true;
};

export type KeychainProbeSuccess = {
  runId: string;
  backend: KeychainBackend;
  referenceFingerprint: string;
  lifecycle: KeychainLifecycleEvidence;
  staleCleanupRecovered: boolean;
  cleanupPending: false;
};

export type KeychainProbeFailure = {
  code: KeychainProbeErrorCode;
  cleanupPending: boolean;
};

export type NativeInvoke = (command: typeof KEYCHAIN_M1_COMMAND) => Promise<unknown>;

const SUCCESS_KEYS = [
  "backend",
  "cleanupPending",
  "lifecycle",
  "referenceFingerprint",
  "runId",
  "staleCleanupRecovered",
] as const;

const LIFECYCLE_KEYS = [
  "absentAfterDelete",
  "absentBeforeCreate",
  "created",
  "deleted",
  "initialReadMatched",
  "updated",
  "updatedReadMatched",
] as const;

const FAILURE_KEYS = ["cleanupPending", "code"] as const;
const FORBIDDEN_KEY_PARTS = ["secret", "password", "token", "raw", "account", "reference"];
const RUN_ID_PATTERN = /^[0-9a-f]{32}$/;
const REFERENCE_FINGERPRINT_PATTERN = /^[0-9a-f]{16}$/;

export class KeychainProbeProtocolError extends Error {
  constructor() {
    super("keychain probe returned an invalid bounded response");
    this.name = "KeychainProbeProtocolError";
  }
}

export class KeychainProbeCommandError extends Error {
  readonly failure: KeychainProbeFailure;

  constructor(failure: KeychainProbeFailure) {
    super("keychain probe command failed");
    this.name = "KeychainProbeCommandError";
    this.failure = failure;
  }
}

function failProtocol(): never {
  throw new KeychainProbeProtocolError();
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

function containsForbiddenKey(key: string): boolean {
  if (key === "referenceFingerprint") return false;
  const normalized = key.toLowerCase();
  return FORBIDDEN_KEY_PARTS.some((part) => normalized.includes(part));
}

function rejectSecretLikeFields(value: unknown): void {
  if (Array.isArray(value)) {
    for (const entry of value) rejectSecretLikeFields(entry);
    return;
  }
  if (!isRecord(value)) return;
  for (const [key, entry] of Object.entries(value)) {
    if (containsForbiddenKey(key)) failProtocol();
    rejectSecretLikeFields(entry);
  }
}

function parseLifecycle(value: unknown): KeychainLifecycleEvidence {
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, LIFECYCLE_KEYS);
  for (const key of LIFECYCLE_KEYS) {
    if (value[key] !== true) failProtocol();
  }
  return value as KeychainLifecycleEvidence;
}

export function parseKeychainProbeSuccess(value: unknown): KeychainProbeSuccess {
  rejectSecretLikeFields(value);
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, SUCCESS_KEYS);

  if (typeof value.runId !== "string" || !RUN_ID_PATTERN.test(value.runId)) {
    failProtocol();
  }
  if (
    typeof value.referenceFingerprint !== "string" ||
    !REFERENCE_FINGERPRINT_PATTERN.test(value.referenceFingerprint)
  ) {
    failProtocol();
  }
  if (
    typeof value.backend !== "string" ||
    !(KEYCHAIN_BACKENDS as readonly string[]).includes(value.backend)
  ) {
    failProtocol();
  }
  if (typeof value.staleCleanupRecovered !== "boolean") failProtocol();
  if (value.cleanupPending !== false) failProtocol();

  return {
    runId: value.runId,
    backend: value.backend as KeychainBackend,
    referenceFingerprint: value.referenceFingerprint,
    lifecycle: parseLifecycle(value.lifecycle),
    staleCleanupRecovered: value.staleCleanupRecovered,
    cleanupPending: false,
  };
}

export function parseKeychainProbeFailure(value: unknown): KeychainProbeFailure {
  rejectSecretLikeFields(value);
  if (!isRecord(value)) failProtocol();
  requireExactKeys(value, FAILURE_KEYS);
  if (
    typeof value.code !== "string" ||
    !(KEYCHAIN_PROBE_ERROR_CODES as readonly string[]).includes(value.code)
  ) {
    failProtocol();
  }
  if (typeof value.cleanupPending !== "boolean") failProtocol();
  return {
    code: value.code as KeychainProbeErrorCode,
    cleanupPending: value.cleanupPending,
  };
}

function invokeKeychainM1Command(): Promise<unknown> {
  return invoke<unknown>("run_keychain_m1_probe");
}

export async function runKeychainM1Probe(
  invokeNative: NativeInvoke = invokeKeychainM1Command,
): Promise<KeychainProbeSuccess> {
  let rawResult: unknown;
  try {
    rawResult = await invokeNative(KEYCHAIN_M1_COMMAND);
  } catch (rawFailure) {
    throw new KeychainProbeCommandError(parseKeychainProbeFailure(rawFailure));
  }
  return parseKeychainProbeSuccess(rawResult);
}
