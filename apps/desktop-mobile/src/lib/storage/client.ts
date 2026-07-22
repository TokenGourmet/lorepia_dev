import { invoke } from "@tauri-apps/api/core";

import type { ThreadMode } from "$lib/chat/types";
import type { ApiKeyProviderId, LlmProviderId } from "$lib/providers/catalog";
import type { ThemePreference } from "$lib/design/theme.svelte";

const MAX_CHAT_PAGE = 100;
export const MAX_MESSAGE_PAGE = 200;
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
  olderCursor: MessageCursor | null;
}>;

export type MessageCursor = Readonly<{
  chatId: string;
  ordinal: number;
}>;

export type ChatCursor = Readonly<{
  updatedAtMs: number;
  chatId: string;
}>;

export type StorageStatus = Readonly<{
  available: boolean;
  schemaVersion: number | null;
  errorCode: string | null;
  walMaintenance: WalMaintenanceStatus;
}>;

export type WalMaintenanceStatus = Readonly<{
  schedulerStarted: boolean;
  running: boolean;
  intervalMs: number;
  restartThresholdBytes: number;
  emergencyTruncateThresholdBytes: number;
  successfulRuns: number;
  failedRuns: number;
  consecutiveStarvationRuns: number;
  activeReaders: number;
  oldestReaderAgeMs: number | null;
  lastAttemptStartedAtMs: number | null;
  lastAttemptCompletedAtMs: number | null;
  lastAttemptDurationMs: number | null;
  lastSuccessAtMs: number | null;
  lastErrorAtMs: number | null;
  lastErrorCode: string | null;
  passiveBusy: boolean | null;
  passiveRemainingFrames: number | null;
  passiveWalFileBytes: number | null;
  restartBusy: boolean | null;
  restartRemainingFrames: number | null;
  restartWalFileBytes: number | null;
  truncateBusy: boolean | null;
  truncateRemainingFrames: number | null;
  truncateWalFileBytes: number | null;
  thresholdExceeded: boolean | null;
  emergencyTruncateThresholdExceeded: boolean | null;
  starvationObserved: boolean | null;
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
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
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
    ["id", "characterId", "title", "revision", "createdAtMs", "updatedAtMs"],
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

function parseMessageCursor(value: unknown): MessageCursor {
  const record = exactRecord(
    value,
    ["chatId", "ordinal"],
    "INVALID_MESSAGE_PAGE",
  );
  if (!isSafeNonNegativeInteger(record.ordinal) || record.ordinal < 1) {
    throw new Error("INVALID_MESSAGE_PAGE");
  }
  return Object.freeze({
    chatId: parseId(record.chatId, "INVALID_MESSAGE_PAGE"),
    ordinal: record.ordinal,
  });
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

function isNullableSafeNonNegativeInteger(
  value: unknown,
): value is number | null {
  return value === null || isSafeNonNegativeInteger(value);
}

function isNullableBoolean(value: unknown): value is boolean | null {
  return value === null || typeof value === "boolean";
}

function parseWalMaintenanceStatus(value: unknown): WalMaintenanceStatus {
  const record = exactRecord(
    value,
    [
      "schedulerStarted",
      "running",
      "intervalMs",
      "restartThresholdBytes",
      "emergencyTruncateThresholdBytes",
      "successfulRuns",
      "failedRuns",
      "consecutiveStarvationRuns",
      "activeReaders",
      "oldestReaderAgeMs",
      "lastAttemptStartedAtMs",
      "lastAttemptCompletedAtMs",
      "lastAttemptDurationMs",
      "lastSuccessAtMs",
      "lastErrorAtMs",
      "lastErrorCode",
      "passiveBusy",
      "passiveRemainingFrames",
      "passiveWalFileBytes",
      "restartBusy",
      "restartRemainingFrames",
      "restartWalFileBytes",
      "truncateBusy",
      "truncateRemainingFrames",
      "truncateWalFileBytes",
      "thresholdExceeded",
      "emergencyTruncateThresholdExceeded",
      "starvationObserved",
    ],
    "INVALID_WAL_MAINTENANCE_STATUS",
  );
  const passiveSamplePresent = record.passiveBusy !== null;
  const restartSamplePresent = record.restartBusy !== null;
  const truncateSamplePresent = record.truncateBusy !== null;
  if (
    typeof record.schedulerStarted !== "boolean" ||
    typeof record.running !== "boolean" ||
    (record.running && !record.schedulerStarted) ||
    !isSafeNonNegativeInteger(record.intervalMs) ||
    record.intervalMs < 1 ||
    !isSafeNonNegativeInteger(record.restartThresholdBytes) ||
    record.restartThresholdBytes < 1 ||
    !isSafeNonNegativeInteger(record.emergencyTruncateThresholdBytes) ||
    record.emergencyTruncateThresholdBytes < record.restartThresholdBytes ||
    !isSafeNonNegativeInteger(record.successfulRuns) ||
    !isSafeNonNegativeInteger(record.failedRuns) ||
    !isSafeNonNegativeInteger(record.consecutiveStarvationRuns) ||
    record.consecutiveStarvationRuns > record.successfulRuns ||
    !isSafeNonNegativeInteger(record.activeReaders) ||
    !isNullableSafeNonNegativeInteger(record.oldestReaderAgeMs) ||
    (record.activeReaders === 0) !== (record.oldestReaderAgeMs === null) ||
    !isNullableSafeNonNegativeInteger(record.lastAttemptStartedAtMs) ||
    !isNullableSafeNonNegativeInteger(record.lastAttemptCompletedAtMs) ||
    !isNullableSafeNonNegativeInteger(record.lastAttemptDurationMs) ||
    !isNullableSafeNonNegativeInteger(record.lastSuccessAtMs) ||
    !isNullableSafeNonNegativeInteger(record.lastErrorAtMs) ||
    !(
      record.lastErrorCode === null || boundedString(record.lastErrorCode, 64)
    ) ||
    !isNullableBoolean(record.passiveBusy) ||
    !isNullableSafeNonNegativeInteger(record.passiveRemainingFrames) ||
    !isNullableSafeNonNegativeInteger(record.passiveWalFileBytes) ||
    !isNullableBoolean(record.restartBusy) ||
    !isNullableSafeNonNegativeInteger(record.restartRemainingFrames) ||
    !isNullableSafeNonNegativeInteger(record.restartWalFileBytes) ||
    !isNullableBoolean(record.truncateBusy) ||
    !isNullableSafeNonNegativeInteger(record.truncateRemainingFrames) ||
    !isNullableSafeNonNegativeInteger(record.truncateWalFileBytes) ||
    !isNullableBoolean(record.thresholdExceeded) ||
    !isNullableBoolean(record.emergencyTruncateThresholdExceeded) ||
    !isNullableBoolean(record.starvationObserved) ||
    passiveSamplePresent !== (record.passiveRemainingFrames !== null) ||
    passiveSamplePresent !== (record.passiveWalFileBytes !== null) ||
    passiveSamplePresent !== (record.thresholdExceeded !== null) ||
    passiveSamplePresent !==
      (record.emergencyTruncateThresholdExceeded !== null) ||
    passiveSamplePresent !== (record.starvationObserved !== null) ||
    restartSamplePresent !== (record.restartRemainingFrames !== null) ||
    restartSamplePresent !== (record.restartWalFileBytes !== null) ||
    truncateSamplePresent !== (record.truncateRemainingFrames !== null) ||
    truncateSamplePresent !== (record.truncateWalFileBytes !== null) ||
    (passiveSamplePresent &&
      restartSamplePresent !== (record.thresholdExceeded === true)) ||
    (record.emergencyTruncateThresholdExceeded === true &&
      !restartSamplePresent) ||
    (truncateSamplePresent &&
      (record.emergencyTruncateThresholdExceeded !== true ||
        record.restartBusy !== false ||
        record.restartRemainingFrames !== 0))
  ) {
    throw new Error("INVALID_WAL_MAINTENANCE_STATUS");
  }
  return Object.freeze({
    schedulerStarted: record.schedulerStarted,
    running: record.running,
    intervalMs: record.intervalMs,
    restartThresholdBytes: record.restartThresholdBytes,
    emergencyTruncateThresholdBytes: record.emergencyTruncateThresholdBytes,
    successfulRuns: record.successfulRuns,
    failedRuns: record.failedRuns,
    consecutiveStarvationRuns: record.consecutiveStarvationRuns,
    activeReaders: record.activeReaders,
    oldestReaderAgeMs: record.oldestReaderAgeMs,
    lastAttemptStartedAtMs: record.lastAttemptStartedAtMs,
    lastAttemptCompletedAtMs: record.lastAttemptCompletedAtMs,
    lastAttemptDurationMs: record.lastAttemptDurationMs,
    lastSuccessAtMs: record.lastSuccessAtMs,
    lastErrorAtMs: record.lastErrorAtMs,
    lastErrorCode: record.lastErrorCode,
    passiveBusy: record.passiveBusy,
    passiveRemainingFrames: record.passiveRemainingFrames,
    passiveWalFileBytes: record.passiveWalFileBytes,
    restartBusy: record.restartBusy,
    restartRemainingFrames: record.restartRemainingFrames,
    restartWalFileBytes: record.restartWalFileBytes,
    truncateBusy: record.truncateBusy,
    truncateRemainingFrames: record.truncateRemainingFrames,
    truncateWalFileBytes: record.truncateWalFileBytes,
    thresholdExceeded: record.thresholdExceeded,
    emergencyTruncateThresholdExceeded:
      record.emergencyTruncateThresholdExceeded,
    starvationObserved: record.starvationObserved,
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
        ["available", "schemaVersion", "errorCode", "walMaintenance"],
        "INVALID_STORAGE_STATUS",
      );
      if (
        typeof record.available !== "boolean" ||
        !(
          record.schemaVersion === null ||
          isSafeNonNegativeInteger(record.schemaVersion)
        ) ||
        !(record.errorCode === null || boundedString(record.errorCode, 64)) ||
        record.available !== (record.errorCode === null)
      ) {
        throw new Error("INVALID_STORAGE_STATUS");
      }
      return Object.freeze({
        available: record.available,
        schemaVersion: record.schemaVersion,
        errorCode: record.errorCode,
        walMaintenance: parseWalMaintenanceStatus(record.walMaintenance),
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
      if (!Number.isSafeInteger(limit) || limit < 1 || limit > MAX_CHAT_PAGE) {
        throw new Error("INVALID_CHAT_PAGE");
      }
      const validatedBefore = before === null ? null : parseChatCursor(before);
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
      before: MessageCursor | null = null,
    ): Promise<MessagePage> {
      parseId(chatId, "INVALID_MESSAGE_PAGE");
      if (
        !Number.isSafeInteger(limit) ||
        limit < 1 ||
        limit > MAX_MESSAGE_PAGE
      ) {
        throw new Error("INVALID_MESSAGE_PAGE");
      }
      const validatedBefore =
        before === null ? null : parseMessageCursor(before);
      if (validatedBefore !== null && validatedBefore.chatId !== chatId) {
        throw new Error("INVALID_MESSAGE_PAGE");
      }
      const record = exactRecord(
        await invokeCommand("load_chat_messages", {
          chatId,
          limit,
          before: validatedBefore,
        }),
        ["items", "hasMore", "olderCursor"],
        "INVALID_MESSAGE_PAGE",
      );
      if (
        !Array.isArray(record.items) ||
        record.items.length > limit ||
        typeof record.hasMore !== "boolean" ||
        !(record.olderCursor === null || isRecord(record.olderCursor))
      ) {
        throw new Error("INVALID_MESSAGE_PAGE");
      }
      const items = record.items.map(parseMessage);
      const seenIds = new Set<string>();
      let previousOrdinal = 0;
      for (const message of items) {
        if (
          message.chatId !== chatId ||
          seenIds.has(message.id) ||
          message.ordinal <= previousOrdinal ||
          (validatedBefore !== null &&
            message.ordinal >= validatedBefore.ordinal)
        ) {
          throw new Error("INVALID_MESSAGE_PAGE");
        }
        previousOrdinal = message.ordinal;
        seenIds.add(message.id);
      }
      const olderCursor =
        record.olderCursor === null
          ? null
          : parseMessageCursor(record.olderCursor);
      if (
        record.hasMore !== (olderCursor !== null) ||
        (olderCursor !== null &&
          (items.length === 0 ||
            olderCursor.chatId !== chatId ||
            olderCursor.ordinal !== items[0]?.ordinal ||
            (validatedBefore !== null &&
              olderCursor.ordinal >= validatedBefore.ordinal)))
      ) {
        throw new Error("INVALID_MESSAGE_PAGE");
      }
      return Object.freeze({
        items: Object.freeze(items),
        hasMore: record.hasMore,
        olderCursor,
      });
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
