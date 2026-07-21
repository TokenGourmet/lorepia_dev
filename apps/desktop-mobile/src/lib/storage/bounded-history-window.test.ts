import { describe, expect, it } from "vitest";

import type { MessagePage, StoredMessage } from "./client";
import {
  appendHistoryItems,
  boundedTailById,
  createBoundedHistoryWindow,
  prependHistoryPage,
} from "./bounded-history-window";

const chatId = "a".repeat(32);

function message(ordinal: number): StoredMessage {
  return {
    id: ordinal.toString(16).padStart(32, "0"),
    chatId,
    ordinal,
    role: ordinal % 2 === 0 ? "assistant" : "user",
    text: `message-${ordinal}`,
    state: "complete",
    createdAtMs: ordinal,
    updatedAtMs: ordinal,
  };
}

function page(
  first: number,
  last: number,
  olderOrdinal: number | null,
): MessagePage {
  return {
    items: Array.from({ length: last - first + 1 }, (_, index) =>
      message(first + index),
    ),
    hasMore: olderOrdinal !== null,
    olderCursor:
      olderOrdinal === null ? null : { chatId, ordinal: olderOrdinal },
  };
}

describe("bounded history window", () => {
  it("prepends an older page with a strictly decreasing cursor and a hard cap", () => {
    const current = createBoundedHistoryWindow(page(6, 10, 6), 8);
    const prepended = prependHistoryPage(
      current,
      { chatId, ordinal: 6 },
      page(3, 5, 3),
    );

    expect(prepended.items.map((item) => item.ordinal)).toEqual([
      3, 4, 5, 6, 7, 8, 9, 10,
    ]);
    expect(prepended.olderCursor).toEqual({ chatId, ordinal: 3 });
    expect(prepended.items).toHaveLength(8);
  });

  it("rejects cursor replay and refuses to discard a newer tail at the cap", () => {
    const current = createBoundedHistoryWindow(page(6, 10, 6), 10);
    const overlapping: MessagePage = {
      items: [message(5)],
      hasMore: false,
      olderCursor: null,
    };
    const prepended = prependHistoryPage(
      current,
      { chatId, ordinal: 6 },
      overlapping,
    );
    expect(prepended.items.map((item) => item.ordinal)).toEqual([
      5, 6, 7, 8, 9, 10,
    ]);
    expect(() =>
      prependHistoryPage(current, { chatId, ordinal: 7 }, overlapping),
    ).toThrow("INVALID_HISTORY_CURSOR");
    expect(() =>
      prependHistoryPage(
        createBoundedHistoryWindow(page(6, 10, 6), 8),
        { chatId, ordinal: 6 },
        page(1, 5, 1),
      ),
    ).toThrow("HISTORY_WINDOW_CAP_REACHED");
  });

  it("keeps only the newest items after append and exposes the evicted boundary", () => {
    const current = createBoundedHistoryWindow(page(1, 4, null), 5);
    const appended = appendHistoryItems(current, [
      message(4),
      message(5),
      message(6),
    ]);
    expect(appended.items.map((item) => item.ordinal)).toEqual([2, 3, 4, 5, 6]);
    expect(appended.olderCursor).toEqual({ chatId, ordinal: 2 });
    expect(appended.hasMore).toBe(true);
    expect(() =>
      appendHistoryItems(current, [{ ...message(4), text: "tampered" }]),
    ).toThrow("INVALID_HISTORY_WINDOW");
  });

  it("bounds transient render state and keeps the newest duplicate identity", () => {
    const items = [
      { id: "one", text: "old" },
      { id: "two", text: "two" },
      { id: "one", text: "new" },
      { id: "three", text: "three" },
    ];
    expect(boundedTailById(items, 2)).toEqual([
      { id: "one", text: "new" },
      { id: "three", text: "three" },
    ]);
  });
});
