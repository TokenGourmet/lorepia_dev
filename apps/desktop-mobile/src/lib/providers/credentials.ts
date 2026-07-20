import { invoke } from "@tauri-apps/api/core";

import type { LlmProviderId } from "./catalog";

export interface ProviderCredentialStatus {
  provider: LlmProviderId;
  configured: boolean;
}

export const CREDENTIAL_ERROR_CODES = [
  "UNSUPPORTED_PROVIDER",
  "INVALID_SECRET",
  "SECRET_TOO_LARGE",
  "NOT_CONFIGURED",
  "STORE_UNAVAILABLE",
  "STORE_LOCKED",
  "STORE_FAILURE",
  "INTERNAL_STATE",
] as const;

export type CredentialErrorCode = (typeof CREDENTIAL_ERROR_CODES)[number];

export const CREDENTIAL_ERROR_MESSAGES: Record<CredentialErrorCode, string> = {
  UNSUPPORTED_PROVIDER: "이 제공자는 API 키 저장을 지원하지 않습니다.",
  INVALID_SECRET: "키 형식이 올바르지 않습니다. 값을 확인해 주세요.",
  SECRET_TOO_LARGE: "키가 허용 길이를 초과했습니다.",
  NOT_CONFIGURED: "저장된 키가 없습니다.",
  STORE_UNAVAILABLE: "이 기기의 보안 저장소를 사용할 수 없습니다.",
  STORE_LOCKED: "보안 저장소가 잠겨 있습니다. 잠금 해제 후 다시 시도해 주세요.",
  STORE_FAILURE: "보안 저장소 작업이 실패했습니다.",
  INTERNAL_STATE: "내부 오류가 발생했습니다. 다시 시도해 주세요.",
};

export const CREDENTIAL_UNKNOWN_ERROR_MESSAGE =
  "자격증명 작업을 완료하지 못했습니다.";

type InvokeCommand = (
  command: string,
  args?: Record<string, unknown>,
) => Promise<unknown>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseCredentialStatus(
  value: unknown,
  provider: LlmProviderId,
): ProviderCredentialStatus {
  if (
    !isRecord(value) ||
    value.provider !== provider ||
    typeof value.configured !== "boolean"
  ) {
    throw new Error("invalid credential status envelope");
  }
  return { provider, configured: value.configured };
}

export function publicCredentialErrorMessage(error: unknown): string {
  if (isRecord(error) && typeof error.code === "string") {
    const code = error.code as CredentialErrorCode;
    if (CREDENTIAL_ERROR_CODES.includes(code)) {
      return CREDENTIAL_ERROR_MESSAGES[code];
    }
  }
  return CREDENTIAL_UNKNOWN_ERROR_MESSAGE;
}

export async function requestCredentialStatus(
  provider: LlmProviderId,
  invokeCommand: InvokeCommand = invoke,
): Promise<ProviderCredentialStatus> {
  const response = await invokeCommand("get_provider_credential_status", {
    provider,
  });
  return parseCredentialStatus(response, provider);
}

export async function saveProviderApiKey(
  provider: LlmProviderId,
  secret: string,
  invokeCommand: InvokeCommand = invoke,
): Promise<ProviderCredentialStatus> {
  const response = await invokeCommand("save_provider_api_key", {
    provider,
    secret,
  });
  return parseCredentialStatus(response, provider);
}

export async function deleteProviderCredential(
  provider: LlmProviderId,
  invokeCommand: InvokeCommand = invoke,
): Promise<ProviderCredentialStatus> {
  const response = await invokeCommand("delete_provider_credential", {
    provider,
  });
  return parseCredentialStatus(response, provider);
}
