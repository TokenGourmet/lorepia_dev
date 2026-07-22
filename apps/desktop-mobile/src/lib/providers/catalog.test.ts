import { describe, expect, it } from "vitest";

import {
  LLM_PROVIDER_CATALOG,
  LLM_PROVIDER_CATALOG_VERSION,
  createProviderProfileDraft,
  getLlmProvider,
} from "./catalog";

const expectedProviderIds = [
  "openai",
  "anthropic",
  "deepseek",
  "ollama-cloud",
  "google-gemini",
  "google-vertex-ai",
];

describe("LLM provider catalog", () => {
  it("publishes the exact first-party configuration set", () => {
    expect(LLM_PROVIDER_CATALOG_VERSION).toBe(1);
    expect(LLM_PROVIDER_CATALOG.map(({ id }) => id)).toEqual(expectedProviderIds);
    expect(new Set(expectedProviderIds).size).toBe(expectedProviderIds.length);
  });

  it("keeps Gemini Developer API and Vertex AI as different contracts", () => {
    const gemini = getLlmProvider("google-gemini");
    const vertex = getLlmProvider("google-vertex-ai");

    expect(gemini.authKind).toBe("api-key");
    expect(gemini.target).toEqual({
      kind: "fixed-origin",
      origin: "https://generativelanguage.googleapis.com",
    });
    expect(vertex.authKind).toBe("google-cloud-oauth");
    expect(vertex.target).toEqual({
      kind: "google-vertex",
      serviceDomain: "aiplatform.googleapis.com",
    });
    expect(vertex.setupFields.map(({ id }) => id)).toEqual([
      "modelId",
      "projectId",
      "location",
    ]);
  });

  it("treats Ollama Cloud as a fixed HTTPS service, not a local daemon", () => {
    const ollamaCloud = getLlmProvider("ollama-cloud");

    expect(ollamaCloud.target).toEqual({
      kind: "fixed-origin",
      origin: "https://ollama.com",
    });
    expect(JSON.stringify(ollamaCloud)).not.toMatch(/localhost|127\.0\.0\.1|baseUrl/i);
  });

  it("does not ship a mutable custom endpoint or a hard-coded model choice", () => {
    for (const provider of LLM_PROVIDER_CATALOG) {
      expect(provider.status).toBe(
        provider.id === "google-vertex-ai"
          ? "configuration-only"
          : "first-chat-ready",
      );
      expect(provider.setupFields[0]?.id).toBe("modelId");
      expect(provider.setupFields[0]?.placeholder).toContain("연결 후");
      expect(JSON.stringify(provider)).not.toMatch(/defaultModel|customEndpoint/i);
    }
  });

  it("creates a non-secret draft only", () => {
    const serialized = JSON.stringify(createProviderProfileDraft("anthropic"));

    expect(JSON.parse(serialized)).toEqual({
      providerId: "anthropic",
      modelId: "",
    });
    expect(serialized).not.toMatch(/api.?key|secret|token|credential|service.?account/i);
  });

  it("keeps project and location fields exclusive to Vertex AI", () => {
    expect(createProviderProfileDraft("google-vertex-ai")).toEqual({
      providerId: "google-vertex-ai",
      modelId: "",
      projectId: "",
      location: "",
    });
    expect(createProviderProfileDraft("openai")).toEqual({
      providerId: "openai",
      modelId: "",
    });
  });
});
