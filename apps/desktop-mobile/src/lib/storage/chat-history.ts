import type { ChatMessage } from "$lib/chat/types";

import {
  storageClient,
  type ChatCursor,
  type MessageCursor,
  type StoredChat,
  type StoredMessage,
} from "./client";
import { createBoundedHistoryWindow } from "./bounded-history-window";

export const FIRST_CHAT_CHARACTER_ID = "seraphine";
const FIRST_CHAT_TITLE = "세라핀과의 대화";
const CHAT_SCAN_PAGE_SIZE = 100;
const MAX_CHAT_SCAN_PAGES = 100;

type ChatStorageClient = Pick<
  typeof storageClient,
  "createChat" | "listChats" | "loadChatMessages"
>;

export type LoadedChatHistory = Readonly<{
  chat: StoredChat;
  messages: readonly ChatMessage[];
  hasMore: boolean;
  olderCursor: MessageCursor | null;
}>;

function restoredAssistantText(message: StoredMessage): string {
  if (message.text.length > 0) return message.text;
  if (message.state === "complete") {
    return "제공자가 빈 응답을 반환했습니다.";
  }
  if (message.state === "partial") {
    return "표시할 수 있는 응답 내용이 없습니다.";
  }
  return "응답을 완료하지 못했습니다.";
}

export function toChatMessage(message: StoredMessage): ChatMessage {
  return {
    id: message.id,
    role: message.role === "assistant" ? "character" : "user",
    text:
      message.role === "assistant"
        ? restoredAssistantText(message)
        : message.text,
    sentAt: new Date(message.createdAtMs),
    deliveryState: message.state,
  };
}

function cursorIsStrictlyOlder(
  cursor: ChatCursor,
  previous: ChatCursor,
): boolean {
  return (
    cursor.updatedAtMs < previous.updatedAtMs ||
    (cursor.updatedAtMs === previous.updatedAtMs &&
      cursor.chatId < previous.chatId)
  );
}

export async function loadOrCreateCharacterChat(
  characterId: string,
  title: string,
  client: ChatStorageClient = storageClient,
): Promise<LoadedChatHistory> {
  const seenChatIds = new Set<string>();
  let before: ChatCursor | null = null;
  let chat: StoredChat | undefined;
  let exhausted = false;

  for (let pageIndex = 0; pageIndex < MAX_CHAT_SCAN_PAGES; pageIndex += 1) {
    const listed = await client.listChats(CHAT_SCAN_PAGE_SIZE, before);
    for (const candidate of listed.items) {
      if (seenChatIds.has(candidate.id)) {
        throw new Error("INVALID_CHAT_PAGINATION");
      }
      seenChatIds.add(candidate.id);
      if (candidate.characterId === characterId) {
        chat = candidate;
        break;
      }
    }
    if (chat !== undefined) break;

    const next = listed.nextCursor;
    if (next === null) {
      exhausted = true;
      break;
    }
    if (
      listed.items.length === 0 ||
      next.updatedAtMs !== listed.items.at(-1)?.updatedAtMs ||
      next.chatId !== listed.items.at(-1)?.id ||
      (before !== null && !cursorIsStrictlyOlder(next, before))
    ) {
      throw new Error("INVALID_CHAT_PAGINATION");
    }
    before = next;
  }

  if (chat === undefined) {
    if (!exhausted) {
      throw new Error("CHAT_SCAN_LIMIT_EXCEEDED");
    }
    chat = await client.createChat(characterId, title);
  }
  const page = await client.loadChatMessages(chat.id);
  const window = createBoundedHistoryWindow(page);
  return Object.freeze({
    chat,
    messages: Object.freeze(window.items.map(toChatMessage)),
    hasMore: window.hasMore,
    olderCursor: window.olderCursor,
  });
}

export function loadOrCreateFirstChat(
  client: ChatStorageClient = storageClient,
): Promise<LoadedChatHistory> {
  return loadOrCreateCharacterChat(
    FIRST_CHAT_CHARACTER_ID,
    FIRST_CHAT_TITLE,
    client,
  );
}
