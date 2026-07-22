import { describe, expect, it, vi } from "vitest";

import {
  CREDENTIAL_UNKNOWN_ERROR_MESSAGE,
  parseCredentialStatus,
  publicCredentialErrorMessage,
  requestCredentialStatus,
  saveProviderApiKey,
  deleteProviderCredential,
} from "./credentials";

describe("parseCredentialStatus", () => {
  it("accepts an exact provider-bound envelope", () => {
    expect(
      parseCredentialStatus({ provider: "openai", configured: true }, "openai"),
    ).toEqual({ provider: "openai", configured: true });
  });

  it("rejects a status for a different provider", () => {
    expect(() =>
      parseCredentialStatus(
        { provider: "anthropic", configured: true },
        "openai",
      ),
    ).toThrow();
  });

  it("rejects malformed envelopes", () => {
    for (const bad of [null, [], "ok", { provider: "openai" }, {}]) {
      expect(() => parseCredentialStatus(bad, "openai")).toThrow();
    }
  });
});

describe("credential commands", () => {
  it("passes the provider and never echoes the secret back", async () => {
    const invokeCommand = vi
      .fn()
      .mockResolvedValue({ provider: "anthropic", configured: true });
    const status = await saveProviderApiKey(
      "anthropic",
      "sk-test",
      invokeCommand,
    );
    expect(invokeCommand).toHaveBeenCalledWith("save_provider_api_key", {
      provider: "anthropic",
      secret: "sk-test",
    });
    expect(status).toEqual({ provider: "anthropic", configured: true });
    expect(JSON.stringify(status)).not.toContain("sk-test");
  });

  it("requests status and delete with provider-only arguments", async () => {
    const invokeCommand = vi
      .fn()
      .mockResolvedValue({ provider: "deepseek", configured: false });
    await requestCredentialStatus("deepseek", invokeCommand);
    await deleteProviderCredential("deepseek", invokeCommand);
    expect(invokeCommand).toHaveBeenNthCalledWith(
      1,
      "get_provider_credential_status",
      { provider: "deepseek" },
    );
    expect(invokeCommand).toHaveBeenNthCalledWith(
      2,
      "delete_provider_credential",
      { provider: "deepseek" },
    );
  });
});

describe("publicCredentialErrorMessage", () => {
  it("maps every native code to fixed user-facing copy", () => {
    expect(publicCredentialErrorMessage({ code: "STORE_LOCKED" })).toContain(
      "잠겨",
    );
    expect(publicCredentialErrorMessage({ code: "INVALID_SECRET" })).toContain(
      "형식",
    );
  });

  it("never reflects unknown error content", () => {
    const message = publicCredentialErrorMessage({
      code: "SOMETHING_ELSE",
      detail: "sk-leaked-value",
    });
    expect(message).toBe(CREDENTIAL_UNKNOWN_ERROR_MESSAGE);
    expect(message).not.toContain("sk-leaked-value");
  });
});
