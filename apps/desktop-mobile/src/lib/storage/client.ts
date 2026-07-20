import { invoke } from "@tauri-apps/api/core";

import type { ThreadMode } from "$lib/chat/types";
import type {
  ApiKeyProviderId,
  LlmProviderId,
} from "$lib/providers/catalog";
import type { ThemePreference } from "$lib/design/theme.svelte";

const MAX_CHAT_PAGE = 100;
const MAX_MESSAGE_PAGE = 200;
const MAX_MESSAGE_HISTORY_PAGES = 50;
const MAX_MESSAGE_HISTORY_ITEMS = MAX_MESSAGE_PAGE * MAX_MESSAGE_HISTORY_PAGES;
const MAX_MESSAGE_HISTORY_TEXT_BYTES = 16 * 1024 * 1024;
const MAX_TITLE_BYTES = 1024;
const MAX_CHARACTER_ID_BYTES = 128;
const MAX_MESSAGE_BYTES = 1024 * 1024;

const PROVIDER_IDS = new Set<LlmProviderId>([
  "openai",
  "anthropic",
  "deepseek",
  "ollama-cloud",
  "google-gemini",
  "google-vertex-ai",
]);
const API_KEY_PROVIDER_IDS = new Set<ApiKeyProviderId>([
  "openai",
  "anthropic",
  "deepseek",
  "ollama-cloud",
  "google-gemini",
]);

export type StoredChat = Readonly<{
  id: string;
  characterId: string;
  title: string;
  revision: number;
  createdAtMs: number;
  updatedAtMs: number;
}>;

export type StoredMessage = Readonly<{
  id: string;
  chatId: string;
  ordinal: number;
  role: "user" | "assistant";
  text: string;
  state: "complete" | "partial" | "failed";
  createdAtMs: number;
  updatedAtMs: number;
}>;

export type AppPreferences = Readonly<{
  selectedProviderId: LlmProviderId;
  modelIds: Readonly<Partial<Record<ApiKeyProviderId, string>>>;
  theme: ThemePreference;
  defaultMode: ThreadMode;
}>;

export type VersionedAppPreferences = Readonly<{
  revision: number;
  value: AppPreferences;
}>;

export type ChatPage = Readonly<{
  items: readonly StoredChat[];
  nextCursor: ChatCursor | null;
}>;

export type MessagePage = Readonly<{
  items: readonly StoredMessage[];
  hasMore: boolean;
}>;

export type ChatCursor = Readonly<{
  updatedAtMs: number;
  chatId: string;
}>;

export type StorageStatus = Readonly<{
  available: boolean;
  schemaVersion: number | null;
  errorCode: string | null;
}>;

export type DeleteChatReceipt = Readonly<{
  chatId: string;
  deleted: boolean;
}>;

export type StorageInvoker = (
  command: string,
  args?: Record<string, unknown>,
) => Promise<unknown>;

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function exactRecord(
  value: unknown,
  keys: readonly string[],
  error: string,
): Record<string, unknown> {
  if (!isRecord(value)) throw new Error(error);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (
    actual.length !== expected.length ||
    !actual.every((key, index) => key === expected[index])
  ) {
    throw new Error(error);
  }
  return value;
}

function isSafeNonNegativeInteger(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 0
  );
}

function boundedString(
  value: unknown,
  maxBytes: number,
  allowEmpty = false,
): value is string {
  return (
    typeof value === "string" &&
    (allowEmpty || value.length > 0) &&
    !value.includes("\0") &&
    utf8Length(value) <= maxBytes
  );
}

function parseId(value: unknown, error: string): string {
  if (typeof value !== "string" || !/^[a-f0-9]{32}$/u.test(value)) {
    throw new Error(error);
  }
  return value;
}

