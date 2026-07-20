import { describe, expect, it } from "vitest";

import type { ApiKeyProviderId } from "./catalog";
import {
  FIRST_CHAT_MAX_INPUT_BYTES,
  buildFirstChatCommand,
} from "./first-chat-request";

const providers: ApiKeyProviderId[] = [
  "openai",
  "anthropic",
  "deepseek",
  "ollama-cloud",
  "google-gemini",
];

describe("first chat command", () => {
  it.each(providers)("sends only the bounded %s profile and user message", (providerId) => {
    expect(
      buildFirstChatCommand(
        { providerId, modelId: "model-example" },
        "  안녕하세요  ",
      ),
    ).toEqual({
      profile: { providerId, modelId: "model-example" },
      userMessage: "안녕하세요",
    });
  });

  it("contains no raw prompt, endpoint, options, preset, memory, persona, or tool surface", () => {
    const serialized = JSON.stringify(
      buildFirstChatCommand(
        { providerId: "openai", modelId: "model-example" },
        "hello",
      ),
    );
    expect(serialized).not.toMatch(
      /system|prompt|endpoint|options|preset|memory|persona|tools?|mcp|override/i,
    );
  });

  it("rejects empty, NUL, and oversized input", () => {
    expect(() =>
      buildFirstChatCommand(
        { providerId: "openai", modelId: "model-example" },
        " ",
      ),
    ).toThrow("FIRST_CHAT_MESSAGE_EMPTY");
    expect(() =>
      buildFirstChatCommand(
        { providerId: "openai", modelId: "model-example" },
        "bad\0message",
      ),
    ).toThrow("FIRST_CHAT_MESSAGE_CONTAINS_NUL");
    expect(() =>
      buildFirstChatCommand(
        { providerId: "openai", modelId: "model-example" },
        "a".repeat(FIRST_CHAT_MAX_INPUT_BYTES + 1),
      ),
    ).toThrow("FIRST_CHAT_MESSAGE_TOO_LARGE");
  });

  it("rejects a profile that cannot pass the native model contract", () => {
    expect(() =>
      buildFirstChatCommand(
        { providerId: "google-gemini", modelId: "models/escape" },
        "hello",
      ),
    ).toThrow("FIRST_CHAT_PROFILE_INVALID");
  });
});
