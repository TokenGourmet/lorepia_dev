import { describe, expect, it } from "vitest";

import type { ApiKeyProviderId } from "./catalog";
import {
  FIRST_CHAT_MAX_INPUT_BYTES,
  buildFirstChatCommand,
  firstChatInputBlockReason,
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
        "a".repeat(32),
        "  안녕하세요  ",
      ),
    ).toEqual({
      chatId: "a".repeat(32),
      profile: { providerId, modelId: "model-example" },
      userMessage: "안녕하세요",
    });
  });

  it("contains no raw prompt, endpoint, options, preset, memory, persona, or tool surface", () => {
    const serialized = JSON.stringify(
      buildFirstChatCommand(
        { providerId: "openai", modelId: "model-example" },
        "a".repeat(32),
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
        "a".repeat(32),
        " ",
      ),
    ).toThrow("FIRST_CHAT_MESSAGE_EMPTY");
    expect(() =>
      buildFirstChatCommand(
        { providerId: "openai", modelId: "model-example" },
        "a".repeat(32),
        "bad\0message",
      ),
    ).toThrow("FIRST_CHAT_MESSAGE_CONTAINS_NUL");
    expect(() =>
      buildFirstChatCommand(
        { providerId: "openai", modelId: "model-example" },
        "a".repeat(32),
        "a".repeat(FIRST_CHAT_MAX_INPUT_BYTES + 1),
      ),
    ).toThrow("FIRST_CHAT_MESSAGE_TOO_LARGE");
  });

  it("explains UTF-8 input failures before the composer drops its draft", () => {
    expect(firstChatInputBlockReason("안녕하세요")).toBeNull();
    expect(firstChatInputBlockReason("bad\0message")).toBe(
      "메시지에 사용할 수 없는 문자가 포함되어 있어 보낼 수 없습니다.",
    );
    expect(
      firstChatInputBlockReason(
        "가".repeat(Math.floor(FIRST_CHAT_MAX_INPUT_BYTES / 3) + 1),
      ),
    ).toBe(
      "메시지는 UTF-8 65,536바이트 이하여야 합니다.",
    );
  });

  it("rejects a profile that cannot pass the native model contract", () => {
    expect(() =>
      buildFirstChatCommand(
        { providerId: "google-gemini", modelId: "models/escape" },
        "a".repeat(32),
        "hello",
      ),
    ).toThrow("FIRST_CHAT_PROFILE_INVALID");
  });

  it("rejects a chat identity outside the native storage contract", () => {
    expect(() =>
      buildFirstChatCommand(
        { providerId: "openai", modelId: "model-example" },
        "chat-a",
        "hello",
      ),
    ).toThrow("FIRST_CHAT_ID_INVALID");
  });
});