function parseChat(value: unknown): StoredChat {
  const record = exactRecord(
    value,
    [
      "id",
      "characterId",
      "title",
      "revision",
      "createdAtMs",
      "updatedAtMs",
    ],
    "INVALID_STORED_CHAT",
  );
  if (
    !boundedString(record.characterId, MAX_CHARACTER_ID_BYTES) ||
    !boundedString(record.title, MAX_TITLE_BYTES, true) ||
    !isSafeNonNegativeInteger(record.revision) ||
    record.revision < 1 ||
    !isSafeNonNegativeInteger(record.createdAtMs) ||
    !isSafeNonNegativeInteger(record.updatedAtMs) ||
    record.updatedAtMs < record.createdAtMs
  ) {
    throw new Error("INVALID_STORED_CHAT");
  }
  return Object.freeze({
    id: parseId(record.id, "INVALID_STORED_CHAT"),
    characterId: record.characterId,
    title: record.title,
    revision: record.revision,
    createdAtMs: record.createdAtMs,
    updatedAtMs: record.updatedAtMs,
  });
}

function parseChatCursor(value: unknown): ChatCursor {
  const record = exactRecord(
    value,
    ["updatedAtMs", "chatId"],
    "INVALID_CHAT_PAGE",
  );
  if (!isSafeNonNegativeInteger(record.updatedAtMs)) {
    throw new Error("INVALID_CHAT_PAGE");
  }
  return Object.freeze({
    updatedAtMs: record.updatedAtMs,
    chatId: parseId(record.chatId, "INVALID_CHAT_PAGE"),
  });
}

function compareChatCursor(left: ChatCursor, right: ChatCursor): number {
  if (left.updatedAtMs !== right.updatedAtMs) {
    return left.updatedAtMs - right.updatedAtMs;
  }
  if (left.chatId === right.chatId) return 0;
  return left.chatId < right.chatId ? -1 : 1;
}

function parseMessage(value: unknown): StoredMessage {
  const record = exactRecord(
    value,
    [
      "id",
      "chatId",
      "ordinal",
      "role",
      "text",
      "state",
      "createdAtMs",
      "updatedAtMs",
    ],
    "INVALID_STORED_MESSAGE",
  );
  if (
    !isSafeNonNegativeInteger(record.ordinal) ||
    record.ordinal < 1 ||
    (record.role !== "user" && record.role !== "assistant") ||
    !boundedString(record.text, MAX_MESSAGE_BYTES, true) ||
    (record.state !== "complete" &&
      record.state !== "partial" &&
      record.state !== "failed") ||
    !isSafeNonNegativeInteger(record.createdAtMs) ||
    !isSafeNonNegativeInteger(record.updatedAtMs) ||
    record.updatedAtMs < record.createdAtMs
  ) {
    throw new Error("INVALID_STORED_MESSAGE");
  }
  return Object.freeze({
    id: parseId(record.id, "INVALID_STORED_MESSAGE"),
    chatId: parseId(record.chatId, "INVALID_STORED_MESSAGE"),
    ordinal: record.ordinal,
    role: record.role,
    text: record.text,
    state: record.state,
    createdAtMs: record.createdAtMs,
    updatedAtMs: record.updatedAtMs,
  });
}

function parseProviderId(value: unknown): LlmProviderId {
  if (typeof value !== "string" || !PROVIDER_IDS.has(value as LlmProviderId)) {
    throw new Error("INVALID_APP_PREFERENCES");
  }
  return value as LlmProviderId;
}

