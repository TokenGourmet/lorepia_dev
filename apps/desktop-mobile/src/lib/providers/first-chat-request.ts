import {
  modelIdValidationMessage,
  type ActiveProviderProfile,
} from "./active-profile.svelte";

export const FIRST_CHAT_MAX_INPUT_BYTES = 64 * 1024;

export type FirstChatCommand = Readonly<{
  chatId: string;
  profile: ActiveProviderProfile;
  userMessage: string;
}>;

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function validateUserMessage(value: string): string {
  const normalized = value.trim();
  if (normalized.length === 0) {
    throw new Error("FIRST_CHAT_MESSAGE_EMPTY");
  }
  if (normalized.includes("\0")) {
    throw new Error("FIRST_CHAT_MESSAGE_CONTAINS_NUL");
  }
  if (utf8Length(normalized) > FIRST_CHAT_MAX_INPUT_BYTES) {
    throw new Error("FIRST_CHAT_MESSAGE_TOO_LARGE");
  }
  return normalized;
}

export function buildFirstChatCommand(
  profile: ActiveProviderProfile,
  chatId: string,
  userMessage: string,
): FirstChatCommand {
  if (modelIdValidationMessage(profile.providerId, profile.modelId) !== null) {
    throw new Error("FIRST_CHAT_PROFILE_INVALID");
  }
  if (!/^[a-f0-9]{32}$/u.test(chatId)) {
    throw new Error("FIRST_CHAT_ID_INVALID");
  }
  return Object.freeze({
    chatId,
    profile: Object.freeze({
      providerId: profile.providerId,
      modelId: profile.modelId.trim(),
    }),
    userMessage: validateUserMessage(userMessage),
  });
}
