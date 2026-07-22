export const IMPORTED_EXECUTABLE_POLICY =
  "DISABLED_BY_SECURITY_POLICY" as const;
export const IMPORTED_EXECUTABLE_DISPOSITION = "INERT_QUARANTINED" as const;
export const IMPORTED_EXECUTABLE_METADATA_VERSION = 1 as const;
export const MAX_IMPORTED_EXECUTABLE_METADATA_BYTES = 4 * 1024;

const METADATA_KEYS = [
  "contentByteLength",
  "contentSha256",
  "language",
  "metadataVersion",
] as const;
const SHA_256_HEX = /^[0-9a-f]{64}$/;

const FIXED_POLICY = Object.freeze({
  disposition: IMPORTED_EXECUTABLE_DISPOSITION,
  executable: false,
  policy: IMPORTED_EXECUTABLE_POLICY,
} as const);

export interface ImportedExecutableMetadata {
  metadataVersion: typeof IMPORTED_EXECUTABLE_METADATA_VERSION;
  language: "JAVASCRIPT";
  contentByteLength: number;
  contentSha256: string;
}

export interface ImportedExecutablePolicyInfluences {
  manifest?: unknown;
  importSettings?: unknown;
  legacySettings?: unknown;
}

export interface QuarantinedImportedExecutable {
  disposition: typeof IMPORTED_EXECUTABLE_DISPOSITION;
  executable: false;
  policy: typeof IMPORTED_EXECUTABLE_POLICY;
  metadata: Readonly<ImportedExecutableMetadata>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function invalidMetadata(): Error {
  return new Error("invalid imported executable metadata");
}

function exceedsUtf8ByteLimit(value: string, limit: number): boolean {
  let bytes = 0;
  for (let index = 0; index < value.length; index += 1) {
    const codeUnit = value.charCodeAt(index);
    if (codeUnit <= 0x7f) {
      bytes += 1;
    } else if (codeUnit <= 0x7ff) {
      bytes += 2;
    } else if (codeUnit >= 0xd800 && codeUnit <= 0xdbff) {
      const nextCodeUnit = value.charCodeAt(index + 1);
      if (nextCodeUnit >= 0xdc00 && nextCodeUnit <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }

    if (bytes > limit) {
      return true;
    }
  }
  return false;
}

export function importedExecutablePolicy(
  _untrustedInfluences: ImportedExecutablePolicyInfluences = {},
): typeof FIXED_POLICY {
  return FIXED_POLICY;
}

export function parseQuarantinedJavaScriptMetadata(
  serializedMetadata: unknown,
): QuarantinedImportedExecutable {
  if (typeof serializedMetadata !== "string") {
    throw invalidMetadata();
  }

  if (
    exceedsUtf8ByteLimit(
      serializedMetadata,
      MAX_IMPORTED_EXECUTABLE_METADATA_BYTES,
    )
  ) {
    throw new Error("imported executable metadata exceeds byte limit");
  }

  let value: unknown;
  try {
    value = JSON.parse(serializedMetadata);
  } catch {
    throw invalidMetadata();
  }

  if (!isRecord(value)) {
    throw invalidMetadata();
  }

  const keys = Object.keys(value).sort();
  if (
    keys.length !== METADATA_KEYS.length ||
    !METADATA_KEYS.every((key, index) => keys[index] === key)
  ) {
    throw invalidMetadata();
  }

  if (
    value.metadataVersion !== IMPORTED_EXECUTABLE_METADATA_VERSION ||
    value.language !== "JAVASCRIPT" ||
    typeof value.contentByteLength !== "number" ||
    !Number.isSafeInteger(value.contentByteLength) ||
    value.contentByteLength < 0 ||
    typeof value.contentSha256 !== "string" ||
    !SHA_256_HEX.test(value.contentSha256)
  ) {
    throw invalidMetadata();
  }

  const metadata = Object.freeze({
    metadataVersion: value.metadataVersion,
    language: value.language,
    contentByteLength: value.contentByteLength,
    contentSha256: value.contentSha256,
  });

  return Object.freeze({
    ...importedExecutablePolicy(),
    metadata,
  });
}