function parsePreferences(value: unknown): AppPreferences {
  const record = exactRecord(
    value,
    ["selectedProviderId", "modelIds", "theme", "defaultMode"],
    "INVALID_APP_PREFERENCES",
  );
  if (
    !isRecord(record.modelIds) ||
    (record.theme !== "system" &&
      record.theme !== "light" &&
      record.theme !== "dark") ||
    (record.defaultMode !== "chat" && record.defaultMode !== "story")
  ) {
    throw new Error("INVALID_APP_PREFERENCES");
  }
  const modelIds: Partial<Record<ApiKeyProviderId, string>> = {};
  for (const [key, modelId] of Object.entries(record.modelIds)) {
    if (
      !API_KEY_PROVIDER_IDS.has(key as ApiKeyProviderId) ||
      !boundedString(modelId, 256, true)
    ) {
      throw new Error("INVALID_APP_PREFERENCES");
    }
    modelIds[key as ApiKeyProviderId] = modelId;
  }
  return Object.freeze({
    selectedProviderId: parseProviderId(record.selectedProviderId),
    modelIds: Object.freeze(modelIds),
    theme: record.theme,
    defaultMode: record.defaultMode,
  });
}

function parseVersionedPreferences(value: unknown): VersionedAppPreferences {
  const record = exactRecord(
    value,
    ["revision", "value"],
    "INVALID_APP_PREFERENCES",
  );
  if (!isSafeNonNegativeInteger(record.revision)) {
    throw new Error("INVALID_APP_PREFERENCES");
  }
  return Object.freeze({
    revision: record.revision,
    value: parsePreferences(record.value),
  });
}

