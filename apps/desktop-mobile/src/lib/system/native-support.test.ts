import { describe, expect, it, vi } from "vitest";

import {
  AI_OUTPUT_REPORT_COMMAND,
  ASSET_STORE_STATUS_COMMAND,
  PRODUCT_SAFETY_CONTRACT_COMMAND,
  REDACTED_DIAGNOSTICS_COMMAND,
  parseAssetStoreStatus,
  parseProductSafetyContract,
  parseRedactedDiagnosticsArtifact,
  requestAiOutputReport,
  requestAssetStoreStatus,
  requestProductSafetyContract,
  requestRedactedDiagnostics,
  toSafetyProviderKind,
  utf8ByteLength,
  validateAiOutputReportInput,
  type AiOutputReportInput,
} from "./native-support";

const VALID_ASSET_STATUS = {
  contractVersion: 1,
  available: true,
  supportedSchemaVersion: 1,
  errorCode: null,
  limits: {
    maxObjectBytes: "1073741824",
    maxTotalBytes: "9223372036854775807",
    maxImageWidth: 16_384,
    maxImageHeight: 16_384,
    maxImagePixels: 67_108_864,
  },
  stats: {
    objectCount: "3",
    activeBytes: "1024",
    referenceCount: "4",
    missingCount: "0",
    quarantinedCount: "0",
    stagingCount: "0",
  },
} as const;

const VALID_SAFETY_CONTRACT = {
  contractVersion: 1,
  releaseProfile: "STORE_SAFE",
  requestDestination: "USER_SELECTED_LLM_PROVIDER_ONLY",
  providerDestinations: [
    "OPEN_AI",
    "ANTHROPIC",
    "DEEP_SEEK",
    "OLLAMA_CLOUD",
    "GOOGLE_GEMINI",
    "GOOGLE_VERTEX_AI",
  ],
  requestData: [
    "CURRENT_CONVERSATION_CONTEXT",
    "ACTIVE_CHARACTER_AND_PERSONA_CONTEXT",
    "ACTIVE_PROMPT_AND_LORE_CONTEXT",
    "REQUESTED_MEDIA_WHEN_PROVIDER_SUPPORTS_IT",
  ],
  credentials: [
    "NATIVE_VAULT_ONLY",
    "REQUEST_AUTHORIZATION_HEADER_ONLY",
    "NEVER_IN_DIAGNOSTIC_EXPORT",
  ],
  diagnostics: [
    "LOCAL_USER_INITIATED_EXPORT_ONLY",
    "ALLOWLISTED_METADATA_WITHOUT_USER_CONTENT",
  ],
  importedJavascript: "DISABLED_BY_SECURITY_POLICY",
  importedLua: "DISABLED_BY_SECURITY_POLICY",
  support: {
    privacyPolicyUrlConfigured: false,
    supportUrlConfigured: false,
    remoteReportSubmissionConfigured: false,
  },
} as const;

function artifact(
  fileName: string,
  mediaType: string,
  value: unknown,
): {
  fileName: string;
  mediaType: string;
  byteLength: number;
  json: string;
} {
  const json = `${JSON.stringify(value, null, 2)}\n`;
  return {
    fileName,
    mediaType,
    byteLength: utf8ByteLength(json),
    json,
  };
}

const VALID_DIAGNOSTIC_VALUE = {
  contractVersion: 1,
  generatedAtMs: 1_800_000_000_000,
  appVersion: "0.1.0",
  platform: "ANDROID",
  architecture: "AARCH64",
  storage: {
    available: true,
    schemaVersion: 1,
    recoveredInterruptedRequests: 0,
  },
  recentCodes: [],
  privacy: {
    containsApiCredentials: false,
    containsPromptOrLoreContent: false,
    containsChatOrPersonaContent: false,
    containsFileSystemPaths: false,
    containsRawProviderErrors: false,
  },
} as const;

const REPORT_INPUT: AiOutputReportInput = {
  messageId: "message_123",
  provider: "OPEN_AI",
  category: "OTHER",
  userComment: "검토 필요",
  selectedOutputExcerpt: null,
  includeSelectedOutput: false,
};

