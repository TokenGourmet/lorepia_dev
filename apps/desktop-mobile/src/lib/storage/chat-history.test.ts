import { describe, expect, it, vi } from "vitest";

import type { MessagePage } from "./client";
import {
  loadOrCreateCharacterChat,
  loadOrCreateFirstChat,
  toChatMessage,
} from "./chat-history";

const chat = {
  id: "a".repeat(32),
  characterId: "seraphine",
  title: "세라핀과의 대화",
  revision: 1,
  createdAtMs: 10,
  updatedAtMs: 10,
} as const;

describe("persistent first chat history", () => {
  it("reuses the persisted character chat and restores canonical messages", async () => {
    const createChat = vi.fn();
    const result = await loadOrCreateFirstChat({
      createChat,
      listChats: vi.fn(async () => ({ items: [chat], nextCursor: null })),
      loadChatMessages: vi.fn(async (): Promise<MessagePage> => ({
        items: [
          {
            id: "b".repeat(32),
            chatId: chat.id,
            ordinal: 1,
            role: "user",
            text: "다시 왔어",
            state: "complete",
            createdAtMs: 20,
            updatedAtMs: 20,
          },
        ],
        hasMore: false,
        olderCursor: null,
      })),
    });

    expect(createChat).not.toHaveBeenCalled();
    expect(result.chat).toEqual(chat);
    expect(result.messages).toMatchObject([
      { id: "b".repeat(32), role: "user", text: "다시 왔어" },
    ]);
  });

  it("creates the fixed product chat only when none exists", async () => {
    const createChat = vi.fn(async () => chat);
    await loadOrCreateFirstChat({
      createChat,
      listChats: vi.fn(async () => ({ items: [], nextCursor: null })),
      loadChatMessages: vi.fn(async () => ({
        items: [],
        hasMore: false,
        olderCursor: null,
      })),
    });

    expect(createChat).toHaveBeenCalledWith("seraphine", "세라핀과의 대화");
  });

  it("finds and creates chats for the selected character", async () => {
    const kaiChat = {
      ...chat,
      id: "d".repeat(32),
      characterId: "kai",
      title: "카이와의 대화",
    };
    const createChat = vi.fn(async () => kaiChat);

    await loadOrCreateCharacterChat("kai", "카이와의 대화", {
      createChat,
      listChats: vi.fn(async () => ({ items: [chat], nextCursor: null })),
      loadChatMessages: vi.fn(async () => ({
        items: [],
        hasMore: false,
        olderCursor: null,
      })),
    });

    expect(createChat).toHaveBeenCalledWith("kai", "카이와의 대화");
  });

  it("scans beyond the first 100 chats before deciding to create", async () => {
    const olderChats = Array.from({ length: 100 }, (_, index) => ({
      ...chat,
      id: (index + 1).toString(16).padStart(32, "0"),
      characterId: `other-${index}`,
      updatedAtMs: 1_000 - index,
    }));
    const cursor = {
      updatedAtMs: olderChats.at(-1)!.updatedAtMs,
      chatId: olderChats.at(-1)!.id,
    };
    const createChat = vi.fn();
    const listChats = vi.fn(
      async (_limit: number, before: typeof cursor | null) =>
        before === null
          ? { items: olderChats, nextCursor: cursor }
          : { items: [chat], nextCursor: null },
    );

    const result = await loadOrCreateFirstChat({
      createChat,
      listChats,
      loadChatMessages: vi.fn(async () => ({
        items: [],
        hasMore: false,
        olderCursor: null,
      })),
    });

    expect(result.chat).toEqual(chat);
    expect(createChat).not.toHaveBeenCalled();
    expect(listChats).toHaveBeenNthCalledWith(2, 100, cursor);
  });

  it("rejects repeated or forged chat cursors without creating a duplicate", async () => {
    const first = {
      ...chat,
      id: "b".repeat(32),
      characterId: "other-1",
      updatedAtMs: 20,
    };
    const second = {
      ...chat,
      id: "c".repeat(32),
      characterId: "other-2",
      updatedAtMs: 30,
    };
    const cursor = { updatedAtMs: first.updatedAtMs, chatId: first.id };
    const createChat = vi.fn();
    const listChats = vi
      .fn()
      .mockResolvedValueOnce({ items: [first], nextCursor: cursor })
      .mockResolvedValueOnce({
        items: [second],
        nextCursor: { updatedAtMs: second.updatedAtMs, chatId: second.id },
      });

    await expect(
      loadOrCreateFirstChat({
        createChat,
        listChats,
        loadChatMessages: vi.fn(),
      }),
    ).rejects.toThrow("INVALID_CHAT_PAGINATION");
    expect(createChat).not.toHaveBeenCalled();
  });

  it("renders recovered empty assistant states without marking them streaming", () => {
    expect(
      toChatMessage({
        id: "b".repeat(32),
        chatId: chat.id,
        ordinal: 2,
        role: "assistant",
        text: "",
        state: "partial",
        createdAtMs: 20,
        updatedAtMs: 20,
      }),
    ).toMatchObject({
      role: "character",
      text: "표시할 수 있는 응답 내용이 없습니다.",
      deliveryState: "partial",
    });
  });

  it("preserves partial and failed state when durable text is non-empty", () => {
    expect(
      toChatMessage({
        id: "c".repeat(32),
        chatId: chat.id,
        ordinal: 2,
        role: "assistant",
        text: "복구된 일부",
        state: "failed",
        createdAtMs: 20,
        updatedAtMs: 21,
      }),
    ).toMatchObject({
      text: "복구된 일부",
      deliveryState: "failed",
    });
  });
});
