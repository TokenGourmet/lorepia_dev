import {
  getLlmProvider,
  type ApiKeyProviderId,
  type LlmProviderId,
} from "./catalog";

export const MAX_MODEL_ID_BYTES = 256;

export type ActiveProviderProfile = Readonly<{
  providerId: ApiKeyProviderId;
  modelId: string;
}>;

export type CredentialState = boolean | null | "error";

const modelIds = $state<Partial<Record<ApiKeyProviderId, string>>>({});
const credentials = $state<Partial<Record<ApiKeyProviderId, CredentialState>>>({});
const credentialEpochs: Partial<Record<LlmProviderId, number>> = {};
let selectedProviderId = $state<LlmProviderId>("openai");
let activeProfile = $state<ActiveProviderProfile | null>(null);

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

export function modelIdValidationMessage(
  providerId: LlmProviderId,
  value: string,
): string | null {
  const normalized = value.trim();
  if (normalized.length === 0) {
    return "모델 ID를 입력하세요.";
  }
  if (/[\u0000-\u001f\u007f-\u009f]/u.test(normalized)) {
    return "모델 ID에는 제어 문자를 넣을 수 없습니다.";
  }
  if (utf8Length(normalized) > MAX_MODEL_ID_BYTES) {
    return `모델 ID는 UTF-8 ${MAX_MODEL_ID_BYTES}바이트 이하여야 합니다.`;
  }
  if (
    (providerId === "google-gemini" || providerId === "google-vertex-ai") &&
    (!/^[A-Za-z0-9._-]+$/u.test(normalized) ||
      normalized === "." ||
      normalized === "..")
  ) {
    return "Gemini 모델 ID는 경로가 아닌 단일 식별자여야 합니다.";
  }
  return null;
}

function reconcile(): void {
  if (selectedProviderId === "google-vertex-ai") {
    activeProfile = null;
    return;
  }

  const modelId = modelIds[selectedProviderId] ?? "";
  if (
    credentials[selectedProviderId] === true &&
    modelIdValidationMessage(selectedProviderId, modelId) === null
  ) {
    activeProfile = Object.freeze({
      providerId: selectedProviderId,
      modelId: modelId.trim(),
    });
  } else {
    activeProfile = null;
  }
}

export const activeProviderProfile = {
  get selectedProviderId(): LlmProviderId {
    return selectedProviderId;
  },
  get modelId(): string {
    return selectedProviderId === "google-vertex-ai"
      ? ""
      : (modelIds[selectedProviderId] ?? "");
  },
  get credentialConfigured(): CredentialState {
    return selectedProviderId === "google-vertex-ai"
      ? false
      : (credentials[selectedProviderId] ?? null);
  },
  get current(): ActiveProviderProfile | null {
    return activeProfile;
  },
  get modelError(): string | null {
    if (selectedProviderId === "google-vertex-ai") {
      return "Vertex AI는 OAuth 연결이 구현되기 전까지 사용할 수 없습니다.";
    }
    return modelIdValidationMessage(
      selectedProviderId,
      modelIds[selectedProviderId] ?? "",
    );
  },
  get sendBlockReason(): string | null {
    const providerId = selectedProviderId;
    const provider = getLlmProvider(providerId);
    if (providerId === "google-vertex-ai") {
      return `${provider.label}는 OAuth 연결이 아직 지원되지 않아 메시지를 보낼 수 없습니다.`;
    }

    const credential = credentials[providerId] ?? null;
    if (credential === null) {
      return `${provider.authLabel} 상태를 확인하는 중이라 아직 메시지를 보낼 수 없습니다.`;
    }
    if (credential === "error") {
      return `${provider.authLabel} 상태를 확인하지 못해 메시지를 보낼 수 없습니다. 설정에서 다시 확인해 주세요.`;
    }
    if (credential === false) {
      return `${provider.authLabel}가 설정되지 않아 메시지를 보낼 수 없습니다.`;
    }

    const modelError = modelIdValidationMessage(
      providerId,
      modelIds[providerId] ?? "",
    );
    if (modelError === "모델 ID를 입력하세요.") {
      return "모델 ID가 설정되지 않아 메시지를 보낼 수 없습니다.";
    }
    if (modelError !== null) {
      return `모델 설정이 올바르지 않아 메시지를 보낼 수 없습니다. ${modelError}`;
    }
    return null;
  },
  select(providerId: LlmProviderId): void {
    selectedProviderId = providerId;
    reconcile();
  },
  setModelId(value: string): void {
    if (selectedProviderId === "google-vertex-ai") {
      return;
    }
    modelIds[selectedProviderId] = value;
    reconcile();
  },
  restoreNonSecretSettings(
    providerId: LlmProviderId,
    restoredModelIds: Readonly<Partial<Record<ApiKeyProviderId, string>>>,
  ): void {
    for (const existing of Object.keys(modelIds) as ApiKeyProviderId[]) {
      delete modelIds[existing];
    }
    for (const [id, modelId] of Object.entries(restoredModelIds) as [
      ApiKeyProviderId,
      string,
    ][]) {
      modelIds[id] = modelId;
    }
    selectedProviderId = providerId;
    reconcile();
  },
  nonSecretModelIds(): Readonly<Partial<Record<ApiKeyProviderId, string>>> {
    return Object.freeze({ ...modelIds });
  },
  setCredentialConfigured(
    providerId: LlmProviderId,
    configured: CredentialState,
  ): void {
    if (providerId !== "google-vertex-ai") {
      credentials[providerId] = configured;
    }
    if (providerId === selectedProviderId) {
      reconcile();
    }
  },
  beginCredentialOperation(providerId: LlmProviderId): number {
    const next = (credentialEpochs[providerId] ?? 0) + 1;
    credentialEpochs[providerId] = next;
    return next;
  },
  isCredentialOperationCurrent(
    providerId: LlmProviderId,
    epoch: number,
  ): boolean {
    return credentialEpochs[providerId] === epoch;
  },
  reset(): void {
    for (const providerId of Object.keys(modelIds) as ApiKeyProviderId[]) {
      delete modelIds[providerId];
    }
    for (const providerId of Object.keys(credentials) as ApiKeyProviderId[]) {
      delete credentials[providerId];
    }
    for (const providerId of Object.keys(credentialEpochs) as LlmProviderId[]) {
      delete credentialEpochs[providerId];
    }
    selectedProviderId = "openai";
    activeProfile = null;
  },
};
