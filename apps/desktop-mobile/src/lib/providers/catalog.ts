export const LLM_PROVIDER_CATALOG_VERSION = 1 as const;

export type LlmProviderId =
  | "openai"
  | "anthropic"
  | "deepseek"
  | "ollama-cloud"
  | "google-gemini"
  | "google-vertex-ai";

export type ProviderAuthKind = "api-key" | "google-cloud-oauth";

export type ProviderSetupFieldId = "modelId" | "projectId" | "location";

export interface ProviderSetupField {
  readonly id: ProviderSetupFieldId;
  readonly label: string;
  readonly placeholder: string;
  readonly required: boolean;
}

export interface FixedProviderTarget {
  readonly kind: "fixed-origin";
  readonly origin: `https://${string}`;
}

export interface VertexProviderTarget {
  readonly kind: "google-vertex";
  readonly serviceDomain: "aiplatform.googleapis.com";
}

export interface LlmProviderDefinition {
  readonly id: LlmProviderId;
  readonly label: string;
  readonly description: string;
  readonly authKind: ProviderAuthKind;
  readonly authLabel: string;
  readonly target: FixedProviderTarget | VertexProviderTarget;
  readonly setupFields: readonly ProviderSetupField[];
  readonly documentationUrl: `https://${string}`;
  readonly status: "configuration-only";
}

const modelField = Object.freeze({
  id: "modelId",
  label: "모델 ID",
  placeholder: "연결 후 모델 목록에서 선택",
  required: true,
} satisfies ProviderSetupField);

const vertexFields = Object.freeze([
  modelField,
  Object.freeze({
    id: "projectId",
    label: "Google Cloud 프로젝트 ID",
    placeholder: "my-google-cloud-project",
    required: true,
  } satisfies ProviderSetupField),
  Object.freeze({
    id: "location",
    label: "리전",
    placeholder: "global 또는 asia-northeast3",
    required: true,
  } satisfies ProviderSetupField),
] as const);

function fixedProvider(
  definition: Omit<LlmProviderDefinition, "setupFields" | "status" | "target"> & {
    origin: `https://${string}`;
  },
): LlmProviderDefinition {
  const { origin, ...rest } = definition;
  return Object.freeze({
    ...rest,
    status: "configuration-only",
    target: Object.freeze({ kind: "fixed-origin", origin }),
    setupFields: Object.freeze([modelField]),
  });
}

export const LLM_PROVIDER_CATALOG = Object.freeze([
  fixedProvider({
    id: "openai",
    label: "OpenAI",
    description: "Responses API와 OpenAI 모델을 사용합니다.",
    authKind: "api-key",
    authLabel: "OpenAI API 키",
    origin: "https://api.openai.com",
    documentationUrl: "https://platform.openai.com/docs/api-reference/introduction",
  }),
  fixedProvider({
    id: "anthropic",
    label: "Anthropic",
    description: "Messages API와 Claude 모델을 사용합니다.",
    authKind: "api-key",
    authLabel: "Anthropic API 키",
    origin: "https://api.anthropic.com",
    documentationUrl: "https://platform.claude.com/docs/en/api/overview",
  }),
  fixedProvider({
    id: "deepseek",
    label: "DeepSeek",
    description: "DeepSeek API의 현재 모델 목록을 연결 시 조회합니다.",
    authKind: "api-key",
    authLabel: "DeepSeek API 키",
    origin: "https://api.deepseek.com",
    documentationUrl: "https://api-docs.deepseek.com/",
  }),
  fixedProvider({
    id: "ollama-cloud",
    label: "Ollama Cloud",
    description: "ollama.com의 클라우드 API를 사용합니다. 로컬 Ollama와 별개입니다.",
    authKind: "api-key",
    authLabel: "Ollama API 키",
    origin: "https://ollama.com",
    documentationUrl: "https://docs.ollama.com/cloud",
  }),
  fixedProvider({
    id: "google-gemini",
    label: "Google Gemini",
    description: "Gemini Developer API를 사용합니다. Vertex AI와 별개입니다.",
    authKind: "api-key",
    authLabel: "Gemini API 키",
    origin: "https://generativelanguage.googleapis.com",
    documentationUrl: "https://ai.google.dev/api",
  }),
  Object.freeze({
    id: "google-vertex-ai",
    label: "Vertex AI Gemini",
    description: "Google Cloud 프로젝트와 OAuth로 Vertex AI의 Gemini를 사용합니다.",
    authKind: "google-cloud-oauth",
    authLabel: "Google Cloud 로그인",
    target: Object.freeze({
      kind: "google-vertex",
      serviceDomain: "aiplatform.googleapis.com",
    }),
    setupFields: vertexFields,
    documentationUrl:
      "https://cloud.google.com/vertex-ai/generative-ai/docs/start/quickstart",
    status: "configuration-only",
  } satisfies LlmProviderDefinition),
] as const satisfies readonly LlmProviderDefinition[]);

type ApiKeyProviderId = Exclude<LlmProviderId, "google-vertex-ai">;

export type ProviderProfileDraft =
  | Readonly<{
      providerId: ApiKeyProviderId;
      modelId: string;
    }>
  | Readonly<{
      providerId: "google-vertex-ai";
      modelId: string;
      projectId: string;
      location: string;
    }>;

type VertexProviderProfileDraft = Extract<
  ProviderProfileDraft,
  { providerId: "google-vertex-ai" }
>;

type ApiKeyProviderProfileDraft = Extract<
  ProviderProfileDraft,
  { providerId: ApiKeyProviderId }
>;

export function createProviderProfileDraft(
  providerId: "google-vertex-ai",
): VertexProviderProfileDraft;
export function createProviderProfileDraft(
  providerId: ApiKeyProviderId,
): ApiKeyProviderProfileDraft;
export function createProviderProfileDraft(
  providerId: LlmProviderId,
): ProviderProfileDraft;
export function createProviderProfileDraft(
  providerId: LlmProviderId,
): ProviderProfileDraft {
  if (providerId === "google-vertex-ai") {
    return Object.freeze({
      providerId,
      modelId: "",
      projectId: "",
      location: "",
    });
  }

  return Object.freeze({
    providerId,
    modelId: "",
  });
}

export function getLlmProvider(id: LlmProviderId): LlmProviderDefinition {
  const provider = LLM_PROVIDER_CATALOG.find((candidate) => candidate.id === id);
  if (!provider) {
    throw new Error("UNKNOWN_LLM_PROVIDER");
  }
  return provider;
}
