import { afterEach, describe, expect, it } from "vitest";

import {
  MAX_MODEL_ID_BYTES,
  activeProviderProfile,
  modelIdValidationMessage,
} from "./active-profile.svelte";

afterEach(() => activeProviderProfile.reset());

describe("active provider profile", () => {
  it("activates only a non-secret API-key profile with a saved credential", () => {
    activeProviderProfile.select("anthropic");
    activeProviderProfile.setModelId("  claude-example  ");
    expect(activeProviderProfile.current).toBeNull();

    activeProviderProfile.setCredentialConfigured("anthropic", true);
    expect(activeProviderProfile.current).toEqual({
      providerId: "anthropic",
      modelId: "claude-example",
    });
    expect(JSON.stringify(activeProviderProfile.current)).not.toMatch(
      /api.?key|secret|token|credential/i,
    );
  });

  it("deactivates when the credential is removed or Vertex is selected", () => {
    activeProviderProfile.setModelId("model-a");
    activeProviderProfile.setCredentialConfigured("openai", true);
    expect(activeProviderProfile.current).not.toBeNull();

    activeProviderProfile.setCredentialConfigured("openai", false);
    expect(activeProviderProfile.current).toBeNull();

    activeProviderProfile.select("google-vertex-ai");
    expect(activeProviderProfile.current).toBeNull();
  });

  it("explains every provider state that blocks a chat submission", () => {
    activeProviderProfile.select("anthropic");
    activeProviderProfile.setModelId("claude-example");
    expect(activeProviderProfile.sendBlockReason).toBe(
      "Anthropic API 키 상태를 확인하는 중이라 아직 메시지를 보낼 수 없습니다.",
    );

    activeProviderProfile.setCredentialConfigured("anthropic", "error");
    expect(activeProviderProfile.sendBlockReason).toBe(
      "Anthropic API 키 상태를 확인하지 못해 메시지를 보낼 수 없습니다. 설정에서 다시 확인해 주세요.",
    );

    activeProviderProfile.setCredentialConfigured("anthropic", false);
    expect(activeProviderProfile.sendBlockReason).toBe(
      "Anthropic API 키가 설정되지 않아 메시지를 보낼 수 없습니다.",
    );

    activeProviderProfile.setCredentialConfigured("anthropic", true);
    activeProviderProfile.setModelId("");
    expect(activeProviderProfile.sendBlockReason).toBe(
      "모델 ID가 설정되지 않아 메시지를 보낼 수 없습니다.",
    );

    activeProviderProfile.setModelId("bad\nmodel");
    expect(activeProviderProfile.sendBlockReason).toBe(
      "모델 설정이 올바르지 않아 메시지를 보낼 수 없습니다. 모델 ID에는 제어 문자를 넣을 수 없습니다.",
    );

    activeProviderProfile.setModelId("claude-example");
    expect(activeProviderProfile.sendBlockReason).toBeNull();

    activeProviderProfile.select("google-vertex-ai");
    expect(activeProviderProfile.sendBlockReason).toBe(
      "Vertex AI Gemini는 OAuth 연결이 아직 지원되지 않아 메시지를 보낼 수 없습니다.",
    );
  });

  it("validates the native model-id byte contract", () => {
    expect(modelIdValidationMessage("openai", "  ")).not.toBeNull();
    expect(modelIdValidationMessage("openai", "bad\nmodel")).not.toBeNull();
    expect(
      modelIdValidationMessage("openai", "a".repeat(MAX_MODEL_ID_BYTES)),
    ).toBeNull();
    expect(
      modelIdValidationMessage(
        "openai",
        "가".repeat(MAX_MODEL_ID_BYTES),
      ),
    ).not.toBeNull();
    expect(
      modelIdValidationMessage("google-gemini", "models/gemini"),
    ).not.toBeNull();
    expect(
      modelIdValidationMessage("google-gemini", "gemini-example"),
    ).toBeNull();
  });

  it("shares credential epochs across refresh and mutation owners", () => {
    const stale = activeProviderProfile.beginCredentialOperation("openai");
    const current = activeProviderProfile.beginCredentialOperation("openai");

    expect(
      activeProviderProfile.isCredentialOperationCurrent("openai", stale),
    ).toBe(false);
    expect(
      activeProviderProfile.isCredentialOperationCurrent("openai", current),
    ).toBe(true);
  });

  it("restores provider and model preferences without credential state", () => {
    activeProviderProfile.setCredentialConfigured("anthropic", true);
    activeProviderProfile.restoreNonSecretSettings("anthropic", {
      openai: "gpt-example",
      anthropic: "claude-example",
    });

    expect(activeProviderProfile.selectedProviderId).toBe("anthropic");
    expect(activeProviderProfile.modelId).toBe("claude-example");
    expect(activeProviderProfile.current).toEqual({
      providerId: "anthropic",
      modelId: "claude-example",
    });
    expect(activeProviderProfile.nonSecretModelIds()).toEqual({
      openai: "gpt-example",
      anthropic: "claude-example",
    });
    expect(JSON.stringify(activeProviderProfile.nonSecretModelIds())).not.toMatch(
      /credential|api.?key|secret/i,
    );
  });
});
