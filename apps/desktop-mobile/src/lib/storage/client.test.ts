import { describe, expect, it, vi } from "vitest";

import { createStorageClient } from "./client";

const id = "a".repeat(32);
const walMaintenanceStatus = Object.freeze({
  schedulerStarted: true,
  running: false,
  intervalMs: 60_000,
  restartThresholdBytes: 64 * 1024 * 1024,
  emergencyTruncateThresholdBytes: 512 * 1024 * 1024,
  successfulRuns: 1,
  failedRuns: 0,
  consecutiveStarvationRuns: 0,
  activeReaders: 0,
  oldestReaderAgeMs: null,
  lastAttemptStartedAtMs: 10,
  lastAttemptCompletedAtMs: 12,
  lastAttemptDurationMs: 2,
  lastSuccessAtMs: 12,
  lastErrorAtMs: null,
  lastErrorCode: null,
  passiveBusy: false,
  passiveRemainingFrames: 0,
  passiveWalFileBytes: 0,
  restartBusy: null,
  restartRemainingFrames: null,
  restartWalFileBytes: null,
  truncateBusy: null,
  truncateRemainingFrames: null,
  truncateWalFileBytes: null,
  thresholdExceeded: false,
  emergencyTruncateThresholdExceeded: false,
  starvationObserved: false,
});

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
          hasMore: false,
          olderCursor: null,
        };
      }
      throw new Error(`unexpected ${command}`);
    });
    const client = createStorageClient(invokeCommand);

    await expect(
      client.createChat("seraphine", "세라핀과의 대화"),
    ).resolves.toMatchObject({ id, characterId: "seraphine" });
    await expect(client.loadChatMessages(id)).resolves.toMatchObject({
      items: [{ chatId: id, text: "안녕" }],
    });
    expect(invokeCommand).toHaveBeenCalledWith("load_chat_messages", {
      chatId: id,
      limit: 200,
      before: null,
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
      hasMore: false,
      olderCursor: null,
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

  it("loads only one newest canonical page and exposes an older cursor", async () => {
    const latest = Array.from({ length: 200 }, (_, index) => ({
      id: (index + 801).toString(16).padStart(32, "0"),
      chatId: id,
      ordinal: index + 801,
      role: index % 2 === 0 ? "user" : "assistant",
      text: `message-${index + 801}`,
      state: "complete",
      createdAtMs: index + 801,
      updatedAtMs: index + 801,
    }));
    const invokeCommand = vi.fn(async () => ({
      items: latest,
      hasMore: true,
      olderCursor: { chatId: id, ordinal: 801 },
    }));
    const client = createStorageClient(invokeCommand);

    const result = await client.loadChatMessages(id);

    expect(result.items).toHaveLength(200);
    expect(result.items.at(0)).toMatchObject({ ordinal: 801 });
    expect(result.items.at(-1)).toMatchObject({ ordinal: 1_000 });
    expect(result.olderCursor).toEqual({ chatId: id, ordinal: 801 });
    expect(invokeCommand).toHaveBeenCalledTimes(1);
    expect(invokeCommand).toHaveBeenCalledWith("load_chat_messages", {
      chatId: id,
      limit: 200,
      before: null,
    });
  });

  it("accepts a byte-bounded short page with a valid older cursor", async () => {
    const only = {
      id: "e".repeat(32),
      chatId: id,
      ordinal: 900,
      role: "assistant",
      text: "large-but-native-bounded",
      state: "complete",
      createdAtMs: 900,
      updatedAtMs: 900,
    };
    const client = createStorageClient(async () => ({
      items: [only],
      hasMore: true,
      olderCursor: { chatId: id, ordinal: 900 },
    }));

    await expect(client.loadChatMessages(id, 200)).resolves.toMatchObject({
      items: [{ ordinal: 900 }],
      hasMore: true,
      olderCursor: { chatId: id, ordinal: 900 },
    });
  });

  it("rejects a forged or non-decreasing older cursor", async () => {
    const page = Array.from({ length: 2 }, (_, index) => ({
      id: (index + 8).toString(16).padStart(32, "0"),
      chatId: id,
      ordinal: index + 8,
      role: "user",
      text: "message",
      state: "complete",
      createdAtMs: index + 8,
      updatedAtMs: index + 8,
    }));
    const client = createStorageClient(async () => ({
      items: page,
      hasMore: true,
      olderCursor: { chatId: id, ordinal: 10 },
    }));

    await expect(
      client.loadChatMessages(id, 2, { chatId: id, ordinal: 10 }),
    ).rejects.toThrow("INVALID_MESSAGE_PAGE");
  });

  it("requests one older page with a closed exclusive cursor", async () => {
    const invokeCommand = vi.fn(async () => ({
      items: [8, 9].map((ordinal) => ({
        id: ordinal.toString(16).padStart(32, "0"),
        chatId: id,
        ordinal,
        role: "assistant",
        text: `message-${ordinal}`,
        state: "complete",
        createdAtMs: ordinal,
        updatedAtMs: ordinal,
      })),
      hasMore: true,
      olderCursor: { chatId: id, ordinal: 8 },
    }));
    const client = createStorageClient(invokeCommand);

    await expect(
      client.loadChatMessages(id, 2, { chatId: id, ordinal: 10 }),
    ).resolves.toMatchObject({
      items: [{ ordinal: 8 }, { ordinal: 9 }],
      hasMore: true,
      olderCursor: { chatId: id, ordinal: 8 },
    });
    expect(invokeCommand).toHaveBeenCalledTimes(1);
    expect(invokeCommand).toHaveBeenCalledWith("load_chat_messages", {
      chatId: id,
      limit: 2,
      before: { chatId: id, ordinal: 10 },
    });

    await expect(
      client.loadChatMessages(id, 2, {
        chatId: id,
        ordinal: 10,
        injected: true,
      } as never),
    ).rejects.toThrow("INVALID_MESSAGE_PAGE");
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
        return {
          available: true,
          schemaVersion: 1,
          errorCode: null,
          walMaintenance: walMaintenanceStatus,
        };
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
      walMaintenance: walMaintenanceStatus,
    });
    await expect(client.deleteChat(id)).resolves.toEqual({
      chatId: id,
      deleted: true,
    });
  });

  it("rejects unbounded or internally inconsistent WAL telemetry", async () => {
    const unsafe = createStorageClient(async () => ({
      available: true,
      schemaVersion: 1,
      errorCode: null,
      walMaintenance: {
        ...walMaintenanceStatus,
        passiveWalFileBytes: Number.MAX_SAFE_INTEGER + 1,
      },
    }));
    await expect(unsafe.getStorageStatus()).rejects.toThrow(
      "INVALID_WAL_MAINTENANCE_STATUS",
    );

    const inconsistent = createStorageClient(async () => ({
      available: true,
      schemaVersion: 1,
      errorCode: null,
      walMaintenance: {
        ...walMaintenanceStatus,
        restartBusy: true,
      },
    }));
    await expect(inconsistent.getStorageStatus()).rejects.toThrow(
      "INVALID_WAL_MAINTENANCE_STATUS",
    );

    const impossibleReaderAge = createStorageClient(async () => ({
      available: true,
      schemaVersion: 1,
      errorCode: null,
      walMaintenance: {
        ...walMaintenanceStatus,
        oldestReaderAgeMs: 5,
      },
    }));
    await expect(impossibleReaderAge.getStorageStatus()).rejects.toThrow(
      "INVALID_WAL_MAINTENANCE_STATUS",
    );

    const unsafeEmergencyPolicy = createStorageClient(async () => ({
      available: true,
      schemaVersion: 1,
      errorCode: null,
      walMaintenance: {
        ...walMaintenanceStatus,
        emergencyTruncateThresholdBytes: 1,
      },
    }));
    await expect(unsafeEmergencyPolicy.getStorageStatus()).rejects.toThrow(
      "INVALID_WAL_MAINTENANCE_STATUS",
    );

    const truncateWithoutHealthyRestart = createStorageClient(async () => ({
      available: true,
      schemaVersion: 1,
      errorCode: null,
      walMaintenance: {
        ...walMaintenanceStatus,
        truncateBusy: false,
        truncateRemainingFrames: 0,
        truncateWalFileBytes: 0,
      },
    }));
    await expect(
      truncateWithoutHealthyRestart.getStorageStatus(),
    ).rejects.toThrow("INVALID_WAL_MAINTENANCE_STATUS");
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
