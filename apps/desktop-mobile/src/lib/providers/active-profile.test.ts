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
});
