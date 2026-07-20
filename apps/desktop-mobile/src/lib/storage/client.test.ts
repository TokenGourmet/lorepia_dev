import { describe, expect, it, vi } from "vitest";

import { createStorageClient } from "./client";

const id = "a".repeat(32);

describe("product storage client", () => {
  it("uses closed chat commands and validates their response identity", async () => {
    const invokeCommand = vi.fn(async (command: string) => {
      if (command === "create_chat") {
        return {
          id,
          characterId: "seraphine",
          title: "세라핀과의 대화",
          revision: 1,
          createdAtMs: 10,
          updatedAtMs: 10,
        };
      }
      if (command === "load_chat_messages") {
        return {
          items: [
            {
              id: "b".repeat(32),
              chatId: id,
              ordinal: 1,
              role: "user",
              text: "안녕",
              state: "complete",
              createdAtMs: 11,
              updatedAtMs: 11,
            },
          ],
          nextOrdinal: null,
        };
      }
      throw new Error(`unexpected ${command}`);
    });
    const client = createStorageClient(invokeCommand);

    await expect(client.createChat("seraphine", "세라핀과의 대화")).resolves
      .toMatchObject({ id, characterId: "seraphine" });
    await expect(client.loadChatMessages(id)).resolves.toMatchObject({
      items: [{ chatId: id, text: "안녕" }],
    });
    expect(invokeCommand).toHaveBeenCalledWith("load_chat_messages", {
      chatId: id,
      limit: 200,
      afterOrdinal: null,
    });
  });

  it("rejects unknown fields and cross-chat messages", async () => {
    const client = createStorageClient(async () => ({
      items: [
        {
          id: "b".repeat(32),
          chatId: "c".repeat(32),
          ordinal: 1,
          role: "assistant",
          text: "no",
          state: "complete",
          createdAtMs: 1,
          updatedAtMs: 1,
        },
      ],
      nextOrdinal: null,
    }));
    await expect(client.loadChatMessages(id)).rejects.toThrow(
      "INVALID_MESSAGE_PAGE",
    );

    const extra = createStorageClient(async () => ({
      items: [],
      nextCursor: null,
      sql: "do not expose",
    }));
    await expect(extra.listChats()).rejects.toThrow("INVALID_CHAT_PAGE");
  });

  it("loads every canonical message page with a strictly advancing cursor", async () => {
    const first = Array.from({ length: 200 }, (_, index) => ({
      id: index.toString(16).padStart(32, "0"),
      chatId: id,
      ordinal: index + 1,
      role: index % 2 === 0 ? "user" : "assistant",
      text: `message-${index + 1}`,
      state: "complete",
      createdAtMs: index + 1,
      updatedAtMs: index + 1,
    }));
    const invokeCommand = vi.fn(
      async (_command: string, args?: Record<string, unknown>) => {
      if (args?.afterOrdinal === null) {
        return { items: first, nextOrdinal: 200 };
      }
      return {
        items: [
          {
            id: "f".repeat(32),
            chatId: id,
            ordinal: 201,
            role: "assistant",
            text: "latest",
            state: "complete",
            createdAtMs: 201,
            updatedAtMs: 201,
          },
        ],
        nextOrdinal: null,
      };
      },
    );
    const client = createStorageClient(invokeCommand);

    const result = await client.loadChatMessages(id);

    expect(result.items).toHaveLength(201);
    expect(result.items.at(-1)).toMatchObject({ ordinal: 201, text: "latest" });
    expect(invokeCommand).toHaveBeenNthCalledWith(2, "load_chat_messages", {
      chatId: id,
      limit: 200,
      afterOrdinal: 200,
    });
  });

  it("rejects non-advancing message cursors instead of looping", async () => {
    const page = Array.from({ length: 2 }, (_, index) => ({
      id: (index + 1).toString(16).padStart(32, "0"),
      chatId: id,
      ordinal: index + 1,
      role: "user",
      text: "message",
      state: "complete",
      createdAtMs: index + 1,
      updatedAtMs: index + 1,
    }));
    const client = createStorageClient(async () => ({
      items: page,
      nextOrdinal: 1,
    }));

    await expect(client.loadChatMessages(id, 2)).rejects.toThrow(
      "INVALID_MESSAGE_PAGE",
    );
  });

  it("rejects a forged chat cursor that does not match the page tail", async () => {
    const client = createStorageClient(async () => ({
      items: [
        {
          id,
          characterId: "other",
          title: "other",
          revision: 1,
          createdAtMs: 1,
          updatedAtMs: 2,
        },
      ],
      nextCursor: { updatedAtMs: 3, chatId: "b".repeat(32) },
    }));

    await expect(client.listChats(1)).rejects.toThrow("INVALID_CHAT_PAGE");
  });

  it("validates storage availability and delete receipts", async () => {
    const invokeCommand = vi.fn(async (command: string) => {
      if (command === "get_storage_status") {
        return { available: true, schemaVersion: 1, errorCode: null };
      }
      if (command === "delete_chat") {
        return { chatId: id, deleted: true };
      }
      throw new Error(`unexpected ${command}`);
    });
    const client = createStorageClient(invokeCommand);

    await expect(client.getStorageStatus()).resolves.toEqual({
      available: true,
      schemaVersion: 1,
      errorCode: null,
    });
    await expect(client.deleteChat(id)).resolves.toEqual({
      chatId: id,
      deleted: true,
    });
  });

  it("round-trips only typed non-secret preferences", async () => {
    const response = {
      revision: 4,
      value: {
        selectedProviderId: "anthropic",
        modelIds: { anthropic: "claude-example" },
        theme: "dark",
        defaultMode: "story",
      },
    };
    const invokeCommand = vi.fn(async () => response);
    const client = createStorageClient(invokeCommand);

    await expect(client.getAppPreferences()).resolves.toEqual(response);
    expect(JSON.stringify(response)).not.toMatch(
      /api.?key|credential|control.?token/i,
    );
  });

  it("rejects unknown providers and secret-shaped preference fields", async () => {
    const invalidProvider = createStorageClient(async () => ({
      revision: 1,
      value: {
        selectedProviderId: "mystery",
        modelIds: {},
        theme: "system",
        defaultMode: "chat",
      },
    }));
    await expect(invalidProvider.getAppPreferences()).rejects.toThrow(
      "INVALID_APP_PREFERENCES",
    );

    const secretField = createStorageClient(async () => ({
      revision: 1,
      value: {
        selectedProviderId: "openai",
        modelIds: {},
        theme: "system",
        defaultMode: "chat",
        apiKey: "secret",
      },
    }));
    await expect(secretField.getAppPreferences()).rejects.toThrow(
      "INVALID_APP_PREFERENCES",
    );
  });
});