export function createStorageClient(
  invokeCommand: StorageInvoker = (command, args) =>
    invoke<unknown>(command, args),
) {
  return Object.freeze({
    async getStorageStatus(): Promise<StorageStatus> {
      const record = exactRecord(
        await invokeCommand("get_storage_status"),
        ["available", "schemaVersion", "errorCode"],
        "INVALID_STORAGE_STATUS",
      );
      if (
        typeof record.available !== "boolean" ||
        !(
          record.schemaVersion === null ||
          isSafeNonNegativeInteger(record.schemaVersion)
        ) ||
        !(
          record.errorCode === null ||
          boundedString(record.errorCode, 64)
        ) ||
        record.available !== (record.errorCode === null)
      ) {
        throw new Error("INVALID_STORAGE_STATUS");
      }
      return Object.freeze({
        available: record.available,
        schemaVersion: record.schemaVersion,
        errorCode: record.errorCode,
      });
    },
    async createChat(characterId: string, title: string): Promise<StoredChat> {
      return parseChat(
        await invokeCommand("create_chat", { characterId, title }),
      );
    },
    async listChats(
      limit = MAX_CHAT_PAGE,
      before: ChatCursor | null = null,
    ): Promise<ChatPage> {
      if (
        !Number.isSafeInteger(limit) ||
        limit < 1 ||
        limit > MAX_CHAT_PAGE
      ) {
        throw new Error("INVALID_CHAT_PAGE");
      }
      const validatedBefore =
        before === null ? null : parseChatCursor(before);
      const record = exactRecord(
        await invokeCommand("list_chats", { limit, before: validatedBefore }),
        ["items", "nextCursor"],
        "INVALID_CHAT_PAGE",
      );
      if (
        !Array.isArray(record.items) ||
        record.items.length > limit ||
        !(record.nextCursor === null || isRecord(record.nextCursor))
      ) {
        throw new Error("INVALID_CHAT_PAGE");
      }
      const items = record.items.map(parseChat);
      const nextCursor =
        record.nextCursor === null ? null : parseChatCursor(record.nextCursor);
      const ids = new Set<string>();
      let previous = validatedBefore;
      for (const item of items) {
        const cursor = { updatedAtMs: item.updatedAtMs, chatId: item.id };
        if (
          ids.has(item.id) ||
          (previous !== null && compareChatCursor(cursor, previous) >= 0)
        ) {
          throw new Error("INVALID_CHAT_PAGE");
        }
        ids.add(item.id);
        previous = cursor;
      }
      if (
        (nextCursor !== null &&
          (items.length === 0 ||
            items.length !== limit ||
            nextCursor.updatedAtMs !== items.at(-1)?.updatedAtMs ||
            nextCursor.chatId !== items.at(-1)?.id)) ||
        (items.length < limit && nextCursor !== null)
      ) {
        throw new Error("INVALID_CHAT_PAGE");
      }
      return Object.freeze({
        items: Object.freeze(items),
        nextCursor,
      });
    },
    async loadChatMessages(
      chatId: string,
      limit = MAX_MESSAGE_PAGE,
    ): Promise<MessagePage> {
      parseId(chatId, "INVALID_MESSAGE_PAGE");
      if (
        !Number.isSafeInteger(limit) ||
        limit < 1 ||
        limit > MAX_MESSAGE_PAGE
      ) {
        throw new Error("INVALID_MESSAGE_PAGE");
      }
      const allItems: StoredMessage[] = [];
      const seenIds = new Set<string>();
      let afterOrdinal: number | null = null;
      let textBytes = 0;
      for (let pageIndex = 0; pageIndex < MAX_MESSAGE_HISTORY_PAGES; pageIndex += 1) {
        const record = exactRecord(
          await invokeCommand("load_chat_messages", {
            chatId,
            limit,
            afterOrdinal,
          }),
          ["items", "nextOrdinal"],
          "INVALID_MESSAGE_PAGE",
        );
        if (
          !Array.isArray(record.items) ||
          record.items.length > limit ||
          !(
            record.nextOrdinal === null ||
            isSafeNonNegativeInteger(record.nextOrdinal)
          )
        ) {
          throw new Error("INVALID_MESSAGE_PAGE");
        }
        const pageItems = record.items.map(parseMessage);
        let previousOrdinal = afterOrdinal ?? 0;
        for (const message of pageItems) {
          if (
            message.chatId !== chatId ||
            seenIds.has(message.id) ||
            message.ordinal <= previousOrdinal
          ) {
            throw new Error("INVALID_MESSAGE_PAGE");
          }
          previousOrdinal = message.ordinal;
          seenIds.add(message.id);
          textBytes += utf8Length(message.text);
          if (
            allItems.length + 1 > MAX_MESSAGE_HISTORY_ITEMS ||
            textBytes > MAX_MESSAGE_HISTORY_TEXT_BYTES
          ) {
            throw new Error("MESSAGE_HISTORY_LIMIT_EXCEEDED");
          }
          allItems.push(message);
        }
        const nextOrdinal = record.nextOrdinal;
        if (
          nextOrdinal !== null &&
          (pageItems.length === 0 ||
            pageItems.length !== limit ||
            nextOrdinal !== pageItems.at(-1)?.ordinal ||
            nextOrdinal <= (afterOrdinal ?? 0))
        ) {
          throw new Error("INVALID_MESSAGE_PAGE");
        }
        if (nextOrdinal === null) {
          return Object.freeze({
            items: Object.freeze(allItems),
            hasMore: false,
          });
        }
        afterOrdinal = nextOrdinal;
      }
      throw new Error("MESSAGE_HISTORY_LIMIT_EXCEEDED");
    },
    async deleteChat(chatId: string): Promise<DeleteChatReceipt> {
      const record = exactRecord(
        await invokeCommand("delete_chat", { chatId }),
        ["chatId", "deleted"],
        "INVALID_DELETE_CHAT_RECEIPT",
      );
      if (typeof record.deleted !== "boolean") {
        throw new Error("INVALID_DELETE_CHAT_RECEIPT");
      }
      return Object.freeze({
        chatId: parseId(record.chatId, "INVALID_DELETE_CHAT_RECEIPT"),
        deleted: record.deleted,
      });
    },
    async getAppPreferences(): Promise<VersionedAppPreferences> {
      return parseVersionedPreferences(
        await invokeCommand("get_app_preferences"),
      );
    },
    async updateAppPreferences(
      expectedRevision: number,
      value: AppPreferences,
    ): Promise<VersionedAppPreferences> {
      return parseVersionedPreferences(
        await invokeCommand("update_app_preferences", {
          expectedRevision,
          value,
        }),
      );
    },
  });
}

export const storageClient = createStorageClient();