describe("native support contracts", () => {
  it("accepts the exact asset status and rejects widened or inconsistent data", () => {
    expect(parseAssetStoreStatus(VALID_ASSET_STATUS)).toEqual(
      VALID_ASSET_STATUS,
    );
    expect(() =>
      parseAssetStoreStatus({ ...VALID_ASSET_STATUS, sqlPath: "/private/db" }),
    ).toThrow("INVALID_ASSET_STORE_STATUS");
    expect(() =>
      parseAssetStoreStatus({
        ...VALID_ASSET_STATUS,
        available: false,
        errorCode: "ASSET_STORE_UNAVAILABLE",
      }),
    ).toThrow("INVALID_ASSET_STORE_STATUS");
    expect(() =>
      parseAssetStoreStatus({
        ...VALID_ASSET_STATUS,
        stats: { ...VALID_ASSET_STATUS.stats, activeBytes: "01" },
      }),
    ).toThrow("INVALID_ASSET_STORE_STATUS");
  });

  it("locks the safety disclosure to the native v1 policy", () => {
    expect(parseProductSafetyContract(VALID_SAFETY_CONTRACT)).toEqual(
      VALID_SAFETY_CONTRACT,
    );
    expect(() =>
      parseProductSafetyContract({
        ...VALID_SAFETY_CONTRACT,
        providerDestinations: [
          ...VALID_SAFETY_CONTRACT.providerDestinations,
        ].reverse(),
      }),
    ).toThrow("INVALID_PRODUCT_SAFETY_CONTRACT");
    expect(() =>
      parseProductSafetyContract({
        ...VALID_SAFETY_CONTRACT,
        importedLua: "ENABLED",
      }),
    ).toThrow("INVALID_PRODUCT_SAFETY_CONTRACT");
  });

  it("validates UTF-8 report limits and explicit excerpt consent", () => {
    expect(utf8ByteLength("한")).toBe(3);
    expect(
      validateAiOutputReportInput({
        ...REPORT_INPUT,
        userComment: ` ${"한".repeat(100)} `,
      }).userComment,
    ).toBe("한".repeat(100));
    expect(() =>
      validateAiOutputReportInput({
        ...REPORT_INPUT,
        userComment: "한".repeat(1_366),
      }),
    ).toThrow("INVALID_AI_OUTPUT_REPORT_INPUT");
    expect(() =>
      validateAiOutputReportInput({
        ...REPORT_INPUT,
        selectedOutputExcerpt: "selected",
        includeSelectedOutput: false,
      }),
    ).toThrow("INVALID_AI_OUTPUT_REPORT_INPUT");
  });

  it("rejects diagnostics that fail the privacy proof or byte envelope", () => {
    const valid = artifact(
      "lorepia-diagnostics.json",
      "application/vnd.lorepia.diagnostics+json",
      VALID_DIAGNOSTIC_VALUE,
    );
    expect(parseRedactedDiagnosticsArtifact(valid)).toEqual(valid);

    const unsafe = artifact(
      "lorepia-diagnostics.json",
      "application/vnd.lorepia.diagnostics+json",
      {
        ...VALID_DIAGNOSTIC_VALUE,
        privacy: {
          ...VALID_DIAGNOSTIC_VALUE.privacy,
          containsChatOrPersonaContent: true,
        },
      },
    );
    expect(() => parseRedactedDiagnosticsArtifact(unsafe)).toThrow(
      "INVALID_REDACTED_DIAGNOSTICS",
    );
    expect(() =>
      parseRedactedDiagnosticsArtifact({
        ...valid,
        byteLength: valid.byteLength + 1,
      }),
    ).toThrow("INVALID_SAFETY_ARTIFACT");
  });

  it("invokes only the closed native commands with the validated report input", async () => {
    const reportValue = {
      contractVersion: 1,
      reportId: "a".repeat(32),
      createdAtMs: 1_800_000_000_000,
      messageId: REPORT_INPUT.messageId,
      provider: REPORT_INPUT.provider,
      category: REPORT_INPUT.category,
      userComment: REPORT_INPUT.userComment,
      containsUserSelectedContent: false,
      readyForUserReview: true,
      submitted: false,
      networkRequestCreated: false,
    };
    const invokeCommand = vi.fn(async (command: string) => {
      switch (command) {
        case ASSET_STORE_STATUS_COMMAND:
          return VALID_ASSET_STATUS;
        case PRODUCT_SAFETY_CONTRACT_COMMAND:
          return VALID_SAFETY_CONTRACT;
        case REDACTED_DIAGNOSTICS_COMMAND:
          return artifact(
            "lorepia-diagnostics.json",
            "application/vnd.lorepia.diagnostics+json",
            VALID_DIAGNOSTIC_VALUE,
          );
        case AI_OUTPUT_REPORT_COMMAND:
          return artifact(
            "lorepia-ai-output-report.json",
            "application/vnd.lorepia.ai-output-report+json",
            reportValue,
          );
        default:
          throw new Error(`unexpected ${command}`);
      }
    });

    await expect(requestAssetStoreStatus(invokeCommand)).resolves.toEqual(
      VALID_ASSET_STATUS,
    );
    await expect(
      requestProductSafetyContract(invokeCommand),
    ).resolves.toEqual(VALID_SAFETY_CONTRACT);
    await expect(requestRedactedDiagnostics(invokeCommand)).resolves.toMatchObject(
      { fileName: "lorepia-diagnostics.json" },
    );
    await expect(
      requestAiOutputReport(REPORT_INPUT, invokeCommand),
    ).resolves.toMatchObject({ fileName: "lorepia-ai-output-report.json" });
    expect(invokeCommand).toHaveBeenLastCalledWith(AI_OUTPUT_REPORT_COMMAND, {
      input: REPORT_INPUT,
    });
  });

  it("maps every product provider to the closed safety enum", () => {
    expect(toSafetyProviderKind("openai")).toBe("OPEN_AI");
    expect(toSafetyProviderKind("anthropic")).toBe("ANTHROPIC");
    expect(toSafetyProviderKind("deepseek")).toBe("DEEP_SEEK");
    expect(toSafetyProviderKind("ollama-cloud")).toBe("OLLAMA_CLOUD");
    expect(toSafetyProviderKind("google-gemini")).toBe("GOOGLE_GEMINI");
    expect(toSafetyProviderKind("google-vertex-ai")).toBe(
      "GOOGLE_VERTEX_AI",
    );
  });
});
